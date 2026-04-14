use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod support;
use support::{terminate_child_tree, SpawnTrackedExt};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[test]
fn direct_mcp_refuses_auto_spawn_when_daemon_absent() {
    let home_dir = unique_temp_dir("mcp_transport");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
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
        .output()
        .expect("run cortex mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "cli mcp must fail when no daemon is already running"
    );
    assert!(
        stderr.contains("cannot start it automatically")
            || stderr.contains("another process still holds the daemon lock"),
        "expected ownership-policy rejection in stderr, got: {stderr}"
    );
    assert!(!health_ok(port), "cli mcp must not auto-spawn daemon");
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_mcp_refuses_auto_spawn_without_opt_in() {
    let home_dir = unique_temp_dir("plugin_mcp_no_autospawn");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
            "mcp",
            "--agent",
            "claude-code",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run cortex plugin mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "plugin mcp must fail when no daemon is already running and local spawn opt-in is absent"
    );
    assert!(
        stderr.contains("cannot start it automatically")
            || stderr.contains("another process still holds the daemon lock"),
        "expected ownership-policy rejection in stderr, got: {stderr}"
    );
    assert!(!health_ok(port), "plugin mcp must not auto-spawn daemon");
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn direct_mcp_still_refuses_auto_spawn_when_stdin_closes_immediately() {
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
        .spawn_tracked("spawn idle cortex mcp");

    wait_for_exit(&mut child, Duration::from_secs(10));
    assert!(!health_ok(port), "cli mcp must not auto-spawn daemon");
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn direct_mcp_does_not_stop_preexisting_daemon() {
    let home_dir = unique_temp_dir("mcp_existing_daemon");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["serve", "--home", &home, "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex serve");

    wait_for_health(port, &mut daemon);

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
        .spawn_tracked("spawn cortex mcp");

    wait_for_exit(&mut child, Duration::from_secs(10));

    assert!(
        health_ok(port),
        "preexisting daemon should remain healthy after mcp wrapper exits"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_owned_mcp_recovers_after_daemon_interruption_and_cleans_up_on_exit() {
    let home_dir = unique_temp_dir("mcp_recovery");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
            "mcp",
            "--agent",
            "claude-code",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex mcp");

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
    assert!(
        initialize.get("result").is_some(),
        "initialize failed: {initialize}"
    );

    let daemon_pid = read_daemon_pid(&home_dir);
    terminate_pid(daemon_pid);

    let mut saw_unavailable = false;
    let mut recovered = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut id = 10;
    while Instant::now() < deadline {
        write_json_line(
            stdin,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/list"
            }),
        );
        let resp = read_json_line(&responses, RESPONSE_TIMEOUT);
        if resp
            .get("result")
            .and_then(|value| value.get("tools"))
            .and_then(|value| value.as_array())
            .is_some_and(|tools| !tools.is_empty())
        {
            recovered = true;
            break;
        }
        if resp
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|msg| msg.contains("Daemon unavailable"))
        {
            saw_unavailable = true;
        }
        id += 1;
        thread::sleep(Duration::from_millis(250));
    }
    assert!(
        saw_unavailable,
        "expected a transient daemon unavailable response after forced kill"
    );
    assert!(
        recovered,
        "owned mcp flow did not recover after forced kill"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, RESPONSE_TIMEOUT);
    wait_for_daemon_shutdown(
        port,
        Duration::from_secs(8),
        "owned daemon remained healthy after MCP session exit",
    );
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_mcp_local_owner_mode_stops_spawned_daemon_on_exit() {
    let home_dir = unique_temp_dir("plugin_mcp_owner");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
            "mcp",
            "--agent",
            "claude-code",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex plugin mcp");

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
    assert!(
        initialize.get("result").is_some(),
        "initialize failed: {initialize}"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, RESPONSE_TIMEOUT);
    wait_for_daemon_shutdown(
        port,
        Duration::from_secs(8),
        "plugin owner mode should stop daemon it spawned",
    );
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_mcp_local_owner_mode_rejects_non_claude_agent() {
    let home_dir = unique_temp_dir("plugin_mcp_reject_non_claude");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
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
        .output()
        .expect("run cortex plugin mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "non-claude plugin local owner mode should fail"
    );
    assert!(
        stderr.contains("Claude-only mode"),
        "expected rejection reason in stderr, got: {stderr}"
    );
    assert!(
        !health_ok(port),
        "daemon should not start for rejected non-claude local plugin invocation"
    );
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_mcp_custom_url_does_not_shutdown_or_respawn_target_daemon() {
    let home_dir = unique_temp_dir("plugin_mcp_custom");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["serve", "--home", &home, "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex serve");
    wait_for_health(port, &mut daemon);

    let token = fs::read_to_string(home_dir.join("cortex.token"))
        .expect("read daemon token")
        .trim()
        .to_string();
    let base_url = format!("http://127.0.0.1:{port}");

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
            "mcp",
            "--url",
            &base_url,
            "--api-key",
            &token,
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
        .spawn_tracked("spawn cortex plugin mcp custom url");

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
    assert!(
        initialize.get("result").is_some(),
        "initialize failed: {initialize}"
    );

    assert!(
        health_ok(port),
        "target daemon must remain healthy before kill"
    );
    daemon.kill().expect("kill target daemon");
    wait_for_exit(&mut daemon, Duration::from_secs(5));

    write_json_line(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let tools = read_json_line(&responses, RESPONSE_TIMEOUT);
    assert!(
        tools
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|msg| msg.contains("Daemon unavailable")),
        "expected daemon unavailable response after target kill: {tools}"
    );
    thread::sleep(Duration::from_millis(800));
    assert!(
        !health_ok(port),
        "custom-url plugin flow must not respawn killed target daemon"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, RESPONSE_TIMEOUT);
    assert!(
        !health_ok(port),
        "custom-url plugin flow must not restart or shutdown-manage target daemon"
    );
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn plugin_mcp_env_remote_target_disables_local_owner_lifecycle() {
    let home_dir = unique_temp_dir("plugin_mcp_env_remote");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let mut daemon = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["serve", "--home", &home, "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex serve");
    wait_for_health(port, &mut daemon);

    let token = fs::read_to_string(home_dir.join("cortex.token"))
        .expect("read daemon token")
        .trim()
        .to_string();
    let base_url = format!("http://127.0.0.1:{port}");

    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "plugin",
            "mcp",
            "--agent",
            "codex",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_API_BASE", &base_url)
        .env("CORTEX_API_KEY", &token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex plugin mcp via env target");

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
    assert!(
        initialize.get("result").is_some(),
        "initialize failed: {initialize}"
    );

    assert!(
        health_ok(port),
        "target daemon must remain healthy before kill"
    );
    daemon.kill().expect("kill target daemon");
    wait_for_exit(&mut daemon, Duration::from_secs(5));

    write_json_line(
        stdin,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let tools = read_json_line(&responses, RESPONSE_TIMEOUT);
    assert!(
        tools
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|msg| msg.contains("Daemon unavailable")),
        "expected daemon unavailable response after target kill: {tools}"
    );
    thread::sleep(Duration::from_millis(800));
    assert!(
        !health_ok(port),
        "env-target plugin flow must not respawn killed target daemon"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, RESPONSE_TIMEOUT);
    assert!(
        !health_ok(port),
        "env-target plugin flow must not restart or shutdown-manage target daemon"
    );
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_local_mode_refuses_spawn_when_control_center_lock_is_active() {
    let home_dir = unique_temp_dir("mcp_control_center_lock");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let runtime_dir = home_dir.join("runtime");
    fs::create_dir_all(&runtime_dir).expect("create runtime dir");
    let control_center_lock = runtime_dir.join("control-center.lock");
    fs::write(&control_center_lock, "owner=control-center").expect("seed control center lock");
    let mut perms = fs::metadata(&control_center_lock)
        .expect("read control center lock metadata")
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(&control_center_lock, perms).expect("mark control center lock readonly");

    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
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
        .output()
        .expect("run cortex mcp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "mcp should fail when control-center lock is active and daemon is unavailable"
    );
    assert!(
        stderr.contains("cannot start it automatically")
            || stderr.contains("another process still holds the daemon lock"),
        "expected ownership-policy rejection in stderr, got: {stderr}"
    );
    assert!(
        !health_ok(port),
        "daemon should not be auto-spawned while control-center lock is active"
    );

    let _ = fs::remove_dir_all(&home_dir);
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    path: String,
    authorization: Option<String>,
}

#[test]
fn mcp_withholds_local_token_fallback_until_health_identity_is_valid() {
    let home_dir = unique_temp_dir("mcp_wrong_instance_guard");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let local_token = "ctx_local_should_not_leak";
    fs::write(home_dir.join("cortex.token"), local_token).expect("write local token");

    let wrong_identity_health = serde_json::json!({
        "status": "ok",
        "runtime": {
            "version": "0.5.0",
            "port": port,
            "token_path": "C:/wrong-instance/cortex.token",
            "pid_path": "C:/wrong-instance/cortex.pid",
            "db_path": "C:/wrong-instance/cortex.db"
        },
        "stats": {
            "memories": 1,
            "home": "C:/wrong-instance"
        }
    })
    .to_string();

    let stop_listener = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let listener_stop = Arc::clone(&stop_listener);
    let captured_requests = Arc::clone(&captured);
    let listener = thread::spawn(move || {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind adversarial listener");
        listener
            .set_nonblocking(true)
            .expect("set adversarial listener nonblocking");

        while !listener_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(request) = read_captured_request(&mut stream) {
                        let path = request.path.clone();
                        captured_requests
                            .lock()
                            .expect("lock captured requests")
                            .push(request);

                        match path.as_str() {
                            "/health" => write_mock_http_response(
                                &mut stream,
                                200,
                                "application/json",
                                &wrong_identity_health,
                            ),
                            "/mcp-rpc" => write_mock_http_response(
                                &mut stream,
                                401,
                                "text/plain",
                                "Unauthorized",
                            ),
                            "/session/end" => {
                                write_mock_http_response(&mut stream, 200, "application/json", "{}")
                            }
                            _ => write_mock_http_response(
                                &mut stream,
                                404,
                                "text/plain",
                                "Not Found",
                            ),
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("adversarial listener accept failed: {err}"),
            }
        }
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "mcp",
            "--url",
            &base_url,
            "--agent",
            "codex",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_HOME", &home)
        .env("CORTEX_PORT", port.to_string())
        .env("CORTEX_BIND", "127.0.0.1")
        .env_remove("CORTEX_API_KEY")
        .env_remove("CORTEX_API_BASE")
        .env_remove("CORTEX_BASE_URL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex mcp against adversarial listener");

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
    let response = read_json_line(&responses, Duration::from_secs(45));
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(
        response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .is_some_and(|msg| msg.contains("Daemon unavailable")),
        "expected daemon unavailable response from adversarial target: {response}"
    );

    drop(child.stdin.take());
    wait_for_exit(&mut child, Duration::from_secs(15));

    stop_listener.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    listener.join().expect("join adversarial listener");

    let captured_requests = captured.lock().expect("lock captured requests");
    assert!(
        captured_requests
            .iter()
            .any(|request| request.path == "/health"),
        "expected health probes against adversarial listener"
    );
    assert!(
        captured_requests
            .iter()
            .any(|request| request.path == "/mcp-rpc"),
        "expected mcp-rpc calls against adversarial listener"
    );

    let leaked_paths: Vec<&str> = captured_requests
        .iter()
        .filter_map(|request| {
            request
                .authorization
                .as_deref()
                .filter(|value| value.contains(local_token))
                .map(|_| request.path.as_str())
        })
        .collect();
    assert!(
        leaked_paths.is_empty(),
        "local token leaked to wrong-instance listener on paths: {leaked_paths:?}"
    );
    assert!(
        captured_requests
            .iter()
            .filter(|request| request.path == "/mcp-rpc")
            .all(|request| request.authorization.is_none()),
        "mcp-rpc requests should not carry Authorization until health identity validates: {captured_requests:?}"
    );

    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn boot_withholds_local_token_fallback_until_health_identity_is_valid() {
    let home_dir = unique_temp_dir("boot_wrong_instance_guard");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let local_token = "ctx_boot_local_should_not_leak";
    fs::write(home_dir.join("cortex.token"), local_token).expect("write local token");

    let wrong_identity_health = serde_json::json!({
        "status": "ok",
        "runtime": {
            "version": "0.5.0",
            "port": port,
            "token_path": "C:/wrong-instance/cortex.token",
            "pid_path": "C:/wrong-instance/cortex.pid",
            "db_path": "C:/wrong-instance/cortex.db"
        },
        "stats": {
            "memories": 1,
            "home": "C:/wrong-instance"
        }
    })
    .to_string();

    let stop_listener = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(Mutex::new(Vec::<CapturedRequest>::new()));
    let listener_stop = Arc::clone(&stop_listener);
    let captured_requests = Arc::clone(&captured);
    let listener = thread::spawn(move || {
        let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind adversarial listener");
        listener
            .set_nonblocking(true)
            .expect("set adversarial listener nonblocking");

        while !listener_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Some(request) = read_captured_request(&mut stream) {
                        let path = request.path.clone();
                        captured_requests
                            .lock()
                            .expect("lock captured requests")
                            .push(request);

                        match path.as_str() {
                            "/health" => write_mock_http_response(
                                &mut stream,
                                200,
                                "application/json",
                                &wrong_identity_health,
                            ),
                            path if path.starts_with("/boot") => {
                                write_mock_http_response(&mut stream, 200, "application/json", "{}")
                            }
                            _ => write_mock_http_response(
                                &mut stream,
                                404,
                                "text/plain",
                                "Not Found",
                            ),
                        }
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(err) => panic!("adversarial listener accept failed: {err}"),
            }
        }
    });

    let base_url = format!("http://127.0.0.1:{port}");
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "boot",
            "--url",
            &base_url,
            "--agent",
            "codex",
            "--home",
            &home,
            "--port",
            &port.to_string(),
        ])
        .env("CORTEX_HOME", &home)
        .env("CORTEX_PORT", port.to_string())
        .env("CORTEX_BIND", "127.0.0.1")
        .env_remove("CORTEX_API_KEY")
        .env_remove("CORTEX_API_BASE")
        .env_remove("CORTEX_BASE_URL")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .expect("run cortex boot against adversarial listener");

    assert!(
        !output.status.success(),
        "boot should fail against wrong-instance listener"
    );

    stop_listener.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(("127.0.0.1", port));
    listener.join().expect("join adversarial listener");

    let captured_requests = captured.lock().expect("lock captured requests");
    assert!(
        captured_requests
            .iter()
            .any(|request| request.path == "/health"),
        "expected health probes against adversarial listener"
    );
    assert!(
        captured_requests
            .iter()
            .any(|request| request.path.starts_with("/boot")),
        "expected boot calls against adversarial listener"
    );

    let leaked_paths: Vec<&str> = captured_requests
        .iter()
        .filter_map(|request| {
            request
                .authorization
                .as_deref()
                .filter(|value| value.contains(local_token))
                .map(|_| request.path.as_str())
        })
        .collect();
    assert!(
        leaked_paths.is_empty(),
        "local token leaked to wrong-instance listener on paths: {leaked_paths:?}"
    );
    assert!(
        captured_requests
            .iter()
            .filter(|request| request.path.starts_with("/boot"))
            .all(|request| request.authorization.is_none()),
        "boot requests should not carry Authorization until health identity validates: {captured_requests:?}"
    );

    let _ = fs::remove_dir_all(&home_dir);
}

fn read_captured_request(stream: &mut TcpStream) -> Option<CapturedRequest> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");

    let mut raw = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        raw.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_terminator(&raw) {
            break pos;
        }
    };

    let header_text = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();

    let mut content_length = 0usize;
    let mut authorization = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse::<usize>().unwrap_or(0);
            } else if name == "authorization" {
                authorization = Some(value);
            }
        }
    }

    let body_start = header_end + 4;
    while raw.len() < body_start + content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..n]);
    }

    Some(CapturedRequest {
        path,
        authorization,
    })
}

fn find_header_terminator(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_mock_http_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("write mock response");
    stream.flush().expect("flush mock response");
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

    terminate_child_tree(child);
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

    terminate_child_tree(child);
    let stderr = read_stderr(child);
    panic!("cortex mcp did not exit after stdin closed\n{stderr}");
}

fn wait_for_daemon_shutdown(port: u16, timeout: Duration, panic_msg: &str) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !health_ok(port) {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("{panic_msg}");
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

fn read_daemon_pid(home_dir: &std::path::Path) -> u32 {
    fs::read_to_string(home_dir.join("cortex.pid"))
        .expect("read daemon pid")
        .trim()
        .parse::<u32>()
        .expect("parse daemon pid")
}

fn terminate_pid(pid: u32) {
    #[cfg(windows)]
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()
        .expect("run taskkill");

    #[cfg(not(windows))]
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .expect("run kill");

    assert!(status.success(), "failed to terminate daemon pid {pid}");
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
