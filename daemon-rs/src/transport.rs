// SPDX-License-Identifier: MIT
use crate::auth::CortexPaths;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

fn normalized_host(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

pub fn local_http_base_url(paths: &CortexPaths) -> String {
    let bind = paths.bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{}", paths.port)
}

pub fn is_local_http_base_url(base_url: &str, paths: &CortexPaths) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if url.port_or_known_default() != Some(paths.port) {
        return false;
    }
    let host_norm = normalized_host(host);
    let bind_norm = normalized_host(&paths.bind);
    matches!(host_norm.as_str(), "127.0.0.1" | "localhost" | "::1")
        || (!bind_norm.is_empty()
            && !matches!(bind_norm.as_str(), "0.0.0.0" | "::")
            && host_norm == bind_norm)
}

pub fn local_ipc_endpoint_for_base_url(base_url: &str, paths: &CortexPaths) -> Option<String> {
    if !is_local_http_base_url(base_url, paths) {
        return None;
    }
    paths.ipc_endpoint.clone()
}

fn split_base_and_path(url: &str) -> Option<(String, String)> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let mut base = parsed.clone();
    base.set_path("");
    base.set_query(None);
    base.set_fragment(None);
    let mut path = parsed.path().to_string();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = parsed.query() {
        path.push('?');
        path.push_str(query);
    }
    Some((base.to_string().trim_end_matches('/').to_string(), path))
}

fn parse_http_response(raw: &[u8]) -> Result<(reqwest::StatusCode, String), String> {
    let Some(header_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("invalid HTTP response from IPC endpoint".to_string());
    };

    let header = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| "IPC response headers are not valid UTF-8".to_string())?;
    let status_code = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "IPC response missing valid status line".to_string())?;
    let status = reqwest::StatusCode::from_u16(status_code)
        .map_err(|_| format!("IPC response returned invalid status code {status_code}"))?;
    let body = String::from_utf8_lossy(&raw[header_end + 4..]).to_string();
    Ok((status, body))
}

async fn send_http_over_stream<S>(
    stream: &mut S,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<(reqwest::StatusCode, String), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let body = body.unwrap_or("");
    let mut request = String::new();
    request.push_str(method);
    request.push(' ');
    request.push_str(path);
    request.push_str(" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("Content-Length: ");
    request.push_str(&body.len().to_string());
    request.push_str("\r\n\r\n");
    request.push_str(body);

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("IPC write failed: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("IPC flush failed: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| format!("IPC read failed: {e}"))?;
    parse_http_response(&response)
}

async fn ipc_http_request(
    endpoint: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout: std::time::Duration,
) -> Result<(reqwest::StatusCode, String), String> {
    let fut = async {
        #[cfg(unix)]
        {
            let mut stream = tokio::net::UnixStream::connect(endpoint)
                .await
                .map_err(|e| format!("IPC connect failed: {e}"))?;
            return send_http_over_stream(&mut stream, method, path, headers, body).await;
        }
        #[cfg(windows)]
        {
            let mut stream = tokio::net::windows::named_pipe::ClientOptions::new()
                .open(endpoint)
                .map_err(|e| format!("IPC connect failed: {e}"))?;
            return send_http_over_stream(&mut stream, method, path, headers, body).await;
        }
        #[allow(unreachable_code)]
        Err("IPC transport is unsupported on this platform".to_string())
    };
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| "IPC request timed out".to_string())?
}

async fn send_http_request(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout: std::time::Duration,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        other => return Err(format!("Unsupported request method '{other}'")),
    };
    req = req.timeout(timeout);
    for (name, value) in headers {
        req = req.header(name, value);
    }
    if let Some(payload) = body {
        req = req.body(payload.to_string());
    }

    let response = req.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    Ok((status, body))
}

#[allow(clippy::too_many_arguments)]
pub async fn request_with_local_ipc_fallback(
    client: &reqwest::Client,
    method: &str,
    base_url: &str,
    path: &str,
    paths: &CortexPaths,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout: std::time::Duration,
) -> Result<(reqwest::StatusCode, String), String> {
    if let Some(endpoint) = local_ipc_endpoint_for_base_url(base_url, paths) {
        match ipc_http_request(&endpoint, method, path, headers, body, timeout).await {
            Ok(response) => return Ok(response),
            Err(err) => {
                eprintln!(
                    "[cortex-transport] IPC request failed for {method} {path} ({endpoint}): {err}; falling back to HTTP"
                );
            }
        }
    }

    let normalized_base = base_url.trim_end_matches('/');
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let url = format!("{normalized_base}{normalized_path}");
    send_http_request(client, method, &url, headers, body, timeout).await
}

pub async fn request_url_with_local_ipc_fallback(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    paths: &CortexPaths,
    headers: &[(String, String)],
    body: Option<&str>,
    timeout: std::time::Duration,
) -> Result<(reqwest::StatusCode, String), String> {
    if let Some((base_url, path)) = split_base_and_path(url) {
        return request_with_local_ipc_fallback(
            client, method, &base_url, &path, paths, headers, body, timeout,
        )
        .await;
    }
    send_http_request(client, method, url, headers, body, timeout).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths(bind: &str, port: u16, ipc_endpoint: Option<&str>) -> CortexPaths {
        let temp = std::env::temp_dir().join("cortex_transport_tests");
        CortexPaths {
            home: temp.clone(),
            db: temp.join("cortex.db"),
            token: temp.join("cortex.token"),
            pid: temp.join("cortex.pid"),
            lock: temp.join("cortex.lock"),
            port,
            bind: bind.to_string(),
            ipc_endpoint: ipc_endpoint.map(|value| value.to_string()),
            models: temp.join("models"),
            write_buffer: temp.join("write_buffer.jsonl"),
        }
    }

    #[test]
    fn local_ipc_endpoint_only_resolves_for_local_targets() {
        let paths = test_paths("127.0.0.1", 7437, Some(r"\\.\pipe\cortex-daemon-7437"));
        assert_eq!(
            local_ipc_endpoint_for_base_url("http://127.0.0.1:7437", &paths),
            Some(r"\\.\pipe\cortex-daemon-7437".to_string())
        );
        assert_eq!(
            local_ipc_endpoint_for_base_url("https://api.example.com:443", &paths),
            None
        );
    }

    #[test]
    fn local_http_base_url_uses_loopback_for_wildcard_bind() {
        let paths = test_paths("0.0.0.0", 7437, None);
        assert_eq!(local_http_base_url(&paths), "http://127.0.0.1:7437");
    }
}
