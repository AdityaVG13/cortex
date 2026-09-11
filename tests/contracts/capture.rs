//! Capture receipts and bounds: every capture reports accepted / redacted /
//! incomplete; redaction runs before projection on every write path
//! including /import; context and indexed files are bounded; the identity
//! capsule hash is O(rendered rows).

// HTTP import/admin status codes and the retired admin bounds-key aggregation
// are no longer surfaces; import redaction and budget validation remain local contracts.

use cortex_kernel::handlers::operations::{Caller, Operation, dispatch};
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::support::solo_state;
use serde_json::json;
use std::fs;

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "capture",
        principal: "solo".into(),
    }
}

#[test]
fn every_capture_returns_a_receipt_naming_what_was_retained() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let clean = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CAP-1 plain observation"}),
        )
        .await
        .unwrap();
        assert_eq!(
            clean["captures"][0]["capture"]["status"], "accepted",
            "{clean}"
        );
        assert_eq!(
            clean["captures"][0]["capture"]["offered_bytes"],
            "CAP-1 plain observation".len()
        );
        let secret = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CAP-2 token sk-test1234567890abcdefFAKE leaked"}),
        )
        .await
        .unwrap();
        assert_eq!(
            secret["captures"][0]["capture"]["status"], "redacted",
            "{secret}"
        );
        assert!(
            secret["captures"][0]["capture"]["retained_bytes"]
                .as_u64()
                .unwrap()
                != secret["captures"][0]["capture"]["offered_bytes"]
                    .as_u64()
                    .unwrap()
        );
        let long = "CAP-3 ".to_string() + &"x".repeat(5000);
        let truncated = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": long}),
        )
        .await
        .unwrap();
        assert_eq!(
            truncated["captures"][0]["capture"]["status"], "incomplete",
            "{truncated}"
        );
        assert!(
            truncated["captures"][0]["capture"]["reason"]
                .as_str()
                .unwrap()
                .contains("truncated")
        );
        // Context is bounded too.
        let runtime = CortexRuntime::from_state(state.clone());
        let _ = runtime;
        let big_context = "c".repeat(20_000);
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"entries": [{"text": "CAP-4 with big context", "context": big_context}]}),
        )
        .await
        .unwrap();
        let stored: i64 = state
            .db
            .lock(&cx)
            .await
            .expect("lock")
            .query_row(
                "SELECT length(context) FROM decisions WHERE decision = 'CAP-4 with big context'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(stored <= 8192 + 16, "context capped: {stored}");
    });
}

#[test]
fn import_redacts_like_store_and_reports_it() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let payload = serde_json::from_value(json!({"memories": [{"text": "IMP-1 key ghp_abcdefghijklmnopqrstuvFAKE here"}, {"text": "   "}], "decisions": [{"decision": "IMP-2 decision with sk-test1234567890abcdefFAKE", "context": "ctx sk-test1234567890abcdefFAKE"}]})).unwrap();
        {
            let mut conn = state.db.lock(&cx).await.expect("database lock");
            let counts = cortex_kernel::export_data::import_payload(
                &mut conn,
                &payload,
                &Default::default(),
            )
            .expect("import");
            assert_eq!(counts.redacted, 2);
            assert_eq!(counts.excluded, 1);
        }
        for (query, secret) in [
            ("IMP-1", "ghp_abcdefghijklmnopqrstuvFAKE"),
            ("IMP-2", "sk-test1234567890abcdefFAKE"),
        ] {
            let recalled = cortex_kernel::handlers::recall::execute_unified_recall(
                &cx,
                &state,
                query,
                400,
                8,
                "capture",
                &cortex_kernel::handlers::recall::RecallContext::solo(),
                None,
            )
            .await
            .expect("recall imported data");
            let text = recalled.to_string();
            assert!(
                !text.contains(secret),
                "imported secret must be redacted before FTS: {text}"
            );
        }
    });
}

#[test]
fn indexer_never_reads_more_than_the_ceiling() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::indexer::INDEXER_MAX_FILE_BYTES;
        use cortex_kernel::runtime::{CortexRuntime, observation::SourceSpec};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.log");
        fs::write(&path, vec![b'x'; INDEXER_MAX_FILE_BYTES as usize + 1]).unwrap();
        let runtime = CortexRuntime::open_db(&dir.path().join("brain.db")).unwrap();
        let key = format!("file:{}", path.canonicalize().unwrap().to_str().unwrap());
        let mut spec = SourceSpec::document(&key, "test");
        spec.max_bytes = 2 * 1024 * 1024;
        runtime.register_source(&cx, spec).await.unwrap();
        assert_eq!(
            runtime.observe_file(&cx, &path).await.unwrap_err(),
            "capture_byte_limit"
        );
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let stored: i64 = conn
            .query_row("SELECT count(*) FROM observation_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            stored, 0,
            "oversized source must not acknowledge a truncated prefix"
        );
    });
}

/// Budget validation remains a local contract after removal of the admin
/// routes and their aggregate bounds document.
#[test]
fn budget_config_rejects_unknown_endpoints_and_preserves_valid_bounds() {
    use cortex_logic::budgets::{BudgetConfig, BudgetEndpoint};
    let bad = BudgetConfig::parse_toml_str("[endpoints.nope]\nlimit = 1\n").unwrap_err();
    assert_eq!(bad.code, "unknown_endpoint");
    assert_eq!(bad.endpoint.as_deref(), Some("nope"));
    let empty = BudgetConfig::parse_toml_str("").expect("default config validates");
    assert!(empty.enabled);
    let valid = BudgetConfig::parse_toml_str("[endpoints.store]\nlimit = 4\nwindow_seconds = 30\n")
        .expect("valid bounded config");
    let store = valid
        .budget_for(BudgetEndpoint::Store)
        .expect("store budget");
    assert_eq!(store.limit, 4);
    assert_eq!(store.window_seconds, 30);
    assert!(
        BudgetConfig::parse_toml_str("[endpoints.store]\nlimit = 0\nwindow_seconds = 30\n")
            .is_err()
    );
}
