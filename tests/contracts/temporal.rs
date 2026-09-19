use cortex_kernel::handlers::store::DecisionProvenance;
use cortex_kernel::runtime::{deposit_decision, BootInput, CortexRuntime, DepositInput, DepositOutcome, LensInput};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};

const AGENT: &str = "temporal-agent";
const OLD_FACT: &str = "Always use Redis for caching in payments TEMPORALREDIS across all deployments";
const NEW_FACT: &str = "Never use Redis for caching in payments TEMPORALREDIS across all deployments";

async fn store(cx: &asupersync::Cx, runtime: &CortexRuntime, text: &str, confidence: f64) -> DepositOutcome {
    let mut conn = runtime.state().db.lock(cx).await.unwrap();
    deposit_decision(&mut conn, DepositInput {
        request_id: text, idempotency_key: None, principal: "solo".into(), text,
        context: None, entry_type: Some("decision".into()), source_agent: AGENT.into(),
        provenance: DecisionProvenance::from_fields(AGENT, Some("claude-opus"), None),
        confidence: Some(confidence), ttl_seconds: None, retention_class: None,
        anchors: vec![], paths: vec![], evidence: vec![], thread: None, fields: None, owner_id: None, benchmark: false,
    }).expect("deposit")
}

async fn recall(cx: &asupersync::Cx, runtime: &CortexRuntime, as_of: Option<String>) -> Value {
    runtime.lens(cx, LensInput { query: "TEMPORALREDIS Redis caching".into(), budget: 600,
        k: 10, agent: AGENT.into(), as_of, ..Default::default() }).await.expect("lens")
}

fn excerpts(body: &Value) -> Vec<&str> {
    body["results"].as_array().expect("results").iter().map(|r| r["excerpt"].as_str().expect("excerpt")).collect()
}

// URL encoding, status codes and dump transport retired; windows and persisted history remain.
#[test]
fn contradiction_closes_old_window_and_as_of_recovers_it() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let old = store(&cx, &runtime, OLD_FACT, 0.65).await;
        let old_id = old.target_id.expect("old id");
        let old_from = old.entry["validFrom"].as_str().unwrap().to_owned();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let new = store(&cx, &runtime, NEW_FACT, 0.99).await;
        let new_id = new.target_id.expect("new id");
        let new_from = new.entry["validFrom"].as_str().unwrap().to_owned();
        assert_ne!(old_from, new_from);
        assert_eq!(new.entry["classification"], "CONTRADICTS");
        assert_eq!(new.entry["supersedes"], old_id);
        let current = recall(&cx, &runtime, None).await;
        assert!(excerpts(&current).contains(&NEW_FACT));
        assert!(!excerpts(&current).contains(&OLD_FACT));
        let historical = recall(&cx, &runtime, Some(old_from.clone())).await;
        assert!(excerpts(&historical).contains(&OLD_FACT));
        assert!(!excerpts(&historical).contains(&NEW_FACT));
        let old_result = historical["results"].as_array().unwrap().iter().find(|r| r["excerpt"] == OLD_FACT).unwrap();
        assert_eq!(old_result["validUntil"], json!(new_from));
        assert_eq!(old_result["status"], "superseded");
        let boundary = recall(&cx, &runtime, Some(new_from.clone())).await;
        assert!(excerpts(&boundary).contains(&NEW_FACT));
        assert!(!excerpts(&boundary).contains(&OLD_FACT), "half-open windows");
        let boot = runtime.boot(&cx, BootInput { agent: AGENT.into(), max_tokens: 600, owner_id: None, ..Default::default() }).await.unwrap();
        assert!(boot.boot_prompt.lines().any(|l| l == "## TRUTH"));
        let expected = format!("FACT? {NEW_FACT}  (valid {} → now)  [d{new_id}]", &new_from[..10]);
        assert!(boot.boot_prompt.lines().any(|l| l == expected), "{}", boot.boot_prompt);
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let old_row: (String, Option<String>) = conn.query_row("SELECT status, valid_until FROM decisions WHERE id=?1", [old_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(old_row, ("superseded".into(), Some(new_from.clone())));
        let new_row: (String, String, Option<String>) = conn.query_row("SELECT status, valid_from, valid_until FROM decisions WHERE id=?1", [new_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(new_row, ("active".into(), new_from, None));
    });
}
