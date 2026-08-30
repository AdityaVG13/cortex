#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

const AGENT: &str = "history-agent";
const DECISION_A: &str = "We chose sqlite WAL journaling HISTORYALPHA for the ledger";
const DECISION_B: &str = "Billing exports move to parquet snapshots HISTORYBETA nightly";
const DECISION_C: &str = "Frontend bundle splits vendor chunks HISTORYGAMMA aggressively";

fn store(port: u16, token: &str, decision: &str) -> Value {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({"decision": decision, "source_agent": AGENT, "confidence": 0.9})),
    )
    .unwrap_or_else(|e| panic!("store {decision:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must return 200, body {}",
        resp.body
    );
    assert_eq!(
        resp.body["stored"],
        json!(true),
        "store must return stored:true, body {}",
        resp.body
    );
    resp.body
}

fn recall_excerpts(port: u16, token: &str, query: &str) -> Vec<String> {
    let encoded: String = query
        .bytes()
        .map(|b| {
            if b == b' ' {
                "%20".to_string()
            } else {
                (b as char).to_string()
            }
        })
        .collect();
    let resp = request_json(
        port,
        "GET",
        &format!("/recall?q={encoded}&k=10"),
        Some(token),
        None,
    )
    .unwrap_or_else(|e| panic!("recall {query:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "recall must return 200, body {}",
        resp.body
    );
    resp.body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results array missing, body {}", resp.body))
        .iter()
        .filter_map(|item| item["excerpt"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn rollback_hides_later_store_from_recall_and_boot() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("history");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let entry_a = store(port, &token, DECISION_A);
    let version_a = entry_a["entry"]["versionId"]
        .as_i64()
        .unwrap_or_else(|| panic!("store response must carry entry.versionId, body {entry_a}"));
    let entry_b = store(port, &token, DECISION_B);
    let version_b = entry_b["entry"]["versionId"]
        .as_i64()
        .expect("versionId for B");
    assert!(
        version_b > version_a,
        "versions must be monotonic: a={version_a} b={version_b}"
    );

    let before = recall_excerpts(port, &token, "HISTORYBETA parquet exports");
    assert!(
        before.iter().any(|e| e == DECISION_B),
        "pre-rollback recall must return B's exact text, got {before:?}"
    );

    let rb = request_json(
        port,
        "POST",
        "/rollback",
        Some(&token),
        Some(json!({"to": version_a})),
    )
    .expect("rollback request");
    assert_eq!(rb.status, 200, "rollback must return 200, body {}", rb.body);
    assert_eq!(rb.body["rolledBack"], json!(true), "body {}", rb.body);
    assert_eq!(
        rb.body["head"],
        json!(version_a),
        "head must move to A's version, body {}",
        rb.body
    );
    assert_eq!(
        rb.body["orphaned"],
        json!(1),
        "exactly B's version must be orphaned, body {}",
        rb.body
    );

    let after = recall_excerpts(port, &token, "HISTORYBETA parquet exports");
    for excerpt in &after {
        assert_ne!(
            excerpt, DECISION_B,
            "rolled-back decision must not surface, got {after:?}"
        );
        assert!(
            !excerpt.contains("HISTORYBETA"),
            "no excerpt may contain B's unique token, got {after:?}"
        );
    }

    let alpha = recall_excerpts(port, &token, "HISTORYALPHA sqlite ledger");
    assert!(
        alpha.iter().any(|e| e == DECISION_A),
        "A must still recall exactly, got {alpha:?}"
    );

    let boot = request_json(
        port,
        "GET",
        "/boot?agent=history-agent&budget=600",
        Some(&token),
        None,
    )
    .expect("boot");
    assert_eq!(boot.status, 200);
    let prompt = boot.body["bootPrompt"].as_str().expect("bootPrompt");
    assert!(
        !prompt.contains("HISTORYBETA"),
        "boot must not contain rolled-back token, got: {prompt}"
    );

    let entry_c = store(port, &token, DECISION_C);
    let version_c = entry_c["entry"]["versionId"]
        .as_i64()
        .expect("versionId for C");
    assert!(
        version_c > version_b,
        "branch continues with monotonic ids: b={version_b} c={version_c}"
    );
    let gamma = recall_excerpts(port, &token, "HISTORYGAMMA vendor chunks");
    assert!(
        gamma.iter().any(|e| e == DECISION_C),
        "post-rollback store must recall, got {gamma:?}"
    );
    let beta_again = recall_excerpts(port, &token, "HISTORYBETA parquet exports");
    for excerpt in &beta_again {
        assert!(
            !excerpt.contains("HISTORYBETA"),
            "B must stay hidden after branch, got {beta_again:?}"
        );
    }

    let versions = request_json(port, "GET", "/versions", Some(&token), None).expect("versions");
    assert_eq!(versions.status, 200);
    let row_b = versions.body["versions"]
        .as_array()
        .expect("versions array")
        .iter()
        .find(|v| v["id"] == json!(version_b))
        .unwrap_or_else(|| {
            panic!(
                "version {version_b} missing from log, body {}",
                versions.body
            )
        })
        .clone();
    assert_eq!(
        row_b["status"],
        json!("orphaned"),
        "B's version must be orphaned, got {row_b}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
