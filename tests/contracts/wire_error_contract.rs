//! L1 wire-correctness contracts: content-types, error envelopes, and the
//! MCP JSON-RPC reply discipline (gauntlet pass 47 sweep, lens L1).
//!
//! Pinned contracts:
//! 1. EVERY daemon error body is an `application/json` envelope with a
//!    non-empty string `error` field. Failure history (RED pre-fix): axum's
//!    default `Json` extractor rejected malformed bodies / wrong content
//!    types with `text/plain` bodies from every POST route that takes a JSON
//!    body, so JSON-parsing clients (desktop control-center api-client, SDKs)
//!    could not read the daemon's error message.
//! 2. /mcp-rpc must NOT reply to JSON-RPC notifications. JSON-RPC 2.0 forbids
//!    replies to notifications; the MCP HTTP transport expresses this as
//!    202 Accepted with no body. Failure history (RED pre-fix): the daemon
//!    answered `200 {"jsonrpc":...nothing...}` i.e. `200 {}` — a reply.
//! 3. Status-code semantics that are load-bearing for clients: 405 + `allow`
//!    on POST-only routes, 400 envelope for missing required /recall query,
//!    deterministic single -32600/id:null response for batch arrays, -32700
//!    parse-error envelope on /mcp-rpc, and unauthenticated CORS preflight.

#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, http_request, http_status, read_token, reserve_port, shutdown_daemon,
    spawn_daemon, split_http_body, unique_temp_dir, wait_for_exit, wait_for_health,
};

struct Daemon {
    port: u16,
    token: String,
    home_dir: std::path::PathBuf,
    child: std::process::Child,
}

fn spawn(home_label: &str) -> Daemon {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir(home_label);
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut child = spawn_daemon(&home, port);
    wait_for_health(port, &mut child);
    let token = read_token(&home_dir);
    Daemon { port, token, home_dir, child }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        shutdown_daemon(self.port, &self.home_dir);
        wait_for_exit(&mut self.child, Duration::from_secs(10));
        let _ = fs::remove_dir_all(&self.home_dir);
    }
}

/// Send one raw request with arbitrary method/headers/body and return
/// (status, content-type, body-text). Headers are read from the wire, so a
/// `text/plain` rejection cannot hide behind a JSON-parsing helper.
fn raw(daemon: &Daemon, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (u16, String, String) {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n", daemon.port);
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    match body {
        Some(text) => {
            request.push_str(&format!("Content-Length: {}\r\n\r\n", text.len()));
            request.push_str(text);
        }
        None => request.push_str("\r\n"),
    }
    let response = http_request(daemon.port, &request).expect("raw wire request");
    let status = http_status(&response);
    let content_type = response
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-type").then(|| value.trim().to_string())
        })
        .unwrap_or_default();
    let body = split_http_body(&response).unwrap_or_default().trim().to_string();
    (status, content_type, body)
}

fn authed(daemon: &Daemon) -> Vec<(&'static str, String)> {
    vec![
        ("Authorization", format!("Bearer {}", daemon.token)),
        ("X-Cortex-Request", "true".to_string()),
    ]
}

/// The discriminating assertion for contract 1: status must match, the body
/// must be served as application/json, must parse as JSON, and must carry a
/// non-empty string `error` field. Any text/plain leak fails here.
fn assert_json_envelope(status: u16, content_type: &str, body: &str, expected_status: u16, context: &str) {
    assert_eq!(status, expected_status, "{context}: status, body {body}");
    assert!(
        content_type.starts_with("application/json"),
        "{context}: Content-Type must be application/json, got {content_type:?} (body {body})"
    );
    let parsed: Value = serde_json::from_str(body)
        .unwrap_or_else(|err| panic!("{context}: error body must be valid JSON: {err}; body {body}"));
    let error = parsed
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: envelope must carry a string `error` field, body {body}"));
    assert!(!error.trim().is_empty(), "{context}: `error` must be non-empty, body {body}");
}

