//! Budget / rate-limit denial contracts (bead cortex-mpo, HYP-019).
//!
//! Pins the exact wire shapes of the two declared denial surfaces:
//!   * HTTP 429 budget denial on POST /store
//!     (crates/daemon/src/handlers/auth/mod.rs:174 `budget_denial_response`,
//!     body from cortex_logic::budgets::BudgetDecision::http_body_json)
//!   * MCP JSON-RPC -32029 denial on POST /mcp-rpc
//!     (crates/daemon/src/server/router.rs:172-182)
//!
//! Determinism: the daemon reads `<home>/budgets.toml` exactly once at state
//! init (crates/daemon/src/state/init.rs:117 ->
//! BudgetConfigStatus::load_from_home), so each test writes a budgets.toml
//! with a tiny limit into its unique temp home BEFORE spawn_daemon. Budget
//! windows are sliding windows over `Instant::now()`
//! (crates/logic/src/rate_limit/mod.rs:214 `check_budget_for_endpoint`), so
//! with `window_seconds = 3600` a `limit = 2` trips deterministically on the
//! third back-to-back request. `retry_after_seconds` is the only
//! clock-derived field (3600 - elapsed_secs, floored at 1,
//! rate_limit/mod.rs:53): the contracts pin it to {3600, 3599, 3598} (three
//! back-to-back requests take < 2s) and pin every other field to its exact
//! value. Allowed-again-after-window would need a 3600s sleep and is
//! intentionally NOT asserted; the deterministic complement is pinned
//! instead: the denial persists on further requests while the window is
//! still full.

#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;

use support::{
    daemon_spawn_test_guard, http_status, post_json, read_token, reserve_port, shutdown_daemon,
    spawn_daemon, split_http_body, unique_temp_dir, wait_for_exit, wait_for_health,
};

const LIMIT: u64 = 2;
const WINDOW_SECONDS: u64 = 3600;
// Clock-derived retry pin: window minus the whole elapsed seconds across
// three back-to-back requests. Deterministic bound, not a loose matcher:
// anything outside these three values means the sliding-window retry math
// drifted.
const RETRY_AFTER_PINNED: [u64; 3] = [3600, 3599, 3598];

struct Daemon {
    home_dir: std::path::PathBuf,
    port: u16,
    token: String,
    child: std::process::Child,
}

fn budgets_toml(endpoint: &str) -> String {
    format!(
        "[defaults]\nenabled = true\n\n[endpoints.{endpoint}]\nlimit = {LIMIT}\nwindow_seconds = {WINDOW_SECONDS}\n"
    )
}

fn spawn_with_budgets(prefix: &str, budgets: Option<&str>) -> Daemon {
    let home_dir = unique_temp_dir(prefix);
    fs::create_dir_all(&home_dir).expect("create temp home");
    if let Some(contents) = budgets {
        fs::write(home_dir.join("budgets.toml"), contents).expect("write budgets.toml");
    }
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut child = spawn_daemon(&home, port);
    wait_for_health(port, &mut child);
    let token = read_token(&home_dir);
    Daemon {
        home_dir,
        port,
        token,
        child,
    }
}

fn shutdown(daemon: Daemon) {
    let Daemon {
        home_dir,
        port,
        mut child,
        ..
    } = daemon;
    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut child, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

fn store_raw(port: u16, token: &str, tag: &str) -> String {
    let auth = format!("Bearer {token}");
    let body = json!({
        "decision": format!("budget denial probe {tag}"),
        "source_agent": "budget-denial-contract",
    });
    post_json(
        port,
        "/store",
        &[("Authorization", &auth), ("X-Cortex-Request", "true")],
        &body.to_string(),
    )
    .expect("store request")
}

fn parse_body(response: &str) -> Value {
    let raw = split_http_body(response).expect("http body");
    serde_json::from_str(raw.trim()).expect("json body")
}

fn header_block(response: &str) -> &str {
    response.split("\r\n\r\n").next().expect("header block")
}

fn assert_exact_budget_denial(response: &str, endpoint: &str) {
    assert_eq!(
        http_status(response),
        429,
        "budget denial must be HTTP 429, got: {response}"
    );
    let mut body = parse_body(response);

    let retry = body["retry_after_seconds"]
        .as_u64()
        .expect("retry_after_seconds is a u64");
    assert!(
        RETRY_AFTER_PINNED.contains(&retry),
        "retry_after_seconds {retry} escaped the deterministic pin {RETRY_AFTER_PINNED:?}"
    );
    let retry_header = header_block(response)
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("retry-after:"))
        .expect("Retry-After header present on 429");
    assert_eq!(
        retry_header.split_once(':').unwrap().1.trim(),
        retry.to_string(),
        "Retry-After header must equal body retry_after_seconds"
    );
    assert!(
        header_block(response)
            .to_ascii_lowercase()
            .contains("cache-control: no-store"),
        "429 must be Cache-Control: no-store"
    );

    body["retry_after_seconds"] = json!(WINDOW_SECONDS);
    assert_eq!(
        body,
        json!({
            "error": "budget_exceeded",
            "endpoint": endpoint,
            "limit": LIMIT,
            "window_seconds": WINDOW_SECONDS,
            "retry_after_seconds": WINDOW_SECONDS,
            "source": "budgets.toml",
        }),
        "429 budget denial envelope drifted from the pinned shape"
    );
}

