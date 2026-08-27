// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use serde_json::json;
#[test]
fn prune_event_type_keep_latest_trims_old_rows() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                source_agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .expect("create events table");
    for idx in 0..6 {
        conn.execute("INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')", rusqlite::params![format!("{{\"idx\":{idx}}}")])
            .expect("insert event");
    }
    prune_event_type_keep_latest(&conn, "decision_stored", 3).expect("prune rows");
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM events WHERE type = 'decision_stored'", [], |row| row.get(0)).expect("count rows");
    assert_eq!(count, 3);
}
#[test]
fn log_event_compacts_large_merge_payload() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                source_agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .expect("create events table");
    let incoming = "x".repeat(10_000);
    log_event(
        &conn,
        "merge",
        json!({
            "target_id": 42,
            "target_type": "decision",
            "incoming_text": incoming,
            "source_agent": "test-agent"
        }),
        "test",
    )
    .expect("log merge event");
    let payload: String = conn.query_row("SELECT data FROM events WHERE type = 'merge' LIMIT 1", [], |row| row.get(0)).expect("read payload");
    let parsed: Value = serde_json::from_str(&payload).expect("valid json");
    assert!(parsed.get("incoming_text").is_none());
    assert_eq!(parsed["incoming_chars"].as_i64(), Some(10_000));
    assert!(parsed["incoming_preview"].as_str().map(|text| text.len() <= MERGE_EVENT_PREVIEW_CHARS).unwrap_or(false));
}
#[test]
fn log_event_keeps_recall_analytics_fields_small() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                source_agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .expect("create events table");
    log_event(
        &conn,
        "recall_query",
        json!({
            "agent": "codex",
            "query": "daemon ownership lock protects startup arbitration",
            "budget": 240,
            "spent": 52,
            "saved": 188,
            "hits": 3,
            "mode": "balanced",
            "cached": false,
            "tier": "hybrid_fusion",
            "latency_ms": 12,
            "semantic_route": {
                "mode": "baseline",
                "reason": "not_sampled",
                "sampled": false,
                "trialPercent": 1,
                "ranked_sources": ["a", "b", "c", "d", "e"]
            },
            "shadow_semantic": {
                "status": "unavailable",
                "reason": "query_embedding_unavailable",
                "baselineTopSources": ["very", "large", "list"],
                "shadowTopSources": ["another", "big", "list"]
            },
            "method_breakdown": {
                "keyword": 2,
                "semantic": 1,
                "unused_verbose_blob": "x".repeat(2000)
            }
        }),
        "codex",
    )
    .expect("log recall event");
    let (payload, bytes): (String, i64) = conn
        .query_row("SELECT data, LENGTH(data) FROM events WHERE type = 'recall_query' LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("read payload");
    let parsed: Value = serde_json::from_str(&payload).expect("valid json");
    assert_eq!(parsed["saved"].as_i64(), Some(188));
    assert_eq!(parsed["budget"].as_i64(), Some(240));
    assert_eq!(parsed["hits"].as_i64(), Some(3));
    assert_eq!(parsed["semantic_route"]["mode"].as_str(), Some("baseline"));
    assert_eq!(parsed["shadow_semantic"]["status"].as_str(), Some("unavailable"));
    assert!(parsed["shadow_semantic"]["baselineTopSources"].is_null());
    assert!(parsed["shadow_semantic"]["shadowTopSources"].is_null());
    assert!(bytes as usize <= MAX_EVENT_JSON_BYTES);
}
#[test]
fn log_event_skips_non_persistent_benchmark_noise() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                source_agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .expect("create events table");
    log_event(
        &conn,
        "recall_query",
        json!({
            "agent": "amb-cortex::run-a",
            "query": "benchmark probe",
            "saved": 50,
            "spent": 20,
            "budget": 70,
            "hits": 1,
            "method_breakdown": json!({
                "alpha": "x".repeat(1024),
                "beta": "x".repeat(1024),
                "gamma": "x".repeat(1024),
                "delta": "x".repeat(1024),
                "epsilon": "x".repeat(1024),
                "zeta": "x".repeat(1024),
                "eta": "x".repeat(1024),
                "theta": "x".repeat(1024)
            })
        }),
        "rust-daemon",
    )
    .expect("skip benchmark recall noise");
    log_event(
        &conn,
        "agent_boot",
        json!({
            "agent": "amb-cortex::run-a",
            "bytes_before": 1,
            "bytes_after": 1
        }),
        "rust-daemon",
    )
    .expect("skip benchmark agent_boot noise");
    log_event(
        &conn,
        "decision_stored",
        json!({
            "id": 42,
            "source_agent": "amb-cortex::run-a"
        }),
        "rust-daemon",
    )
    .expect("skip benchmark decision_stored noise");
    let skipped_count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0)).expect("count skipped rows");
    assert_eq!(skipped_count, 0);
    log_event(
        &conn,
        "recall_query",
        json!({
            "agent": "codex",
            "query": "production request",
            "saved": 12,
            "spent": 8,
            "budget": 20,
            "hits": 1
        }),
        "codex",
    )
    .expect("persist non-benchmark event");
    let persisted_count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0)).expect("count persisted rows");
    assert_eq!(persisted_count, 1);
}
#[test]
fn log_event_payload_fallback_keeps_savings_fields_bounded() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch(
        "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT NOT NULL,
                source_agent TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
    )
    .expect("create events table");
    let mut method_breakdown = serde_json::Map::new();
    for idx in 0..24 {
        method_breakdown.insert(format!("bucket_{idx}"), Value::String("x".repeat(1024)));
    }
    log_event(
        &conn,
        "recall_query",
        json!({
            "agent": "codex",
            "query": "q".repeat(1200),
            "budget": 240,
            "spent": 52,
            "saved": 188,
            "hits": 3,
            "mode": "balanced",
            "cached": false,
            "tier": "hybrid_fusion",
            "latency_ms": 12,
            "semantic_route": {
                "mode": "baseline",
                "reason": "not_sampled",
                "sampled": false,
                "trialPercent": 1
            },
            "shadow_semantic": {
                "status": "unavailable",
                "reason": "query_embedding_unavailable",
                "baselineTopSources": ["very", "large", "list"]
            },
            "method_breakdown": Value::Object(method_breakdown)
        }),
        "codex",
    )
    .expect("log oversized recall event");
    let (payload, bytes): (String, i64) = conn
        .query_row("SELECT data, LENGTH(data) FROM events WHERE type = 'recall_query' LIMIT 1", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("read payload");
    let parsed: Value = serde_json::from_str(&payload).expect("valid json");
    assert_eq!(parsed["truncated"].as_bool(), Some(true));
    assert_eq!(parsed["saved"].as_i64(), Some(188));
    assert_eq!(parsed["budget"].as_i64(), Some(240));
    assert_eq!(parsed["hits"].as_i64(), Some(3));
    assert_eq!(parsed["agent"].as_str(), Some("codex"));
    assert!(parsed["query"].as_str().map(|query| query.chars().count() <= 120).unwrap_or(false), "query should stay bounded in fallback payload");
    assert!(bytes as usize <= MAX_EVENT_JSON_BYTES);
}
