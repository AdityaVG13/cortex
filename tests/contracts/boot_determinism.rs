use serde_json::json;
use std::fs;
use std::thread;
use std::time::Duration;

#[path = "../support/mod.rs"]
mod support;
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};

fn store_decision(port: u16, token: &str, decision: &str, retention: &str, context: &str) {
    let resp = request_json(
        port,
        "POST",
        "/store",
        Some(token),
        Some(json!({
            "decision": decision,
            "context": context,
            "retention_class": retention,
            "source_agent": "boot-determinism-agent"
        })),
    )
    .unwrap_or_else(|e| panic!("store failed for {decision:?}: {e}"));
    assert_eq!(
        resp.status, 200,
        "store must return 200 for {decision:?}, got {resp:?}"
    );
    assert_eq!(
        resp.body["stored"],
        json!(true),
        "store must return stored:true, got {}",
        resp.body
    );
}

fn fetch_boot(port: u16, token: &str) -> String {
    let resp = request_json(
        port,
        "GET",
        "/boot?agent=boot-determinism-agent&budget=600",
        Some(token),
        None,
    )
    .unwrap_or_else(|e| panic!("boot fetch failed: {e}"));
    assert_eq!(resp.status, 200, "boot must return 200, got {resp:?}");
    let prompt = resp
        .body
        .get("bootPrompt")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("bootPrompt missing, body: {}", resp.body));
    assert!(prompt.len() > 0, "bootPrompt must be non-empty");
    assert_eq!(
        resp.body.get("profile").and_then(|v| v.as_str()),
        Some("capsules"),
        "profile must be capsules"
    );
    assert!(
        resp.body.get("tokenEstimate").is_some(),
        "tokenEstimate must be present"
    );
    assert!(
        resp.body
            .get("capsules")
            .and_then(|v| v.as_array())
            .is_some(),
        "capsules array must be present"
    );
    assert!(
        resp.body
            .get("savings")
            .and_then(|v| v.as_object())
            .is_some(),
        "savings object must be present"
    );
    prompt.to_string()
}

#[test]
fn boot_is_byte_identical_across_consecutive_calls_and_changes_after_write() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("boot_determinism");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let seeds = [
        ("BOOT_DETERMINISM_DURABLE_ALPHA The payments API contract requires idempotency keys on every write operation architecture", "durable", "C1 durable"),
        ("BOOT_DETERMINISM_DURABLE_BETA Architecture decision: Use Postgres for primary store with WAL durability", "durable", "C1 durable second"),
        ("BOOT_DETERMINISM_OPERATIONAL_GAMMA Operational note: Deploy to staging via cargo run --release pipeline", "operational", "ops context"),
        ("BOOT_DETERMINISM_AUDIT_DELTA Audit event: Permission grant for deploy pipeline reviewed by security team", "audit", "audit context"),
        ("BOOT_DETERMINISM_EPHEMERAL_EPSILON Scratch: Temporary cache key for ephemeral test data transients", "ephemeral", "ephemeral context"),
        ("BOOT_DETERMINISM_OPERATIONAL_ZETA Operational fact: Health check endpoint responds at /health within 50ms", "operational", "ops context 2"),
        ("BOOT_DETERMINISM_DURABLE_ETA Durable policy: API rate limits enforced at 100 requests per minute per IP", "durable", "C1 policy"),
    ];
    for (decision, retention, context) in seeds {
        store_decision(port, &token, decision, retention, context);
        thread::sleep(Duration::from_millis(10));
    }

    let first = fetch_boot(port, &token);
    let second = fetch_boot(port, &token);

    assert_eq!(first, second, "boot capsules must be byte-identical across two consecutive boots with no intervening writes\nfirst len {} second len {}\nfirst: {:?}\nsecond: {:?}", first.len(), second.len(), &first[..first.len().min(500)], &second[..second.len().min(500)]);

    store_decision(
        port,
        &token,
        "BOOT_DETERMINISM_NEW_THETA Post-boot decision: Added caching layer with Redis TTL 300 seconds operational",
        "operational",
        "new write context",
    );
    thread::sleep(Duration::from_millis(50));
    let third = fetch_boot(port, &token);
    assert_ne!(first, third, "capsule must change after an additional store (guards trivially-constant); first len {} third len {}", first.len(), third.len());
    assert!(
        third.contains("BOOT_DETERMINISM_NEW_THETA"),
        "third boot must contain newly stored decision, got {:?}",
        &third[..third.len().min(800)]
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
}
