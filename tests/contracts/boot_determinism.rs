use cortex_kernel::handlers::store::DecisionProvenance;
use cortex_kernel::runtime::{deposit_decision, BootInput, CortexRuntime, DepositInput};
use cortex_tests::support::{run_with_cx, solo_state};
use std::thread;
use std::time::Duration;

async fn store_decision(runtime: &CortexRuntime, cx: &asupersync::Cx, decision: &str, retention: &str, context: &str) {
    let mut conn = runtime.state().db.lock(cx).await.unwrap();
    let outcome = deposit_decision(&mut conn, DepositInput {
        request_id: decision, idempotency_key: None, principal: "solo".into(), text: decision,
        context: Some(context.into()), entry_type: Some("decision".into()),
        source_agent: "boot-determinism-agent".into(),
        provenance: DecisionProvenance::from_fields("boot-determinism-agent", None, None),
        confidence: None, ttl_seconds: None,
        retention_class: Some(serde_json::from_value(serde_json::json!(retention)).expect("retention")),
        anchors: vec![], paths: vec![], evidence: vec![], thread: None, fields: None, owner_id: None, benchmark: false,
    }).expect("deposit");
    assert!(outcome.target_id.is_some());
}

async fn fetch_boot(runtime: &CortexRuntime, cx: &asupersync::Cx) -> String {
    let boot = runtime.boot(cx, BootInput { agent: "boot-determinism-agent".into(), max_tokens: 600, owner_id: None, ..Default::default() }).await.expect("boot");
    assert!(!boot.boot_prompt.is_empty());
    assert!(boot.token_estimate > 0);
    assert!(!boot.capsules.is_empty());
    assert!(boot.savings.is_object());
    boot.boot_prompt
}

// HTTP profile envelope/status checks retired; typed compiler output is exercised.
#[test]
fn boot_is_byte_identical_across_consecutive_calls_and_changes_after_write() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
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
            store_decision(&runtime, &cx, decision, retention, context).await;
            thread::sleep(Duration::from_millis(10));
        }
        let first = fetch_boot(&runtime, &cx).await;
        let second = fetch_boot(&runtime, &cx).await;
        assert_eq!(first.as_bytes(), second.as_bytes(), "boots without intervening writes must be byte-identical");
        store_decision(&runtime, &cx,
            "BOOT_DETERMINISM_NEW_THETA Post-boot decision: Added caching layer with Redis TTL 300 seconds operational",
            "operational", "new write context").await;
        thread::sleep(Duration::from_millis(50));
        let third = fetch_boot(&runtime, &cx).await;
        assert_ne!(first, third, "additional write must change capsule");
        assert!(third.contains("BOOT_DETERMINISM_NEW_THETA"), "{third}");
    });
}
