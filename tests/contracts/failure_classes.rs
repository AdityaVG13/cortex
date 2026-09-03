use cortex_daemon::handlers::health::build_health_payload;
use cortex_daemon::state::RuntimeState;
use cortex_tests::support::solo_state;
use serde_json::{json, Value};
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[path = "../support/mod.rs"]
mod support;
use support::{
    read_token, request_json, request_json_with_headers, reserve_port, shutdown_daemon,
    spawn_daemon, unique_temp_dir, wait_for_exit, wait_for_health,
};

#[test]
fn http_store_rejects_malformed_json_body_with_400() {
    let home_dir = unique_temp_dir("fc_garbage_json");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

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
    let body = support::split_http_body(&response)
        .expect("http body")
        .trim()
        .to_string();
    assert!(
        !body.is_empty(),
        "malformed body rejection must carry an error body"
    );
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

    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({ "decision": "x", "type": "decision" })),
    )
    .expect("store request");
    assert_eq!(
        store.status, 400,
        "vague decision must be rejected with 400"
    );
    assert_eq!(
        store.body["error"], "Memory too vague",
        "vague decision rejection must carry the exact validation message: {}",
        store.body
    );
    assert!(
        store.body.get("quality").is_some() && store.body.get("factors").is_some(),
        "vague decision rejection must carry quality/factors evidence: {}",
        store.body
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn http_rejects_missing_bearer_token_with_401() {
    let home_dir = unique_temp_dir("fc_auth_missing");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    let store = request_json_with_headers(
        port,
        "POST",
        "/store",
        &[("X-Cortex-Request", "true")],
        Some(json!({ "decision": "nope" })),
    );
    let store = store.expect("store request");
    assert_eq!(
        store.status, 401,
        "missing bearer token must be refused with 401"
    );
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
    assert_eq!(
        status, 401,
        "malformed bearer token must be refused with 401"
    );
    let body = support::split_http_body(&response).expect("http body");
    let payload: Value = serde_json::from_str(body.trim()).expect("json error payload");
    assert_eq!(payload["error"], "Unauthorized");

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn health_reports_db_corruption_without_crashing() {
    let state: RuntimeState = solo_state();
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

    // "Without loss" must mean more than client-side acks. Near-identical
    // sentinels trip the daemon's agreement-merge dedupe (jaccard > merge
    // threshold): the 12 acks must survive inside ONE merged decision
    // (merged_count == 11) with every acknowledged text retrievable from the
    // row itself -- not 12 literal rows.
    let conn = rusqlite::Connection::open(home_dir.join("cortex.db"))
        .expect("open daemon db for loss check");
    let _ = conn.busy_timeout(Duration::from_millis(2000));
    let (row_count, merged_count, decision_text, context_text): (i64, i64, String, Option<String>) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(MAX(merged_count), -1), COALESCE(MAX(decision), ''), \
             COALESCE(MAX(context), '') FROM decisions \
             WHERE decision LIKE 'concurrent sentinel memory number %'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read persisted concurrent sentinels");
    assert_eq!(
        row_count, 1,
        "near-identical concurrent stores must agreement-merge into exactly one row, found {row_count}"
    );
    assert_eq!(
        merged_count, 11,
        "the merged row must record all 11 merged acks, found merged_count={merged_count}"
    );
    let stored_everywhere = format!("{decision_text}\n\n{}", context_text.unwrap_or_default());
    for i in 0..n {
        let sentinel = format!("concurrent sentinel memory number {i} with enough specificity to pass the quality gate");
        assert!(
            stored_everywhere.contains(&sentinel),
            "acknowledged store {i} must survive the merge verbatim"
        );
    }
    let (stored_events, merge_events): (i64, i64) = conn
        .query_row(
            "SELECT \
             (SELECT COUNT(*) FROM events WHERE type = 'decision_stored'), \
             (SELECT COUNT(*) FROM events WHERE type = 'decision_agreement_merge')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read store/merge event counts");
    assert_eq!(
        stored_events, 1,
        "exactly one decision_stored event for the surviving row"
    );
    assert_eq!(
        merge_events, 11,
        "exactly 11 decision_agreement_merge events, one per merged ack"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
