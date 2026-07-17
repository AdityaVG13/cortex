use crate::constants::*;
use crate::daemon::paths::daemon_port;
use serde::Serialize;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct RequestTimeouts {
    pub connect: std::time::Duration,
    pub read: std::time::Duration,
    pub write: std::time::Duration,
}

#[derive(Serialize)]
pub struct FetchCortexResponse {
    pub status: u16,
    pub body: String,
}
pub fn send_cortex_request(method: &str, path: &str, auth_token: &str, body: Option<&str>, timeout_ms: Option<u64>) -> Result<FetchCortexResponse, String> {
    let read_timeout = Duration::from_millis(timeout_ms.unwrap_or(DAEMON_READ_TIMEOUT_MS).clamp(DAEMON_MIN_REQUEST_TIMEOUT_MS, DAEMON_MAX_REQUEST_TIMEOUT_MS));
    send_cortex_request_with_port(
        daemon_port(),
        method,
        path,
        auth_token,
        body,
        RequestTimeouts {
            connect: Duration::from_millis(DAEMON_CONNECT_TIMEOUT_MS),
            read: read_timeout,
            write: Duration::from_millis(DAEMON_WRITE_TIMEOUT_MS),
        },
    )
}
pub fn should_use_partial_response_on_read_timeout(err: &std::io::Error, response_len: usize) -> bool {
    if response_len == 0 {
        return false;
    }

    if matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
        return true;
    }

    // Windows socket timeouts are sometimes reported as WSAETIMEDOUT (10060)
    // with a non-timeout ErrorKind; treat them as timeout-equivalent.
    err.raw_os_error() == Some(10060)
}

pub fn validate_cortex_request_path(path: &str) -> Result<(), String> {
    if path.contains('\r') || path.contains('\n') {
        return Err("Invalid request path".to_string());
    }
    if !path.starts_with('/') {
        return Err("Request path must be origin-form".to_string());
    }
    if path.contains("://") || path.contains(' ') {
        return Err("Invalid request path".to_string());
    }
    Ok(())
}

pub fn send_cortex_request_with_port(
    port: u16,
    method: &str,
    path: &str,
    auth_token: &str,
    body: Option<&str>,
    timeouts: RequestTimeouts,
) -> Result<FetchCortexResponse, String> {
    use std::io::{Read, Write};

    validate_cortex_request_path(path)?;

    let mut stream =
        TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeouts.connect).map_err(|e| format!("Cannot connect to daemon: {e}"))?;
    stream.set_read_timeout(Some(timeouts.read)).map_err(|e| format!("Cannot set read timeout: {e}"))?;
    stream.set_write_timeout(Some(timeouts.write)).map_err(|e| format!("Cannot set write timeout: {e}"))?;

    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Cortex-Request: true\r\n");
    if !auth_token.is_empty() {
        request.push_str(&format!("Authorization: Bearer {auth_token}\r\n"));
    }
    if let Some(payload) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    }
    request.push_str("Connection: close\r\n\r\n");
    if let Some(payload) = body {
        request.push_str(payload);
    }

    stream.write_all(request.as_bytes()).map_err(|e| format!("Write failed: {e}"))?;

    let mut response = Vec::new();
    if let Err(err) = stream.read_to_end(&mut response) {
        if !should_use_partial_response_on_read_timeout(&err, response.len()) {
            return Err(format!("Read failed: {err}"));
        }
    }

    // Split headers from body
    if let Some(pos) = find_bytes(&response, b"\r\n\r\n") {
        let headers = &response[..pos];
        let body = &response[pos + 4..];
        let headers_text = String::from_utf8_lossy(headers);
        let status = parse_status_code(&headers_text)?;
        let chunked = headers_text.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("transfer-encoding:") && lower.contains("chunked")
        });

        // Check for chunked transfer encoding
        let body_bytes = if chunked { decode_chunked_bytes(body)? } else { body.to_vec() };
        let body_text = String::from_utf8(body_bytes).map_err(|e| format!("Response body is not valid UTF-8: {e}"))?;
        Ok(FetchCortexResponse { status, body: body_text })
    } else {
        Err("Invalid HTTP response".to_string())
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn parse_status_code(headers: &str) -> Result<u16, String> {
    let status_line = headers.lines().next().ok_or_else(|| "Missing HTTP status line".to_string())?;
    let code = status_line.split_whitespace().nth(1).ok_or_else(|| format!("Invalid HTTP status line: {status_line}"))?;
    code.parse::<u16>().map_err(|e| format!("Invalid HTTP status code '{code}': {e}"))
}

fn decode_chunked_bytes(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut remaining = body;

    loop {
        let line_end = find_bytes(remaining, b"\r\n").ok_or_else(|| "Invalid chunked encoding: missing chunk size line ending".to_string())?;
        let size_line = std::str::from_utf8(&remaining[..line_end]).map_err(|e| format!("Invalid chunk size line UTF-8: {e}"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|e| format!("Invalid chunk size '{size_hex}': {e}"))?;

        let data_start = line_end + 2;
        if data_start > remaining.len() {
            return Err("Invalid chunked encoding: malformed chunk header".to_string());
        }
        remaining = &remaining[data_start..];

        if size == 0 {
            break;
        }

        if remaining.len() < size + 2 {
            return Err("Invalid chunked encoding: chunk truncated".to_string());
        }

        result.extend_from_slice(&remaining[..size]);
        remaining = &remaining[size..];

        if !remaining.starts_with(b"\r\n") {
            return Err("Invalid chunked encoding: missing CRLF after chunk".to_string());
        }
        remaining = &remaining[2..];
    }

    Ok(result)
}
