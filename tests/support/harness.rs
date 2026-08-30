use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{terminate_child_tree, SpawnTrackedExt};

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct JsonHttpResponse {
    pub status: u16,
    pub body: Value,
}

pub fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("cortex-{prefix}-{nanos}"))
}

pub fn spawn_daemon(home: &str, port: u16) -> Child {
    Command::new(cortex_tests::cortex_bin())
        .args(["serve", "--home", home, "--port", &port.to_string()])
        .env("CORTEX_SINGLE_DAEMON_TEST_BYPASS", "1")
        .env("CORTEX_BIND", "127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn_daemon")
}

pub fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon") {
            let stderr = read_stderr(child);
            panic!("daemon exited before health check succeeded: {status}\n{stderr}");
        }
        if health_ok(port) {
            return;
        }
        thread::sleep(HEALTH_POLL_INTERVAL);
    }
    terminate_child_tree(child);
    let stderr = read_stderr(child);
    panic!("daemon did not become healthy on port {port}\n{stderr}");
}

pub fn health_ok(port: u16) -> bool {
    let Ok(response) = http_request(
        port,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    ) else {
        return false;
    };
    let Some(body) = split_http_body(&response) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(body.trim()) else {
        return false;
    };
    matches!(
        json.get("status").and_then(Value::as_str),
        Some("ok" | "degraded")
    )
}

pub fn read_token(home_dir: &Path) -> String {
    let token_path = home_dir.join("cortex.token");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if token_path.exists() {
            let token = fs::read_to_string(&token_path).unwrap_or_default();
            if !token.trim().is_empty() {
                return token.trim().to_string();
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("token file not written at {}", token_path.display());
}

pub fn shutdown_daemon(port: u16, home_dir: &Path) {
    let token = read_token(home_dir);
    let request = format!(
        "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nX-Cortex-Request: true\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    let _ = http_request(port, &request);
}

pub fn shutdown_daemon_best_effort(port: u16, home_dir: &Path) {
    if let Ok(token) = fs::read_to_string(home_dir.join("cortex.token")) {
        let token = token.trim();
        if !token.is_empty() {
            let request = format!(
                "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nX-Cortex-Request: true\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            );
            let _ = http_request(port, &request);
        }
    }
}

pub fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait().expect("poll daemon").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    terminate_child_tree(child);
    let stderr = read_stderr(child);
    panic!("daemon did not exit in time\n{stderr}");
}

pub fn http_request(port: u16, request: &str) -> Result<String, String> {
    let mut stream =
        TcpStream::connect(format!("127.0.0.1:{port}")).map_err(|err| err.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|err| err.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|err| err.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|err| err.to_string())?;
    stream.flush().map_err(|err| err.to_string())?;
    let mut buffer = String::new();
    stream
        .read_to_string(&mut buffer)
        .map_err(|err| err.to_string())?;
    Ok(buffer)
}

pub fn post_raw(
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    http_request(port, &request)
}

pub fn post_json(
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let mut merged: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
    merged.extend_from_slice(headers);
    post_raw(port, path, &merged, body)
}

pub fn request_json(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<JsonHttpResponse, String> {
    let body_text = body.map(|value| value.to_string());
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    if let Some(token) = token {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        request.push_str("X-Cortex-Request: true\r\n");
        request.push_str("X-Source-Agent: adapter-conformance\r\n");
    }
    if let Some(body) = &body_text {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("Connection: close\r\n\r\n");
    if let Some(body) = &body_text {
        request.push_str(body);
    }
    let response = http_request(port, &request)?;
    parse_json_response(method, path, &response)
}

pub fn request_json_with_headers(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Result<JsonHttpResponse, String> {
    let body_text = body.map(|value| value.to_string());
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = &body_text {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    request.push_str("Connection: close\r\n\r\n");
    if let Some(body) = &body_text {
        request.push_str(body);
    }
    let response = http_request(port, &request)?;
    parse_json_response(method, path, &response)
}

fn parse_json_response(
    method: &str,
    path: &str,
    response: &str,
) -> Result<JsonHttpResponse, String> {
    let status = http_status(response);
    let body = split_http_body(response).ok_or_else(|| "missing HTTP body".to_string())?;
    let body = serde_json::from_str(body.trim()).map_err(|err| {
        format!("failed to parse JSON response for {method} {path}: {err}; body={body}")
    })?;
    Ok(JsonHttpResponse { status, body })
}

pub fn http_status(response: &str) -> u16 {
    response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

pub fn split_http_body(response: &str) -> Option<&str> {
    response.split("\r\n\r\n").nth(1)
}

pub fn normalize_path_for_compare(raw: &str) -> String {
    raw.replace('\\', "/").trim_end_matches('/').to_string()
}

pub fn assert_path_scoped_to_home(label: &str, reported: &str, expected_home: &str) {
    let normalized = normalize_path_for_compare(reported);
    let home_with_sep = format!("{expected_home}/");
    assert!(
        normalized == expected_home || normalized.starts_with(&home_with_sep),
        "{label} escaped requested home (reported={reported}, expected_home={expected_home})"
    );
}

fn read_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(handle) = child.stderr.as_mut() {
        let _ = handle.read_to_string(&mut stderr);
    }
    stderr
}

fn test_guard(name: &'static str) -> MutexGuard<'static, ()> {
    static REGISTRY: OnceLock<Vec<(&'static str, Mutex<()>)>> = OnceLock::new();
    let registry = REGISTRY.get_or_init(|| {
        vec![
            ("daemon_spawn", Mutex::new(())),
            ("transport", Mutex::new(())),
            ("adapter_conformance", Mutex::new(())),
        ]
    });
    for (label, mutex) in registry {
        if *label == name {
            return mutex
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
    panic!("unknown test guard: {name}");
}

pub fn daemon_spawn_test_guard() -> MutexGuard<'static, ()> {
    test_guard("daemon_spawn")
}

pub fn singleton_transport_test_guard() -> MutexGuard<'static, ()> {
    test_guard("transport")
}

pub fn adapter_conformance_guard() -> MutexGuard<'static, ()> {
    test_guard("adapter_conformance")
}
