#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use support::{
    read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon, unique_temp_dir,
    wait_for_exit, wait_for_health,
};

// Eight topically distinct decisions. Shared tokens are minimized so the
// agreement-merge dedupe cannot fold them together; each carries enough
// specificity to pass the store quality gate.
const SENTINELS: [&str; 8] = [
    "Ingest queue rings at 4096 entries before the oldest tile is evicted.",
    "Ledger compaction runs nightly at 03:00 UTC on the primary shard.",
    "Retrieval index rebuilds incrementally after every 500 stores.",
    "External API gateway pins TLS 1.3 and rejects older handshakes.",
    "Config loader treats unknown keys as fatal boot errors.",
    "Scheduler caps eight concurrent tiles per worker process.",
    "Export writer flushes to disk every 32 megabytes.",
    "Health probe times out a backend after 2500 milliseconds.",
];

fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn store_sentinel(port: u16, token: &str, text: &str) {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({
            "decision": text,
            "type": "decision",
            "source_agent": "crash-durability",
            "confidence": 0.9,
        })),
    )
    .unwrap_or_else(|e| panic!("store {text:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must ack 200, got {} body {}",
        resp.status, resp.body
    );
    assert_eq!(
        resp.body["stored"].as_bool(),
        Some(true),
        "store must ack stored:true for {text:?}, got {}",
        resp.body
    );
}

fn recall_exact(port: u16, token: &str, expected: &str) {
    let path = format!(
        "/recall?q={}&k=10&budget=320",
        urlencoding(expected)
    );
    let resp = request_json(port, "GET", &path, Some(token), None)
        .unwrap_or_else(|e| panic!("recall for {expected:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "recall must return 200, got {} body {}",
        resp.status, resp.body
    );
    let results = resp.body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("recall results missing: {}", resp.body));
    let hit = results
        .iter()
        .any(|item| item["excerpt"].as_str() == Some(expected));
    assert!(
        hit,
        "recall must return the exact stored text {expected:?}, got {results:?}"
    );
}

fn sigkill(child: &mut std::process::Child) {
    child.kill().expect("SIGKILL daemon child");
    let status = child.wait().expect("reap SIGKILLed daemon child");
    assert!(
        !status.success(),
        "daemon must not exit cleanly across the crash boundary, got {status}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(9),
            "crash boundary must be an actual SIGKILL, got {status}"
        );
    }
}

#[test]
fn acked_stores_survive_sigkill() {
    let home_dir = unique_temp_dir("cd_sigkill_acks");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    for text in &SENTINELS {
        store_sentinel(port, &token, text);
    }

    // Sanity while alive: the acked path is recallable before the crash.
    recall_exact(port, &token, SENTINELS[2]);

    // Crash boundary: SIGKILL, no graceful shutdown, no checkpoint request.
    sigkill(&mut daemon);

    // Fresh daemon on the SAME home; new port is fine.
    let port2 = reserve_port();
    let mut daemon2 = spawn_daemon(&home, port2);
    wait_for_health(port2, &mut daemon2);
    let token2 = read_token(&home_dir);

    for text in &SENTINELS {
        recall_exact(port2, &token2, text);
    }

    // The recovered database must be structurally sound, not merely readable.
    let conn = rusqlite::Connection::open(home_dir.join("cortex.db"))
        .expect("open survived db for quick_check");
    let _ = conn.busy_timeout(Duration::from_millis(2000));
    let quick_check: Vec<String> = conn
        .prepare("PRAGMA quick_check")
        .expect("prepare quick_check")
        .query_map([], |row| row.get(0))
        .expect("run quick_check")
        .collect::<Result<_, _>>()
        .expect("collect quick_check");
    assert_eq!(
        quick_check, ["ok"],
        "quick_check must report the recovered db is intact: {quick_check:?}"
    );

    shutdown_daemon(port2, &home_dir);
    wait_for_exit(&mut daemon2, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn boot_after_sigkill_reports_healthy() {
    let home_dir = unique_temp_dir("cd_sigkill_health");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    store_sentinel(port, &token, SENTINELS[0]);
    sigkill(&mut daemon);

    let port2 = reserve_port();
    let mut daemon2 = spawn_daemon(&home, port2);
    wait_for_health(port2, &mut daemon2);

    // Pinned by build_health_payload: status is "ok" exactly when neither
    // degraded nor db_corrupted is set (the complement of the degraded pin in
    // failure_classes::health_reports_db_corruption_without_crashing).
    let health = request_json(port2, "GET", "/health", None, None).expect("health request");
    assert_eq!(
        health.status, 200,
        "health must return 200, got {} body {}",
        health.status, health.body
    );
    assert_eq!(
        health.body["status"].as_str(),
        Some("ok"),
        "fresh boot after SIGKILL must report status ok, got {}",
        health.body
    );
    assert_eq!(
        health.body["degraded"].as_bool(),
        Some(false),
        "crash recovery must not set degraded, got {}",
        health.body
    );
    assert_eq!(
        health.body["db_corrupted"].as_bool(),
        Some(false),
        "WAL recovery after SIGKILL must not set db_corrupted, got {}",
        health.body
    );

    shutdown_daemon(port2, &home_dir);
    wait_for_exit(&mut daemon2, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
