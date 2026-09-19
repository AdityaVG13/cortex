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
fn import_preserves_policy_kind() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let payload = serde_json::from_value(json!({
            "decisions": [{
                "decision": "Always require TLS 1.3 on the imported payments webhook in src/payments/webhook.rs",
                "type": "policy"
            }]
        }))
        .unwrap();
        let mut conn = state.db.lock(&cx).await.expect("database lock");
        cortex_kernel::export_data::import_payload(&mut conn, &payload, &Default::default())
            .expect("import");
        let kind: String = conn
            .query_row(
                "SELECT type FROM decisions WHERE decision LIKE 'Always require TLS 1.3%'",
                [],
                |r| r.get(0),
            )
            .expect("imported policy");
        assert_eq!(
            kind, "policy",
            "import must not collapse policy into decision"
        );
    });
}

#[test]
fn import_empty_temporal_fields_are_unbounded_not_present() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let payload = serde_json::from_value(json!({
            "memories": [{
                "text": "EDGE-EMPTY-TEMPORAL imported fact remains recallable",
                "observed_at": "",
                "valid_from": "  ",
                "valid_until": ""
            }]
        }))
        .unwrap();
        {
            let mut conn = state.db.lock(&cx).await.expect("database lock");
            cortex_kernel::export_data::import_payload(
                &mut conn,
                &payload,
                &Default::default(),
            )
            .expect("import");
            let (observed, valid_from, valid_until): (Option<String>, Option<String>, Option<String>) =
                conn.query_row(
                    "SELECT observed_at, valid_from, valid_until FROM memories WHERE text LIKE 'EDGE-EMPTY-TEMPORAL%'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .expect("imported row");
            assert!(
                observed.as_deref().is_some_and(|v| !v.trim().is_empty()),
                "empty observed_at must fall back to now, not store '': {observed:?}"
            );
            assert!(
                valid_from.as_deref().is_some_and(|v| !v.trim().is_empty()),
                "empty valid_from must fall back to now, not store '': {valid_from:?}"
            );
            assert_eq!(
                valid_until, None,
                "empty valid_until must bind NULL (unbounded), not '': {valid_until:?}"
            );
        }
        let recalled = cortex_kernel::handlers::recall::execute_unified_recall(
            &cx,
            &state,
            "EDGE-EMPTY-TEMPORAL",
            400,
            8,
            "capture",
            &cortex_kernel::handlers::recall::RecallContext::solo(),
            None,
        )
        .await
        .expect("recall imported empty-temporal row");
        let text = recalled.to_string();
        assert!(
            text.contains("EDGE-EMPTY-TEMPORAL"),
            "empty valid_until must not hide the imported row: {text}"
        );
        let mut as_of_ctx = cortex_kernel::handlers::recall::RecallContext::solo();
        as_of_ctx.as_of = Some(String::new());
        let as_of_recall = cortex_kernel::handlers::recall::execute_unified_recall(
            &cx,
            &state,
            "EDGE-EMPTY-TEMPORAL",
            400,
            8,
            "capture",
            &as_of_ctx,
            None,
        )
        .await
        .expect("recall with empty as_of");
        let as_of_text = as_of_recall.to_string();
        assert!(
            as_of_text.contains("EDGE-EMPTY-TEMPORAL"),
            "empty as_of must not be treated as a present bound: {as_of_text}"
        );
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

#[test]
fn cargo_test_nonzero_exit_is_not_recorded_as_passed() {
    use cortex_logic::capture::{parse_tool_result, CheckKind};
    // First crate printed a clean summary; a later crate failed to compile.
    // Parsed "0 failed" must not override the process exit.
    let out = "     Running unittests src/lib.rs (target/debug/deps/foo)\n\
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n\
error: could not compile `bar` (lib) due to 1 previous error\n";
    let facts = parse_tool_result("Bash", Some("cargo test --workspace"), out, Some(101));
    let test = facts
        .checks
        .iter()
        .find(|c| c.kind == CheckKind::Test)
        .expect("test check");
    assert!(
        !test.passed,
        "a later crate compile failure must not be stored as a passing test: {test:?}"
    );
    assert_eq!(test.passed_count, Some(5));
    assert_eq!(test.failed_count, Some(0));
}

#[test]
fn cargo_test_output_yields_a_typed_check_with_counts() {
    use cortex_logic::capture::{parse_tool_result, CheckKind, TypedCheck};
    let out = "running 3 tests\ntest a ... ok\ntest result: FAILED. 2 passed; 1 failed; 0 ignored\n  --> crates/logic/src/lens.rs:41:9";
    let f = parse_tool_result("Bash", Some("cargo test -p cortex-logic"), out, Some(101));
    assert_eq!(
        f.checks,
        vec![TypedCheck {
            kind: CheckKind::Test,
            passed: false,
            passed_count: Some(2),
            failed_count: Some(1)
        }]
    );
    assert_eq!(f.paths, vec!["crates/logic/src/lens.rs"]);
    assert!(f.is_material());
    assert!(f.statement().contains("exit 101"));
    assert!(
        f.statement().contains("test failed (2 passed, 1 failed)"),
        "{}",
        f.statement()
    );
    assert_eq!(
        f.idempotency_key(),
        parse_tool_result("Bash", Some("cargo test -p cortex-logic"), out, Some(101))
            .idempotency_key()
    );
}

#[test]
fn symbols_error_codes_and_non_material_results() {
    use cortex_logic::capture::{parse_tool_result, CheckKind};
    let f = parse_tool_result(
        "Bash",
        Some("cargo check"),
        "error[E0277]: the trait bound ... in crate::store_spi::sqlite::SqliteStore\n --> src/a.rs:3:1",
        Some(1),
    );
    assert_eq!(f.error_codes, vec!["E0277"]);
    assert!(
        f.symbols.iter().any(|s| s.contains("SqliteStore")),
        "{:?}",
        f.symbols
    );
    assert_eq!(f.checks[0].kind, CheckKind::Typecheck);
    assert!(!f.checks[0].passed);
    let idle = parse_tool_result("Read", None, "just some prose without anything", None);
    assert!(!idle.is_material(), "{idle:?}");
    let explained = parse_tool_result(
        "Bash",
        Some("ls"),
        "because the cache was cold the build was slow",
        Some(0),
    );
    assert!(
        !explained.statement().contains("because"),
        "explanations are never captured as facts"
    );
    let cmake = parse_tool_result("Bash", Some("cmake -S . -B build"), "Configuring done", Some(0));
    assert!(
        cmake.checks.iter().all(|c| c.kind != CheckKind::Build),
        "cmake is not a make invocation: {:?}",
        cmake.checks
    );
    let make = parse_tool_result("Bash", Some("make -j4"), "error: *** missing separator", Some(2));
    assert!(
        make.checks.iter().any(|c| c.kind == CheckKind::Build && !c.passed),
        "{:?}",
        make.checks
    );
    let tsconfig = parse_tool_result(
        "Bash",
        Some("cat tsconfig.json"),
        "{ \"compilerOptions\": {} }",
        Some(0),
    );
    assert!(
        tsconfig.checks.iter().all(|c| c.kind != CheckKind::Typecheck),
        "tsconfig is not a tsc invocation: {:?}",
        tsconfig.checks
    );
    let tsc = parse_tool_result(
        "Bash",
        Some("npx tsc --noEmit"),
        "error TS2304: Cannot find name 'x'.",
        Some(1),
    );
    assert!(
        tsc.checks.iter().any(|c| c.kind == CheckKind::Typecheck && !c.passed),
        "{:?}",
        tsc.checks
    );
    let vue_tsc = parse_tool_result("Bash", Some("npx vue-tsc --noEmit"), "Found 0 errors", Some(0));
    assert!(
        vue_tsc
            .checks
            .iter()
            .any(|c| c.kind == CheckKind::Typecheck && c.passed),
        "vue-tsc is a tsc wrapper: {:?}",
        vue_tsc.checks
    );
}

