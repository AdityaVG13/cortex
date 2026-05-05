use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod support;
use support::{terminate_child_tree, SpawnTrackedExt};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SPEC: &str = include_str!("../../specs/cortex-adapter-contract.yaml");

#[test]
fn adapter_contract_spec_covers_required_matrix() {
    let spec: Value = serde_json::from_str(SPEC).expect("contract spec is JSON-compatible YAML");
    assert_eq!(spec["schema"], "cortex.adapter.contract");
    assert_eq!(spec["version"], "0.6.0");

    let scenarios = spec["scenarios"].as_array().expect("scenarios array");
    assert!(
        scenarios.len() >= 10,
        "adapter contract must keep at least 10 scenarios"
    );

    let ids: BTreeSet<&str> = scenarios
        .iter()
        .filter_map(|scenario| scenario["id"].as_str())
        .collect();
    for required in [
        "health-public",
        "store-decision",
        "recall-get",
        "recall-post",
        "peek",
        "boot",
        "export-json",
        "mcp-initialize",
        "mcp-tools-list",
        "mcp-health-tool",
        "mcp-store-tool",
        "mcp-recall-tool",
    ] {
        assert!(
            ids.contains(required),
            "missing contract scenario {required}"
        );
    }
}

#[test]
fn http_and_mcp_rpc_match_adapter_contract() {
    let _guard = adapter_conformance_guard();
    let home_dir = unique_temp_dir("adapter_conformance");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let health = request_json(port, "GET", "/health", None, None).expect("health");
    assert_eq!(health.status, 200);
    assert_json_fields(&health.body, &["status", "runtime", "stats"]);

    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Adapter conformance sentinel memory",
            "context": "C4 contract round trip",
            "type": "decision",
            "source_agent": "adapter-conformance-sdk",
            "source_model": "gpt-5.4",
            "confidence": 0.93,
            "reasoning_depth": "high",
            "ttl_seconds": 3600
        })),
    )
    .expect("store");
    assert_eq!(store.status, 200);
    assert_eq!(store.body["stored"], true);
    assert!(
        store.body.get("entry").is_some(),
        "store entry missing: {}",
        store.body
    );

    let recall_get = request_json(
        port,
        "GET",
        "/recall?q=Adapter%20conformance%20sentinel%20memory&budget=200&k=5&agent=adapter-conformance-sdk",
        Some(&token),
        None,
    )
    .expect("recall get");
    assert_eq!(recall_get.status, 200);
    assert_json_fields(
        &recall_get.body,
        &["results", "budget", "spent", "saved", "tokenUsageLine"],
    );
    assert!(
        recall_get.body["results"].as_array().is_some(),
        "recall results should be an array: {}",
        recall_get.body
    );

    let recall_post = request_json(
        port,
        "POST",
        "/recall",
        Some(&token),
        Some(json!({
            "q": "Adapter conformance sentinel memory",
            "budget": 200,
            "k": 5,
            "agent": "adapter-conformance-sdk"
        })),
    )
    .expect("recall post");
    assert_eq!(recall_post.status, 200);
    assert_json_fields(
        &recall_post.body,
        &["results", "budget", "spent", "saved", "tokenUsageLine"],
    );

    let peek = request_json(
        port,
        "GET",
        "/peek?q=Adapter%20conformance%20sentinel%20memory&k=5",
        Some(&token),
        None,
    )
    .expect("peek");
    assert_eq!(peek.status, 200);
    assert_json_fields(&peek.body, &["matches", "count", "tokenUsage"]);

    let boot = request_json(
        port,
        "GET",
        "/boot?agent=adapter-conformance-sdk&budget=120",
        Some(&token),
        None,
    )
    .expect("boot");
    assert_eq!(boot.status, 200);
    assert_json_fields(
        &boot.body,
        &["bootPrompt", "tokenEstimate", "savings", "tokenUsage"],
    );

    let export =
        request_json(port, "GET", "/export?format=json", Some(&token), None).expect("export");
    assert_eq!(export.status, 200);
    assert_json_fields(&export.body, &["memories", "decisions"]);

    let initialize = mcp_rpc(
        port,
        &token,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "adapter-conformance", "version": "0.6.0" }
            }
        }),
    )
    .expect("mcp initialize");
    assert_eq!(initialize.status, 200);
    assert_json_fields(&initialize.body, &["jsonrpc", "id", "result"]);

    let tools = mcp_rpc(
        port,
        &token,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .expect("mcp tools/list");
    assert_eq!(tools.status, 200);
    let tool_names: BTreeSet<&str> = tools.body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    for required in [
        "cortex_health",
        "cortex_store",
        "cortex_recall",
        "cortex_boot",
        "cortex_peek",
    ] {
        assert!(tool_names.contains(required), "missing MCP tool {required}");
    }

    assert_mcp_tool_ok(
        &mcp_rpc(
            port,
            &token,
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "cortex_health", "arguments": {} }
            }),
        )
        .expect("mcp cortex_health")
        .body,
    );

    assert_mcp_tool_ok(
        &mcp_rpc(
            port,
            &token,
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "cortex_store",
                    "arguments": {
                        "decision": "Adapter conformance MCP memory",
                        "context": "C4 MCP contract"
                    }
                }
            }),
        )
        .expect("mcp cortex_store")
        .body,
    );

    assert_mcp_tool_ok(
        &mcp_rpc(
            port,
            &token,
            json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "cortex_recall",
                    "arguments": {
                        "query": "Adapter conformance MCP memory",
                        "budget": 120,
                        "k": 5
                    }
                }
            }),
        )
        .expect("mcp cortex_recall")
        .body,
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[derive(Debug)]
struct JsonHttpResponse {
    status: u16,
    body: Value,
}

