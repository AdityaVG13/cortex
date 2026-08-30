#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

const AGENT: &str = "temporal-agent";
const OLD_FACT: &str =
    "Always use Redis for caching in payments TEMPORALREDIS across all deployments";
const NEW_FACT: &str =
    "Never use Redis for caching in payments TEMPORALREDIS across all deployments";

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn store(port: u16, token: &str, decision: &str, confidence: f64) -> Value {
    let response = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({"decision":decision,"source_agent":AGENT,"source_model":"claude-opus","confidence":confidence})),
    )
    .unwrap_or_else(|error| panic!("store {decision:?} failed: {error}"));
    assert_eq!(
        response.status, 200,
        "store must return 200, body {}",
        response.body
    );
    response.body
}

fn excerpts(body: &Value) -> Vec<String> {
    body["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results array missing, body {body}"))
        .iter()
        .filter_map(|item| item["excerpt"].as_str().map(str::to_string))
        .collect()
}

fn recall(port: u16, token: &str, path: &str) -> Value {
    let response = request_json(port, "GET", path, Some(token), None)
        .unwrap_or_else(|error| panic!("GET {path} failed: {error}"));
    assert_eq!(
        response.status, 200,
        "GET {path} must return 200, body {}",
        response.body
    );
    response.body
}

#[test]
fn contradiction_closes_old_window_and_as_of_recovers_it() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("temporal-truth");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let old_store = store(port, &token, OLD_FACT, 0.65);
    let old_id = old_store["entry"]["id"].as_i64().expect("old id");
    let old_valid_from = old_store["entry"]["validFrom"]
        .as_str()
        .expect("old validFrom")
        .to_string();

    std::thread::sleep(Duration::from_millis(5));
    let new_store = store(port, &token, NEW_FACT, 0.99);
    let new_id = new_store["entry"]["id"].as_i64().expect("new id");
    let new_valid_from = new_store["entry"]["validFrom"]
        .as_str()
        .expect("new validFrom")
        .to_string();
    assert_ne!(
        old_valid_from, new_valid_from,
        "the test needs distinct validity boundaries"
    );
    assert_eq!(
        new_store["entry"]["classification"],
        json!("CONTRADICTS"),
        "body {new_store}"
    );
    assert_eq!(
        new_store["entry"]["supersedes"],
        json!(old_id),
        "body {new_store}"
    );

    let current = recall(
        port,
        &token,
        "/recall?q=TEMPORALREDIS%20Redis%20caching&k=10",
    );
    let current_excerpts = excerpts(&current);
    assert!(
        current_excerpts.iter().any(|excerpt| excerpt == NEW_FACT),
        "current recall must return the replacement, got {current_excerpts:?}"
    );
    assert!(
        !current_excerpts.iter().any(|excerpt| excerpt == OLD_FACT),
        "current recall must hide the closed fact, got {current_excerpts:?}"
    );

    let old_path = format!(
        "/as-of?q={}&t={}&k=10",
        percent_encode("TEMPORALREDIS Redis caching"),
        percent_encode(&old_valid_from)
    );
    let historical = recall(port, &token, &old_path);
    assert_eq!(historical["asOf"], json!(old_valid_from));
    let historical_excerpts = excerpts(&historical);
    assert!(
        historical_excerpts
            .iter()
            .any(|excerpt| excerpt == OLD_FACT),
        "historical recall must recover the old fact, got {historical_excerpts:?}"
    );
    assert!(
        !historical_excerpts
            .iter()
            .any(|excerpt| excerpt == NEW_FACT),
        "the replacement did not exist at the old boundary, got {historical_excerpts:?}"
    );
    let old_result = historical["results"]
        .as_array()
        .expect("historical results")
        .iter()
        .find(|item| item["excerpt"] == json!(OLD_FACT))
        .expect("old result metadata");
    assert_eq!(
        old_result["validUntil"],
        json!(new_valid_from),
        "the old window must close exactly when its replacement starts"
    );
    assert_eq!(
        old_result["status"],
        json!("superseded"),
        "historical losers must carry their exact present status"
    );

    let new_path = format!(
        "/as-of?q={}&t={}&k=10",
        percent_encode("TEMPORALREDIS Redis caching"),
        percent_encode(&new_valid_from)
    );
    let replacement_boundary = recall(port, &token, &new_path);
    let replacement_excerpts = excerpts(&replacement_boundary);
    assert!(
        replacement_excerpts
            .iter()
            .any(|excerpt| excerpt == NEW_FACT),
        "replacement must be valid at its own boundary, got {replacement_excerpts:?}"
    );
    assert!(
        !replacement_excerpts
            .iter()
            .any(|excerpt| excerpt == OLD_FACT),
        "validity windows are half-open, got {replacement_excerpts:?}"
    );

    let boot = recall(port, &token, "/boot?agent=temporal-agent&budget=600");
    let boot_prompt = boot["bootPrompt"].as_str().expect("bootPrompt");
    let boot_lines = boot_prompt.lines().collect::<Vec<_>>();
    assert!(
        boot_lines.contains(&"## TRUTH"),
        "boot must expose the frozen TRUTH section, got {boot_prompt}"
    );
    let expected_fact_line = format!(
        "FACT? {NEW_FACT}  (valid {} → now)  [d{new_id}]",
        &new_valid_from[..10]
    );
    assert!(
        boot_lines.iter().any(|line| *line == expected_fact_line),
        "boot must render the exact trust sigil, validity window, and citeable id; expected {expected_fact_line:?}, got {boot_prompt}"
    );

    let dump = recall(port, &token, "/dump");
    let decisions = dump["decisions"].as_array().expect("dump decisions");
    let dumped_old = decisions
        .iter()
        .find(|decision| decision["id"] == json!(old_id))
        .expect("dump must retain superseded fact");
    let dumped_new = decisions
        .iter()
        .find(|decision| decision["id"] == json!(new_id))
        .expect("dump must include active replacement");
    assert_eq!(dumped_old["status"], json!("superseded"));
    assert_eq!(dumped_old["valid_until"], json!(new_valid_from));
    assert_eq!(dumped_new["status"], json!("active"));
    assert_eq!(dumped_new["valid_from"], json!(new_valid_from));
    assert_eq!(dumped_new["valid_until"], Value::Null);

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
