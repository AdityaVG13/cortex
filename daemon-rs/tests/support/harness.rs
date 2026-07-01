//! Shared integration-test harness: temp dirs, daemon spawn, HTTP helpers.
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{terminate_child_tree, SpawnTrackedExt};

pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
pub const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

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
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "serve",
            "--home",
            home,
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_tracked("spawn_daemon")
}

pub fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(code) = child.try_wait().expect("poll daemon") {
            panic!("daemon exited early with status {code}");
        }
        if health_ok(port) {
            return;
        }
        thread::sleep(HEALTH_POLL_INTERVAL);
    }
    panic!("daemon did not become healthy on port {port}");
}

pub fn health_ok(port: u16) -> bool {
    let mut stream = match TcpStream::connect(format!("127.0.0.1:{port}")) {
        Ok(stream) => stream,
        Err(_) => return false,
    };
    let request = format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

pub fn read_token(home_dir: &PathBuf) -> String {
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

pub fn shutdown_daemon(port: u16, home_dir: &PathBuf) {
    let token = read_token(home_dir);
    let auth = format!("Bearer {token}");
    let _ = post_raw(
        port,
        "/admin/shutdown",
        &[("Authorization", auth.as_str()), ("X-Cortex-Request", "true")],
        "",
    );
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
}

pub fn post_raw(
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Result<String, String> {
    let mut stream =
        TcpStream::connect(format!("127.0.0.1:{port}")).map_err(|err| err.to_string())?;
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream
        .write_all(request.as_bytes())
        .map_err(|err| err.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|err| err.to_string())?;
    Ok(response)
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

static DAEMON_SPAWN_GUARD: OnceLock<Mutex<()>> = OnceLock::new();

pub fn daemon_spawn_test_guard() -> MutexGuard<'static, ()> {
    DAEMON_SPAWN_GUARD
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("daemon spawn test guard")
}