fn spawn_daemon(home: &str, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(["serve", "--home", home, "--port", &port.to_string()])
        .env("CORTEX_SINGLE_DAEMON_TEST_BYPASS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn_tracked("spawn cortex serve")
}

fn request_json(
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
    let status = http_status(&response);
    let body = split_http_body(&response).ok_or_else(|| "missing HTTP body".to_string())?;
    let body = serde_json::from_str(body.trim()).map_err(|err| {
        format!("failed to parse JSON response for {method} {path}: {err}; body={body}")
    })?;
    Ok(JsonHttpResponse { status, body })
}

fn mcp_rpc(port: u16, token: &str, body: Value) -> Result<JsonHttpResponse, String> {
    request_json(port, "POST", "/mcp-rpc", Some(token), Some(body))
}

fn assert_json_fields(payload: &Value, fields: &[&str]) {
    for field in fields {
        assert!(
            payload.get(*field).is_some(),
            "missing field {field} in payload {payload}"
        );
    }
}

fn assert_mcp_tool_ok(payload: &Value) {
    assert!(
        payload.get("error").is_none(),
        "MCP tool returned JSON-RPC error: {payload}"
    );
    assert_eq!(payload["jsonrpc"], "2.0");
    assert!(
        payload["result"].is_object(),
        "missing MCP result: {payload}"
    );
    assert_ne!(
        payload["result"]["isError"], true,
        "MCP tool returned isError=true: {payload}"
    );
    let text = payload["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP tool text content");
    let parsed: Value = serde_json::from_str(text).expect("MCP tool text should be JSON");
    assert!(
        parsed.get("tokenUsage").is_some() || parsed.get("stats").is_some(),
        "MCP tool payload should expose result metadata: {parsed}"
    );
}

fn adapter_conformance_guard() -> MutexGuard<'static, ()> {
    static ADAPTER_CONFORMANCE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    ADAPTER_CONFORMANCE_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("adapter conformance mutex poisoned")
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

fn http_request(port: u16, request: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
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
