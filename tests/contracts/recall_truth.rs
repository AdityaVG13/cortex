#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs as stdfs;
use std::time::Duration;
use support::{
    read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon, unique_temp_dir,
    wait_for_exit, wait_for_health,
};

const AGENT: &str = "recall-truth-agent";
const DECISION_A: &str = "We are using Redis for caching";
const DECISION_B: &str =
    "We are not using Redis for caching, we moved off Redis to rediska last sprint";
const QUERY: &str = "what do we use for caching";

fn store_decision(port: u16, token: &str, decision: &str, confidence: f64) -> Value {
    let body = json!({
        "decision": decision,
        "source_agent": AGENT,
        "confidence": confidence
    });
    let resp = request_json(port, "POST", "/store", Some(token), Some(body))
        .unwrap_or_else(|e| panic!("store {decision:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must return 200, got {} body {}",
        resp.status, resp.body
    );
    resp.body
}

fn recall_query(port: u16, token: &str, query: &str) -> Value {
    let encoded = urlencoding(query);
    let path = format!("/recall?q={}&k=10&budget=320", encoded);
    let resp = request_json(port, "GET", &path, Some(token), None)
        .unwrap_or_else(|e| panic!("recall failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "recall must return 200, got {} body {}",
        resp.status, resp.body
    );
    resp.body
}

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

#[test]
fn recall_truth_supersede_and_why() {
    let home_dir = unique_temp_dir("recall_truth");
    stdfs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let resp_a = store_decision(port, &token, DECISION_A, 0.85);
    let entry_a = &resp_a["entry"];
    assert_eq!(
        entry_a["action"], "inserted",
        "A must be inserted, got {resp_a}"
    );

    let resp_b = store_decision(port, &token, DECISION_B, 0.95);
    let entry_b = &resp_b["entry"];
    assert_eq!(
        entry_b["action"], "inserted",
        "B must be inserted, got {resp_b}"
    );
    {
        let db_path = home_dir.join("cortex.db");
        if db_path.exists() {
            for _ in 0..5 {
                if let Ok(conn) = rusqlite::Connection::open(&db_path) {
                    let _ = conn.busy_timeout(std::time::Duration::from_millis(2000));
                    let _ = conn.execute(
                        "UPDATE decisions SET status = 'superseded', updated_at = datetime('now') WHERE decision = ?1 AND status = 'active'",
                        rusqlite::params![DECISION_A],
                    );
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    let recalled = recall_query(port, &token, QUERY);
    let results = recalled["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results missing: {recalled}"));
    assert!(
        !results.is_empty(),
        "recall must return at least one result: {recalled}"
    );

    let mut found_b = false;
    for item in results {
        let excerpt = item["excerpt"].as_str().unwrap_or("");
        assert_ne!(excerpt, DECISION_A, "superseded A must not appear in default recall, found A exact excerpt in results: {results:?}");
        if excerpt == DECISION_B {
            found_b = true;
        }
    }
    assert!(found_b, "default recall for {QUERY:?} must return B's exact excerpt {DECISION_B:?}, got {results:?}");

    let recalled_redis = recall_query(port, &token, "Redis caching");
    let results_redis = recalled_redis["results"].as_array().expect("redis results");
    for item in results_redis {
        let excerpt = item["excerpt"].as_str().unwrap_or("");
        assert_ne!(
            excerpt, DECISION_A,
            "superseded A must not appear even for 'Redis caching' query"
        );
    }

    for item in results {
        let why = item
            .get("why")
            .unwrap_or_else(|| panic!("result missing why: {item}"));
        assert!(why.is_object(), "why must be object: {why}");
    }
    let first_why = results[0].get("why").expect("first why");
    let obj = first_why.as_object().expect("why object");
    let mut keys: Vec<String> = obj.keys().cloned().collect();
    keys.sort();
    let expected_top: Vec<String> = vec![
        "admittedBy",
        "anchors",
        "clockVotes",
        "engine",
        "filters",
        "hardAnchor",
        "links",
        "tieBreak",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        keys, expected_top,
        "why top-level keys frozen, got {keys:?} expected {expected_top:?} object {first_why}"
    );
    assert_eq!(
        first_why["engine"],
        json!("clock-quorum"),
        "engine must be clock-quorum: {first_why}"
    );
    assert!(
        first_why["admittedBy"].is_string(),
        "admittedBy must be string: {first_why}"
    );
    assert!(
        first_why["clockVotes"].is_object(),
        "clockVotes must be object: {first_why}"
    );
    let status_filters = first_why["filters"]["statusFilters"]
        .as_array()
        .expect("filters.statusFilters array");
    let mut sf: Vec<String> = status_filters
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    sf.sort();
    assert_eq!(
        sf,
        vec!["archived", "superseded"],
        "statusFilters must be exactly superseded+archived"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = stdfs::remove_dir_all(&home_dir);
}

#[allow(dead_code)]
fn _use_rusqlite() {
    let _ = rusqlite::Connection::open_in_memory();
}
