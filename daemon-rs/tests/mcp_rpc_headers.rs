use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[test]
fn mcp_rpc_missing_auth_returns_jsonrpc_unauthorized() {
    let home_dir = unique_temp_dir("mcp_rpc_missing_auth");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 17,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ci", "version": "1.0.0" }
        }
    });
    let response = post_json(
        port,
        "/mcp-rpc",
        &[("X-Cortex-Request", "true")],
        &request_body.to_string(),
    )
    .expect("request");
    assert_eq!(http_status(&response), 401);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["error"]["code"], -32600);
    assert_eq!(payload["error"]["message"], "Unauthorized");
    assert_eq!(payload["id"], 17);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_missing_x_cortex_request_with_non_local_origin_returns_forbidden_jsonrpc() {
    let home_dir = unique_temp_dir("mcp_rpc_missing_header_origin");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 29,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ci", "version": "1.0.0" }
        }
    });
    let auth = format!("Bearer {token}");
    let response = post_json(
        port,
        "/mcp-rpc",
        &[("Authorization", &auth), ("Origin", "https://evil.example")],
        &request_body.to_string(),
    )
    .expect("request");
    assert_eq!(http_status(&response), 403);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["error"]["code"], -32600);
    assert_eq!(
        payload["error"]["message"],
        "Missing X-Cortex-Request header"
    );
    assert_eq!(payload["id"], 29);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_local_context_without_x_cortex_request_still_initializes() {
    let home_dir = unique_temp_dir("mcp_rpc_local_no_header");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 31,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ci", "version": "1.0.0" }
        }
    });
    let auth = format!("Bearer {token}");
    let response = post_json(
        port,
        "/mcp-rpc",
        &[("Authorization", &auth)],
        &request_body.to_string(),
    )
    .expect("request");
    assert_eq!(http_status(&response), 200);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], 31);
    assert!(
        payload.get("result").is_some(),
        "expected initialize result, got: {payload}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_x_auth_header_alias_authenticates() {
    let home_dir = unique_temp_dir("mcp_rpc_x_auth_header");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "ci", "version": "1.0.0" }
        }
    });
    let x_auth_header = format!("Authorization: Bearer {token}");
    let response = post_json(
        port,
        "/mcp-rpc",
        &[
            ("X-Cortex-Request", "true"),
            ("X-Auth-Header", &x_auth_header),
        ],
        &request_body.to_string(),
    )
    .expect("request");
    assert_eq!(http_status(&response), 200);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["id"], 41);
    assert!(
        payload.get("result").is_some(),
        "expected initialize result, got: {payload}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

fn spawn_daemon(home: &str, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["serve", "--home", home, "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cortex serve")
}

fn read_token(home_dir: &std::path::Path) -> String {
    fs::read_to_string(home_dir.join("cortex.token"))
        .expect("read daemon token")
        .trim()
        .to_string()
}

fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            let stderr = read_stderr(child);
            panic!("daemon exited before health check succeeded: {status}\n{stderr}");
        }
        if health_ok(port) {
            return;
        }
        thread::sleep(HEALTH_POLL_INTERVAL);
    }

    let stderr = read_stderr(child);
    panic!("daemon did not become healthy on port {port}\n{stderr}");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if child.try_wait().expect("poll child exit").is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    let stderr = read_stderr(child);
    panic!("daemon did not exit in time\n{stderr}");
}

fn health_ok(port: u16) -> bool {
    let Ok(body) = http_request(
        port,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    ) else {
        return false;
    };
    let Some(body) = split_http_body(&body) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(body.trim()) else {
        return false;
    };
    matches!(
        json.get("status").and_then(|value| value.as_str()),
        Some("ok" | "degraded")
    )
}

fn shutdown_daemon(port: u16, home_dir: &std::path::Path) {
    let token = read_token(home_dir);
    let request = format!(
        "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nX-Cortex-Request: true\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    let _ = http_request(port, &request).expect("shutdown daemon");
}

fn post_json(
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(body);
    http_request(port, &request)
}

fn http_request(port: u16, request: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut buffer = String::new();
    stream
        .read_to_string(&mut buffer)
        .map_err(|e| e.to_string())?;
    Ok(buffer)
}

fn http_status(response: &str) -> u16 {
    let status_line = response.lines().next().expect("status line");
    let code = status_line
        .split_whitespace()
        .nth(1)
        .expect("status code field");
    code.parse::<u16>().expect("parse status code")
}

fn split_http_body(response: &str) -> Option<&str> {
    response.split_once("\r\n\r\n").map(|(_, body)| body)
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve local port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("cortex_{prefix}_{unique}"))
}

fn read_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(handle) = child.stderr.as_mut() {
        let _ = handle.read_to_string(&mut stderr);
    }
    stderr
}