/// Contract 1: malformed body and wrong-content-type requests to JSON-POST
/// routes get the daemon JSON envelope, never axum's text/plain rejection.
/// Covers the store family (400 syntax, 415 media type, 422 type mismatch),
/// the recall family, and one conductor route (B5 facade surface).
#[test]
fn wire_error_bodies_are_application_json_envelopes() {
    let daemon = spawn("cortex-wire-env");
    let auth = authed(&daemon);
    let mut headers: Vec<(&str, &str)> = auth.iter().map(|(k, v)| (*k, v.as_str())).collect();

    // 400 syntax error, correct content-type.
    headers.push(("Content-Type", "application/json"));
    let (status, ctype, body) = raw(&daemon, "POST", "/store", &headers, Some("{not json"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /store malformed JSON");

    // 415 unsupported media type: valid JSON sent as text/plain.
    let wrong_type: Vec<(&str, &str)> = headers
        .iter()
        .map(|(k, v)| (*k, if *k == "Content-Type" { "text/plain" } else { *v }))
        .collect();
    let (status, ctype, body) = raw(&daemon, "POST", "/store", &wrong_type, Some(r#"{"content":"wire-envelope-probe"}"#));
    assert_json_envelope(status, &ctype, &body, 415, "POST /store text/plain content-type");

    // 422 type mismatch: valid JSON that does not fit the request type.
    // (`decision` is Option<String> in StoreRequest; a number must die in the
    // extractor, not reach the handler.)
    let (status, ctype, body) = raw(&daemon, "POST", "/store", &headers, Some(r#"{"decision":123}"#));
    assert_json_envelope(status, &ctype, &body, 422, "POST /store type mismatch");

    // Recall POST family: 400 syntax error.
    let (status, ctype, body) = raw(&daemon, "POST", "/recall", &headers, Some("garbage"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /recall malformed JSON");

    // Conductor family (B5 facade surface): 400 syntax error on /lock.
    let (status, ctype, body) = raw(&daemon, "POST", "/lock", &headers, Some("{bad"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /lock malformed JSON");

    // Server-handler family (server/handlers.rs): these three routes live in
    // their own module and were missed by the pass that converted every other
    // handler to the envelope extractor. Failure history (RED pre-fix): axum's
    // default `Json` extractor leaked text/plain rejection bodies here.
    let (status, ctype, body) = raw(&daemon, "POST", "/focus/start", &headers, Some("{bad"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /focus/start malformed JSON");
    let (status, ctype, body) = raw(&daemon, "POST", "/focus/end", &headers, Some("{bad"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /focus/end malformed JSON");
    let (status, ctype, body) = raw(&daemon, "POST", "/rollback", &headers, Some("{bad"));
    assert_json_envelope(status, &ctype, &body, 400, "POST /rollback malformed JSON");

    // 415 media-type rejection on the same family: valid JSON sent as
    // text/plain must still get the daemon envelope, not axum's plaintext.
    let (status, ctype, body) = raw(&daemon, "POST", "/focus/end", &wrong_type, Some(r#"{"label":"wire-envelope-probe"}"#));
    assert_json_envelope(status, &ctype, &body, 415, "POST /focus/end text/plain content-type");
}

/// Contract 2: JSON-RPC notifications get NO reply body. The daemon must
/// answer 202 Accepted with an empty body, not `200 {}`.
#[test]
fn wire_mcp_rpc_notification_receives_no_reply() {
    let daemon = spawn("cortex-wire-notify");
    let auth = authed(&daemon);
    let headers: Vec<(&str, &str)> = auth
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .chain([("Content-Type", "application/json")])
        .collect();

    for notification in [
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/unknown-pass47"}"#,
    ] {
        let (status, ctype, body) = raw(&daemon, "POST", "/mcp-rpc", &headers, Some(notification));
        assert_eq!(status, 202, "notification must be answered 202, not replied to (body {body})");
        assert!(body.is_empty(), "notification response must carry no body, got {body}");
        assert!(!ctype.starts_with("application/json"), "a bodyless 202 must not claim a JSON body");
    }
}

/// Contract 3: /mcp-rpc request-shape semantics stay deterministic — parse
/// error is 400 + -32700 envelope; batch arrays are rejected with a single
/// -32600/id:null response (documented no-batch stance, MCP protocolVersion
/// 2024-11-05); request ids are echoed verbatim (string and number).
#[test]
fn wire_mcp_rpc_parse_error_batch_and_id_echo_shapes() {
    let daemon = spawn("cortex-wire-mcp");
    let auth = authed(&daemon);
    let headers: Vec<(&str, &str)> = auth
        .iter()
        .map(|(k, v)| (*k, v.as_str()))
        .chain([("Content-Type", "application/json")])
        .collect();

    let (status, ctype, body) = raw(&daemon, "POST", "/mcp-rpc", &headers, Some("{broken"));
    assert_eq!(status, 400, "parse error must be 400, body {body}");
    assert!(ctype.starts_with("application/json"), "parse error must be JSON, got {ctype:?}");
    let parsed: Value = serde_json::from_str(&body).expect("parse-error body is JSON");
    assert_eq!(parsed["jsonrpc"], "2.0");
    assert_eq!(parsed["error"]["code"], -32700, "body {body}");
    assert_eq!(parsed["id"], Value::Null);

    let batch = r#"[{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}},{"jsonrpc":"2.0","id":2,"method":"tools/list"}]"#;
    let (status, ctype, body) = raw(&daemon, "POST", "/mcp-rpc", &headers, Some(batch));
    assert_eq!(status, 200, "batch rejection is a JSON-RPC response, body {body}");
    assert!(ctype.starts_with("application/json"), "batch rejection must be JSON, got {ctype:?}");
    let parsed: Value = serde_json::from_str(&body).expect("batch rejection body is JSON");
    assert_eq!(parsed["error"]["code"], -32600, "batch must be rejected as invalid request, body {body}");
    assert_eq!(parsed["id"], Value::Null, "batch rejection carries null id, body {body}");
    assert!(parsed.get("result").is_none(), "no partial results may leak for a rejected batch");

    for id in [json!("wire-string-id"), json!(42)] {
        let request = json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{}}).to_string();
        let (status, _, body) = raw(&daemon, "POST", "/mcp-rpc", &headers, Some(&request));
        assert_eq!(status, 200);
        let parsed: Value = serde_json::from_str(&body).expect("initialize body is JSON");
        assert_eq!(parsed["id"], id, "request id must be echoed verbatim, body {body}");
        assert_eq!(parsed["result"]["protocolVersion"], "2024-11-05");
    }
}

/// Contract 4: method-mismatch, required-query, and CORS preflight semantics.
/// GET on a POST-only route is 405 with an `allow` header (not a misleading
/// 404); GET /recall without `q` is a 400 JSON envelope naming the parameter;
/// OPTIONS preflight is answered by the CORS layer without credentials.
#[test]
fn wire_method_mismatch_required_query_and_preflight_semantics() {
    let daemon = spawn("cortex-wire-semantics");
    let auth = authed(&daemon);
    let headers: Vec<(&str, &str)> = auth.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let (status, _, body) = raw(&daemon, "GET", "/store", &headers, None);
    assert_eq!(status, 405, "GET on POST-only /store must be 405, body {body}");
    let response = {
        let mut request = format!("GET /store HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n", daemon.port);
        for (name, value) in &headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        http_request(daemon.port, &request).expect("allow-header probe")
    };
    let allow = response
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("allow").then(|| value.trim().to_string())
        })
        .unwrap_or_default();
    assert_eq!(allow, "POST", "405 must advertise the supported method");

    let (status, ctype, body) = raw(&daemon, "GET", "/recall", &headers, None);
    assert_json_envelope(status, &ctype, &body, 400, "GET /recall missing q");
    let parsed: Value = serde_json::from_str(&body).expect("missing-q body is JSON");
    assert!(
        // "q" alone would match any message containing "query"/"required";
        // pin the full stable suffix so a mutant that stops naming the
        // parameter fails this oracle.
        parsed["error"].as_str().unwrap_or_default().contains("parameter: q"),
        "error must name the missing parameter, body {body}"
    );

    let preflight: Vec<(&str, &str)> = vec![
        ("Origin", "http://localhost:1420"),
        ("Access-Control-Request-Method", "POST"),
        ("Access-Control-Request-Headers", "authorization, x-cortex-request, content-type"),
    ];
    let (status, _, body) = raw(&daemon, "OPTIONS", "/store", &preflight, None);
    assert_eq!(status, 200, "unauthenticated CORS preflight must be answered, body {body}");
    assert!(body.is_empty(), "preflight answer carries no body, got {body}");
}
