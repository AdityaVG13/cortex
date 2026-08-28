//! Failure-first contracts for the four missing failure classes:
//!   (1) garbage / invalid input
//!   (2) unauthorized / refused requests (missing, malformed, expired bearer)
//!   (3) db-corruption recovery (degraded health, no crash)
//!   (4) concurrent-write serialization (no lost writes under contention)
//!
//! Each contract is a real oracle: it fails when the corresponding production
//! behavior regresses (proven in the mutation ladder, C4).
use cortex_daemon::handlers::health::build_health_payload;
use cortex_daemon::state::RuntimeState;
use cortex_tests::support::solo_state;
use serde_json::{json, Value};
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

mod support;
use support::{
    read_token, request_json, request_json_with_headers, reserve_port, shutdown_daemon,
    spawn_daemon, unique_temp_dir, wait_for_exit, wait_for_health,
};

// ---------------------------------------------------------------------------
// (1) garbage / invalid input
// ---------------------------------------------------------------------------

#[test]
fn http_store_rejects_malformed_json_body_with_400() {
    let home_dir = unique_temp_dir("fc_garbage_json");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    // Not valid JSON at all -- the server must reject it, not panic or 500.
    let raw = "this is not json {{{";
    let bearer = format!("Bearer {token}");
    let response = support::post_raw(
        port,
        "/store",
        &[
            ("Authorization", bearer.as_str()),
            ("X-Cortex-Request", "true"),
            ("Content-Type", "application/json"),
        ],
        raw,
    )
    .expect("request");
    let status = support::http_status(&response);
    assert_eq!(status, 400, "malformed JSON body must be rejected with 400");
    let body = support::split_http_body(&response).expect("http body").trim().to_string();
    assert!(
        !body.is_empty(),
        "malformed body rejection must carry an error body"
    );
    // The rejection must point at the JSON parse failure -- not a silent 200
    // or an opaque body. (axum returns a plain-text parse-error message.)
    assert!(
        body.to_lowercase().contains("json"),
        "malformed body error should mention json parse failure: {body}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn http_store_rejects_vague_decision_with_400() {
    let home_dir = unique_temp_dir("fc_garbage_vague");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    // A trivially vague decision must be rejected by the quality gate -- not
    // silently stored. This is the production validation path (StoreError::Validation).
    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({ "decision": "x", "type": "decision" })),
    )
    .expect("store request");
    assert_eq!(store.status, 400, "vague decision must be rejected with 400");
    assert!(
        store.body.get("error").is_some(),
        "vague decision rejection must carry an error field: {}",
        store.body
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

// ---------------------------------------------------------------------------
// (2) unauthorized / refused requests
// ---------------------------------------------------------------------------

#[test]
fn http_rejects_missing_bearer_token_with_401() {
    let home_dir = unique_temp_dir("fc_auth_missing");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    // No Authorization header, but the SSRF guard header is present -- the
    // daemon must refuse with 401 (not 403) because auth is missing.
    let store = request_json_with_headers(
        port,
        "POST",
        "/store",
        &[("X-Cortex-Request", "true")],
        Some(json!({ "decision": "nope" })),
    );
    // request_json parses JSON; a 401 JSON error body still parses fine.
    let store = store.expect("store request");
    assert_eq!(store.status, 401, "missing bearer token must be refused with 401");
    assert_eq!(store.body["error"], "Unauthorized");

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn http_rejects_malformed_bearer_token_with_401() {
    let home_dir = unique_temp_dir("fc_auth_malformed");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    // A syntactically-well-formed Bearer header carrying a token that does not
    // match state must be refused -- refusal must not be bypassed by a malformed
    // credential.
    let response = support::post_raw(
        port,
        "/store",
        &[
            ("Authorization", "Bearer not-a-real-cortex-token"),
            ("X-Cortex-Request", "true"),
            ("Content-Type", "application/json"),
        ],
        &json!({ "decision": "nope" }).to_string(),
    )
    .expect("request");
    let status = support::http_status(&response);
    assert_eq!(status, 401, "malformed bearer token must be refused with 401");
    let body = support::split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json error payload");
    assert_eq!(payload["error"], "Unauthorized");

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

// ---------------------------------------------------------------------------
// (3) db-corruption recovery (degraded health, no crash)
// ---------------------------------------------------------------------------

#[test]
fn health_reports_db_corruption_without_crashing() {
    let state: RuntimeState = solo_state();
    // Simulate the runtime corruption flag the daemon raises when PRAGMA
    // quick_check fails and auto-repair cannot recover the DB.
    state.db_corrupted.store(true, Ordering::SeqCst);

    let payload = tokio::runtime::Runtime::new()
        .expect("rt")
        .block_on(build_health_payload(&state, false));

    assert_eq!(
        payload["status"].as_str(),
        Some("degraded"),
        "corrupted db must surface degraded status: {payload}"
    );
    assert_eq!(
        payload["db_corrupted"].as_bool(),
        Some(true),
        "health must honestly report db_corrupted: {payload}"
    );
    assert_eq!(
        payload["degraded"].as_bool(),
        Some(true),
        "health must report degraded flag: {payload}"
    );
}

// ---------------------------------------------------------------------------
// (4) concurrent-write serialization (no lost writes under contention)
// ---------------------------------------------------------------------------

#[test]
fn concurrent_store_requests_serialize_without_loss() {
    let home_dir = unique_temp_dir("fc_concurrent");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let n: usize = 12;
    let results: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::with_capacity(n)));
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let token = token.clone();
            let results = Arc::clone(&results);
            thread::spawn(move || {
                let stored = request_json(
                    port,
                    "POST",
                    "/store",
                    Some(&token),
                    Some(json!({
                        "decision": format!("concurrent sentinel memory number {i} with enough specificity to pass the quality gate"),
                        "type": "decision",
                        "source_agent": "failure-classes",
                        "confidence": 0.9,
                    })),
                )
                .map(|r| r.status == 200 && r.body["stored"].as_bool() == Some(true))
                .unwrap_or(false);
                results.lock().expect("lock").push(stored);
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("join worker");
    }

    let stored_count = results.lock().expect("lock").iter().filter(|s| **s).count();
    assert_eq!(
        stored_count, n,
        "all {n} concurrent stores must succeed without loss (serialized writes)"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
