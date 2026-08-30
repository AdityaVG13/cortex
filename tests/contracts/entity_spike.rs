#[path = "../support/mod.rs"]
mod support;

use serde_json::json;
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

fn store(port: u16, token: &str, decision: &str) {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({"decision": decision, "source_agent": "entity-spike", "confidence": 0.9})),
    )
    .unwrap_or_else(|e| panic!("store {decision:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must return 200, body {}",
        resp.body
    );
}

fn resolve(port: u16, token: &str, query: &str) -> Vec<(i64, String, String)> {
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
        &format!("/entities?q={encoded}"),
        Some(token),
        None,
    )
    .unwrap_or_else(|e| panic!("entities {query:?} failed: {e}"));
    assert_eq!(
        resp.status, 200,
        "entities must return 200, body {}",
        resp.body
    );
    resp.body["entities"]
        .as_array()
        .expect("entities array")
        .iter()
        .map(|e| {
            (
                e["id"].as_i64().expect("entity id"),
                e["qualifier"].as_str().expect("qualifier").to_string(),
                e["kind"].as_str().expect("kind").to_string(),
            )
        })
        .collect()
}

#[test]
fn aliases_resolve_to_one_entity_without_an_llm() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("entity_spike");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    store(
        port,
        &token,
        "The auth service issues OAuth2 bearer credentials for the dashboard",
    );
    store(
        port,
        &token,
        "OAuth microservice rotates signing keys weekly without downtime",
    );
    store(
        port,
        &token,
        "The payments service handles refunds through the ledger queue",
    );

    let via_auth = resolve(port, &token, "auth service");
    assert_eq!(
        via_auth.len(),
        1,
        "auth service must resolve to exactly one entity, got {via_auth:?}"
    );
    let (auth_id, qualifier, kind) = via_auth[0].clone();
    assert_eq!(
        qualifier, "auth",
        "canonical qualifier must be the first writer's, got {via_auth:?}"
    );
    assert_eq!(
        kind, "service",
        "kind class must be service, got {via_auth:?}"
    );

    let via_oauth = resolve(port, &token, "OAuth microservice");
    assert_eq!(
        via_oauth.len(),
        1,
        "OAuth microservice must resolve to one entity, got {via_oauth:?}"
    );
    assert_eq!(
        via_oauth[0].0, auth_id,
        "OAuth microservice must resolve to the auth entity, got {via_oauth:?}"
    );

    let via_login = resolve(port, &token, "login system");
    assert_eq!(
        via_login.len(),
        1,
        "login system must resolve to one entity, got {via_login:?}"
    );
    assert_eq!(
        via_login[0].0, auth_id,
        "login system must resolve to the auth entity, got {via_login:?}"
    );

    let via_payments = resolve(port, &token, "payments service");
    assert_eq!(
        via_payments.len(),
        1,
        "payments service must resolve to one entity, got {via_payments:?}"
    );
    assert_ne!(
        via_payments[0].0, auth_id,
        "payments must NOT merge into auth, got {via_payments:?}"
    );
    assert_eq!(
        via_payments[0].1, "payments",
        "payments qualifier exact, got {via_payments:?}"
    );

    let via_unknown = resolve(port, &token, "warp drive assembly");
    assert_eq!(
        via_unknown.len(),
        0,
        "unknown mention must resolve to zero entities, got {via_unknown:?}"
    );
    let via_unknown_again = resolve(port, &token, "warp drive assembly");
    assert_eq!(
        via_unknown_again.len(),
        0,
        "repeat query must not have created an entity, got {via_unknown_again:?}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}

#[test]
fn entity_arm_recalls_alias_row_with_zero_keyword_overlap() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("entity_arm");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    const DECISION: &str = "The auth service migrated to argon2 hashing yesterday";
    store(port, &token, DECISION);

    let resp = request_json(
        port,
        "GET",
        "/recall?q=login%20system&k=5",
        Some(&token),
        None,
    )
    .expect("recall");
    assert_eq!(
        resp.status, 200,
        "recall must return 200, body {}",
        resp.body
    );
    let results = resp.body["results"].as_array().expect("results array");
    let hit = results
        .iter()
        .find(|item| item["excerpt"].as_str() == Some(DECISION))
        .unwrap_or_else(|| {
            panic!(
                "entity arm must recall the alias row exactly, got {}",
                resp.body
            )
        });
    assert_eq!(
        hit["method"].as_str(),
        Some("entity"),
        "hit must come from the entity arm, got {hit}"
    );
    assert_eq!(
        hit["why"]["boosts"]["entity"].as_f64().map(|v| v > 0.0),
        Some(true),
        "why must attribute the entity arm, got {hit}"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