fn mcp_call(port: u16, token: &str, id: i64) -> (u16, Value) {
    let auth = format!("Bearer {token}");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "budget-denial-contract", "version": "1.0.0" }
        }
    });
    let response = post_json(
        port,
        "/mcp-rpc",
        &[("Authorization", &auth), ("X-Cortex-Request", "true")],
        &body.to_string(),
    )
    .expect("mcp-rpc request");
    (http_status(&response), parse_body(&response))
}

#[test]
fn store_budget_denial_returns_exact_429_envelope() {
    let _guard = daemon_spawn_test_guard();
    let daemon = spawn_with_budgets("budget_store_429", Some(&budgets_toml("store")));

    // limit = 2: exactly two stores fit in the window.
    for tag in ["allowed-1", "allowed-2"] {
        let response = store_raw(daemon.port, &daemon.token, tag);
        assert_eq!(http_status(&response), 200, "store {tag} must be allowed");
        assert_eq!(
            parse_body(&response)["stored"],
            json!(true),
            "store {tag} must be a real store, not an error shell"
        );
    }

    // Third and fourth back-to-back stores trip the budget; the fourth also
    // proves the denial persists while the window is still full (recovery
    // after the 3600s window would need a 3600s sleep and is not asserted).
    let denied = store_raw(daemon.port, &daemon.token, "denied-1");
    assert_exact_budget_denial(&denied, "store");
    let still_denied = store_raw(daemon.port, &daemon.token, "denied-2");
    assert_exact_budget_denial(&still_denied, "store");

    shutdown(daemon);
}

#[test]
fn mcp_budget_denial_returns_exact_jsonrpc_32029_object() {
    let _guard = daemon_spawn_test_guard();
    let daemon = spawn_with_budgets("budget_mcp_32029", Some(&budgets_toml("mcp")));

    // limit = 2: two initialize calls fit in the window.
    for id in [1, 2] {
        let (status, payload) = mcp_call(daemon.port, &daemon.token, id);
        assert_eq!(status, 200, "mcp call {id} must be allowed");
        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], json!(id));
        assert!(
            payload.get("error").is_none(),
            "mcp call {id} must not be denied: {payload}"
        );
    }

    // Third call trips the mcp budget: transport-level 200 with an exact
    // JSON-RPC error object (router.rs:172-182), echoing the request id.
    let (status, mut payload) = mcp_call(daemon.port, &daemon.token, 7);
    assert_eq!(
        status, 200,
        "MCP budget denial is a JSON-RPC error over HTTP 200, not a 429"
    );
    let retry = payload["error"]["data"]["retry_after_seconds"]
        .as_u64()
        .expect("error.data.retry_after_seconds is a u64");
    assert!(
        RETRY_AFTER_PINNED.contains(&retry),
        "retry_after_seconds {retry} escaped the deterministic pin {RETRY_AFTER_PINNED:?}"
    );
    payload["error"]["data"]["retry_after_seconds"] = json!(WINDOW_SECONDS);
    assert_eq!(
        payload,
        json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32029,
                "message": "budget_exceeded",
                "data": {
                    "error": "budget_exceeded",
                    "endpoint": "mcp",
                    "limit": LIMIT,
                    "window_seconds": WINDOW_SECONDS,
                    "retry_after_seconds": WINDOW_SECONDS,
                    "source": "budgets.toml",
                },
            },
            "id": 7,
        }),
        "MCP -32029 denial object drifted from the pinned shape"
    );

    shutdown(daemon);
}

#[test]
fn default_limits_do_not_false_positive_budget_denial() {
    let _guard = daemon_spawn_test_guard();
    let daemon = spawn_with_budgets("budget_no_false_positive", None);

    // No budgets.toml: budgets are absent, so neither the budget limiter nor
    // the request-class limiter (default loopback limits are 10000/min) may
    // deny normal traffic. Every store must be an exact 200 stored:true.
    for i in 1..=12 {
        let response = store_raw(daemon.port, &daemon.token, &format!("clean-{i}"));
        assert_eq!(
            http_status(&response),
            200,
            "store {i} must not be denied under default limits"
        );
        assert_eq!(parse_body(&response)["stored"], json!(true));
    }
    // Same negative claim for the MCP -32029 path.
    for id in 100..=103 {
        let (status, payload) = mcp_call(daemon.port, &daemon.token, id);
        assert_eq!(status, 200, "mcp call {id} must not be denied");
        assert!(
            payload.get("error").is_none(),
            "mcp call {id} must not be budget-denied under default limits: {payload}"
        );
    }

    shutdown(daemon);
}
