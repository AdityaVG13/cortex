use serde_json::{json, Value};
use std::fs;
use std::time::Duration;

mod support;
use support::{
    assert_path_scoped_to_home, daemon_spawn_test_guard, http_request, http_status, post_json,
    read_token, reserve_port, shutdown_daemon, spawn_daemon, split_http_body, unique_temp_dir,
    wait_for_exit, wait_for_health, normalize_path_for_compare,
};

#[test]
fn mcp_rpc_missing_auth_returns_jsonrpc_unauthorized() {
    let _guard = daemon_spawn_test_guard();
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
    assert_eq!(payload["id"], Value::Null);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_missing_x_cortex_request_with_non_local_origin_returns_forbidden_jsonrpc() {
    let _guard = daemon_spawn_test_guard();
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
    assert_eq!(payload["id"], Value::Null);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_local_context_without_x_cortex_request_is_forbidden() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("mcp_rpc_local_missing_header");
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
    assert_eq!(http_status(&response), 403);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["error"]["code"], -32600);
    assert_eq!(
        payload["error"]["message"],
        "Missing X-Cortex-Request header"
    );
    assert_eq!(payload["id"], Value::Null);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn mcp_rpc_x_auth_header_alias_is_rejected() {
    let _guard = daemon_spawn_test_guard();
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
    assert_eq!(http_status(&response), 401);
    let body = split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json payload");
    assert_eq!(payload["jsonrpc"], "2.0");
    assert_eq!(payload["error"]["code"], -32600);
    assert_eq!(payload["error"]["message"], "Unauthorized");
    assert_eq!(payload["id"], Value::Null);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn health_runtime_paths_remain_scoped_to_requested_home() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("health_runtime_paths");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    let public_response = http_request(
        port,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .expect("health request");
    assert_eq!(http_status(&public_response), 200);
    let public_body = split_http_body(&public_response).expect("public http body");
    let public_payload: Value =
        serde_json::from_str(public_body.trim()).expect("public json payload");

    let public_runtime = public_payload
        .get("runtime")
        .and_then(|value| value.as_object())
        .expect("public runtime object");
    let public_stats = public_payload
        .get("stats")
        .and_then(|value| value.as_object())
        .expect("public stats object");
    assert_eq!(
        public_stats.get("home"),
        None,
        "public health payload should redact stats.home"
    );
    for key in ["token_path", "db_path", "pid_path"] {
        assert_eq!(
            public_runtime.get(key),
            None,
            "public health payload should redact runtime.{key}"
        );
    }

    let private_response = http_request(
        port,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Cortex-Request: true\r\nConnection: close\r\n\r\n",
    )
    .expect("private health request");
    assert_eq!(http_status(&private_response), 200);
    let private_body = split_http_body(&private_response).expect("private http body");
    let payload: Value = serde_json::from_str(private_body.trim()).expect("private json payload");

    let runtime = payload
        .get("runtime")
        .and_then(|value| value.as_object())
        .expect("runtime object");
    let stats = payload
        .get("stats")
        .and_then(|value| value.as_object())
        .expect("stats object");

    let expected_home = normalize_path_for_compare(&home);
    let reported_home = stats
        .get("home")
        .and_then(|value| value.as_str())
        .expect("stats.home");
    assert_eq!(
        normalize_path_for_compare(reported_home),
        expected_home,
        "stats.home must be the requested home"
    );

    let expected_paths = [
        ("token_path", home_dir.join("cortex.token")),
        ("db_path", home_dir.join("cortex.db")),
        ("pid_path", home_dir.join("cortex.pid")),
    ];
    for (key, expected) in expected_paths {
        let reported = runtime
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| panic!("runtime.{key}"));
        assert_eq!(
            normalize_path_for_compare(reported),
            normalize_path_for_compare(&expected.to_string_lossy()),
            "{key} must resolve inside the requested home"
        );
        assert_path_scoped_to_home(key, reported, &expected_home);
    }

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
