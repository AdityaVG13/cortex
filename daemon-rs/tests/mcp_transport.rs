use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[test]
fn direct_mcp_can_start_daemon_and_keep_it_running() {
    let home_dir = unique_temp_dir("mcp_transport");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "mcp",
            "--agent",
            "codex",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cortex mcp");

    wait_for_health(port, &mut child);

    let stdout = child.stdout.take().expect("child stdout");
    let responses = spawn_stdout_reader(stdout);
    let stdin = child.stdin.as_mut().expect("child stdin");

    write_json_line(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "ci", "version": "1.0.0" }
            }
        }),
    );
    let initialize = read_json_line(&responses, RESPONSE_TIMEOUT);
    assert_eq!(initialize["jsonrpc"], "2.0");
    assert_eq!(initialize["id"], 1);
    assert!(
        initialize.get("result").is_some(),
        "missing initialize result: {initialize}"
    );

    write_json_line(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let tools = read_json_line(&responses, RESPONSE_TIMEOUT);
    assert_eq!(tools["jsonrpc"], "2.0");
    assert_eq!(tools["id"], 2);
    assert!(
        tools["result"]["tools"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "expected tools/list to return tools: {tools}"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, RESPONSE_TIMEOUT);
    assert!(
        health_ok(port),
        "daemon should stay healthy after MCP client exit"
    );

    shutdown_daemon(port, &home_dir);
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn direct_mcp_cleans_up_unused_owned_daemon_when_stdin_closes_immediately() {
    let home_dir = unique_temp_dir("mcp_idle");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "mcp",
            "--agent",
            "codex",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn idle cortex mcp");

    wait_for_exit(&mut child, Duration::from_secs(10));

    let shutdown_deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < shutdown_deadline {
        if !health_ok(port) {
            let _ = fs::remove_dir_all(&home_dir);
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    panic!("owned daemon remained healthy after idle MCP wrapper exit");
}

fn write_json_line(stdin: &mut std::process::ChildStdin, value: Value) {
    let mut line = serde_json::to_vec(&value).expect("serialize request");
    line.push(b'\n');
    stdin.write_all(&line).expect("write request");
    stdin.flush().expect("flush request");
}

fn read_json_line(responses: &Receiver<Result<String, String>>, timeout: Duration) -> Value {
    let line = responses
        .recv_timeout(timeout)
        .expect("timed out waiting for MCP response")
        .expect("read MCP response");
    serde_json::from_str(line.trim()).expect("parse MCP response")
}

fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                    break;
                }
            }
        }
    });
    rx
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

fn wait_for_health(port: u16, child: &mut Child) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            let stderr = read_stderr(child);
            panic!("cortex mcp exited before daemon health check succeeded: {status}\n{stderr}");
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
    panic!("cortex mcp did not exit after stdin closed\n{stderr}");
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
    let token = fs::read_to_string(home_dir.join("cortex.token"))
        .expect("read daemon token")
        .trim()
        .to_string();
    let request = format!(
        "POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nX-Cortex-Request: true\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    );
    let _ = http_request(port, &request).expect("shutdown daemon");
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

fn split_http_body(response: &str) -> Option<&str> {
    response.split_once("\r\n\r\n").map(|(_, body)| body)
}

fn read_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(handle) = child.stderr.as_mut() {
        let _ = handle.read_to_string(&mut stderr);
    }
    stderr
}
