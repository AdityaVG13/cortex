use cortex_kernel::handlers::store::{store_decision_with_ttl, DecisionProvenance};
use cortex_kernel::runtime::{deposit_decision, CortexRuntime, DepositInput, DepositOutcome, LensInput};
use cortex_tests::support::{run_with_cx, solo_state, test_conn};
use serde_json::Value;

async fn store(cx: &asupersync::Cx, runtime: &CortexRuntime, text: &str, context: &str) -> DepositOutcome {
    let mut conn = runtime.state().db.lock(cx).await.unwrap();
    let outcome = deposit_decision(&mut conn, DepositInput {
        request_id: text, idempotency_key: None, principal: "solo".into(), text,
        context: Some(context.into()), entry_type: Some("decision".into()), source_agent: "redaction-test".into(),
        provenance: DecisionProvenance::from_fields("redaction-test", None, None),
        confidence: Some(0.92), ttl_seconds: None, retention_class: None,
        anchors: vec![], fields: None, owner_id: None, benchmark: false,
    }).expect("deposit");
    assert!(outcome.target_id.is_some());
    assert_eq!(outcome.entry["action"], "inserted");
    assert_eq!(outcome.entry["status"], "active");
    outcome
}

async fn recall(cx: &asupersync::Cx, runtime: &CortexRuntime, query: &str) -> Value {
    runtime.lens(cx, LensInput { query: query.into(), budget: 300, k: 10,
        agent: "redaction-test".into(), ..Default::default() }).await.expect("lens")
}

// Listener/status/home-path assertions retired; persisted rows and recall are unconditional proofs.
#[test]
fn store_redacts_sk_and_ghp_to_redacted_exact() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let raw_sk = "sk-test1234567890abcdefFAKE";
        let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
        let raw = format!("Deployment pipeline uses {raw_sk} and {raw_ghp} for service auth in production handler");
        let expected = "Deployment pipeline uses [redacted] and [redacted] for service auth in production handler";
        let outcome = store(&cx, &runtime, &raw, "redaction secrets context check").await;
        let recalled = recall(&cx, &runtime, "Deployment pipeline service auth production handler").await;
        let hit = recalled["results"].as_array().unwrap().iter().find(|r| r["excerpt"] == expected).expect("exact redacted excerpt");
        assert_eq!(hit["excerpt"], expected);
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            let persisted: String = conn.query_row("SELECT decision FROM decisions WHERE id=?1", [outcome.target_id.unwrap()], |r| r.get(0)).unwrap();
            assert_eq!(persisted, expected);
            assert!(!persisted.contains(raw_sk) && !persisted.contains(raw_ghp));
        }
        let raw_recall = recall(&cx, &runtime, raw_sk).await;
        assert!(raw_recall["results"].as_array().unwrap().iter().all(|r| {
            let excerpt = r["excerpt"].as_str().unwrap();
            !excerpt.contains(raw_sk) && !excerpt.contains(raw_ghp)
        }), "FTS must not leak raw secrets");
    });
}

#[test]
fn store_preserves_benign_text_no_false_positive() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let benign = "Service handles risk-free tokens and mask-1234 in cache layer for user session store handler";
        let outcome = store(&cx, &runtime, benign, "benign redaction check").await;
        let recalled = recall(&cx, &runtime, "Service handles risk-free tokens mask-1234 cache layer").await;
        let hit = recalled["results"].as_array().unwrap().iter().find(|r| r["excerpt"] == benign).expect("exact benign excerpt");
        assert!(!hit["excerpt"].as_str().unwrap().contains("[redacted]"));
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let persisted: String = conn.query_row("SELECT decision FROM decisions WHERE id=?1", [outcome.target_id.unwrap()], |r| r.get(0)).unwrap();
        assert_eq!(persisted, benign);
    });
}

#[test]
fn store_redacts_context_field_exact() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
        let raw_context = format!("operator context with {raw_ghp} for deploy config store handler");
        let expected = "operator context with [redacted] for deploy config store handler";
        let decision = "Context redaction check for production deployment handler with sufficient specificity and tokens";
        let outcome = store(&cx, &runtime, decision, &raw_context).await;
        let recalled = recall(&cx, &runtime, "Context redaction production deployment handler").await;
        let hit = recalled["results"].as_array().unwrap().iter().find(|r| r["excerpt"] == decision).expect("decision recalled");
        assert_eq!(hit["source"], expected, "source provenance must preserve redacted context exactly");
        assert!(!hit["source"].as_str().unwrap().contains(raw_ghp));
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let persisted: String = conn.query_row("SELECT context FROM decisions WHERE id=?1", [outcome.target_id.unwrap()], |r| r.get(0)).unwrap();
        assert_eq!(persisted, expected);
        assert!(!persisted.contains(raw_ghp));
    });
}

#[test]
fn lib_store_redacts_before_graph_and_clock_projection() {
    run_with_cx(|_cx| async move {
        let mut conn = test_conn();
        let raw_sk = "sk-test1234567890abcdefFAKE";
        let raw_ghp = "ghp_abcdefghijklmnopqrstuvFAKE";
        let raw = format!("Deployment pipeline wires {raw_sk} service into production handler and syncs {raw_ghp} service with staging cache cluster");
        let expected = "Deployment pipeline wires [redacted] service into production handler and syncs [redacted] service with staging cache cluster";
        let (entry, id) = store_decision_with_ttl(&mut conn, &raw, Some("redaction projection context".into()), Some("decision".into()), "redaction-test".into(), Some(0.92), None, None).expect("lib store");
        assert_eq!(entry["action"], "inserted");
        let persisted: String = conn.query_row("SELECT decision FROM decisions WHERE id=?1", [id.unwrap()], |r| r.get(0)).unwrap();
        assert_eq!(persisted, expected);
        let queries = [
            "SELECT canonical_name FROM entities",
            "SELECT alias FROM entity_aliases",
            "SELECT value FROM clock_anchors",
            "SELECT COALESCE(display_value, '') FROM clock_anchors",
        ];
        for (index, sql) in queries.iter().enumerate() {
            let mut stmt = conn.prepare(sql).unwrap();
            let values: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
            for value in &values {
                for needle in [raw_sk, raw_ghp, "sktest1234567890abcdeffake", "ghpabcdefghijklmnopqrstuvfake"] {
                    assert!(!value.to_lowercase().contains(&needle.to_lowercase()), "{sql} leaked {needle}: {value}");
                }
            }
            if index < 3 {
                assert!(values.iter().any(|v| v == "[redacted] service"), "projection positive control {sql}: {values:?}");
            }
        }
    });
}
