#[path = "../support/mod.rs"]
mod support;
use rusqlite::Connection;
use serde_json::json;
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

const DUP_A: &str =
    "payments service cluster postgres migration rollout requires idempotency keys deploy window checklist";
const DUP_B: &str =
    "payments service cluster postgres migration rollout blocked pending schema review sign off";
const DUP_C: &str =
    "payments service cluster postgres migration rollout staged behind feature flag ramp plan";
const DUP_D: &str =
    "payments service cluster postgres migration rollout owner assigned oncall rotation handles paging";
const UNRELATED_E: &str =
    "The quarterly financial audit report needs review by the compliance team for regulatory approval";
const UNRELATED_F: &str =
    "Recipe for sourdough bread requires flour water salt and long fermentation overnight";

fn store_decision(port: u16, token: &str, text: &str) {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({
            "decision": text,
            "context": "crystallize-oracle",
            "type": "decision",
            "source_agent": "crystallize-test",
            "confidence": 0.9
        })),
    )
    .unwrap_or_else(|e| panic!("store {text:?} failed: {e}"));
    assert_eq!(resp.status, 200, "store {text:?} status {}", resp.status);
    assert_eq!(
        resp.body["stored"], true,
        "store must return stored true for {text:?}, got {}",
        resp.body
    );
}

#[test]
fn crystallize_groups_duplicates_and_is_idempotent() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("crystallize");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home_str = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home_str, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    for text in [DUP_A, DUP_B, DUP_C, DUP_D, UNRELATED_E, UNRELATED_F] {
        store_decision(port, &token, text);
    }

    let first =
        request_json(port, "POST", "/crystallize", Some(&token), None).expect("crystallize first");
    assert_eq!(first.status, 200, "crystallize status {}", first.status);
    let clusters = first.body["clusters"]
        .as_u64()
        .expect("clusters field missing or not u64");
    let created = first.body["created"]
        .as_u64()
        .expect("created field missing");
    let updated = first.body["updated"]
        .as_u64()
        .expect("updated field missing");
    let consolidated = first.body["consolidated"]
        .as_u64()
        .expect("consolidated field missing");
    assert!(
        clusters >= 1,
        "clusters must be >=1 after duplicates, got clusters={clusters} body={}",
        first.body
    );
    assert_eq!(
        created, clusters,
        "created must equal clusters (no updates path), got created={created} clusters={clusters}"
    );
    assert_eq!(
        updated, 0,
        "updated must be 0 for fresh clusters, got {updated}"
    );
    assert!(
        consolidated >= 4,
        "consolidated must be >=4 (the dupes), got {consolidated}"
    );

    let list = request_json(port, "GET", "/crystals", Some(&token), None).expect("list crystals");
    assert_eq!(list.status, 200, "crystals list status {}", list.status);
    assert_eq!(
        list.body["count"].as_u64().unwrap_or(0),
        clusters,
        "crystals count must equal clusters from pass, got {} vs {}",
        list.body["count"],
        clusters
    );
    let crystals = list.body["crystals"]
        .as_array()
        .expect("crystals array missing");
    assert!(!crystals.is_empty(), "crystals array must not be empty");
    let big = crystals
        .iter()
        .find(|c| c["members"].as_u64().unwrap_or(0) >= 4)
        .expect("must have a crystal with members >=4");
    let crystal_id = big["id"].as_i64().expect("crystal id missing");
    assert!(
        big["label"].as_str().unwrap_or("").len() > 0,
        "label must be non-empty"
    );
    let consolidated_text = big["text"].as_str().expect("text missing");
    assert!(
        [DUP_A, DUP_B, DUP_C, DUP_D].contains(&consolidated_text),
        "consolidated_text must be one of the dupes, got {consolidated_text:?}"
    );

    let db_path = home_dir.join("cortex.db");
    let conn = Connection::open(&db_path).expect("open db");
    let mut stmt = conn
        .prepare("SELECT source FROM cluster_members WHERE cluster_id = ?1 ORDER BY id ASC")
        .expect("prepare cluster_members");
    let sources: Vec<String> = stmt
        .query_map([crystal_id], |row| row.get::<_, String>(0))
        .expect("query members")
        .filter_map(Result::ok)
        .collect();

    let mut sorted_sources = sources.clone();
    sorted_sources.sort();
    let mut expected = vec![
        DUP_A.to_string(),
        DUP_B.to_string(),
        DUP_C.to_string(),
        DUP_D.to_string(),
    ];
    expected.sort();
    assert_eq!(
        sorted_sources, expected,
        "cluster members must be exactly the 4 dupes (sorted), got {sorted_sources:?}"
    );
    assert_eq!(
        sources.len(),
        4,
        "cluster must have exactly 4 members, got {}: {sources:?}",
        sources.len()
    );
    for s in &sources {
        assert!(
            [DUP_A, DUP_B, DUP_C, DUP_D].contains(&s.as_str()),
            "member source must be one of dupes, got {s:?}"
        );
    }

    for unrelated in [UNRELATED_E, UNRELATED_F] {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM cluster_members WHERE source = ?1",
                [unrelated],
                |row| row.get(0),
            )
            .expect("count unrelated");
        assert_eq!(
            count, 0,
            "unrelated text must NOT be a cluster member, got count={count} for {unrelated:?}"
        );
    }

    let second =
        request_json(port, "POST", "/crystallize", Some(&token), None).expect("crystallize second");
    assert_eq!(
        second.status, 200,
        "second crystallize status {}",
        second.status
    );
    assert_eq!(
        second.body["clusters"].as_u64().unwrap(),
        0,
        "second pass clusters must be 0, got {}",
        second.body
    );
    assert_eq!(
        second.body["created"].as_u64().unwrap(),
        0,
        "second pass created must be 0"
    );
    assert_eq!(
        second.body["consolidated"].as_u64().unwrap(),
        0,
        "second pass consolidated must be 0"
    );

    let list2 =
        request_json(port, "GET", "/crystals", Some(&token), None).expect("list crystals 2");
    assert_eq!(
        list2.body["count"].as_u64().unwrap(),
        list.body["count"].as_u64().unwrap(),
        "crystals count must be equal before/after second pass (idempotent)"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
