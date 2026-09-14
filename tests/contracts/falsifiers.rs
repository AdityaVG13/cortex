//! The decisive falsifier suite (zero tolerance). Each falsifier either runs
//! here or names the contract that already holds it, so the release gate is
//! one file: `cargo test -p cortex-tests --test falsifiers` plus the listed
//! companions. A falsifier is a claim the system must be *unable* to make.

use cortex_logic::clockwork::{
    admit_with_lineage, independent_support, ClockEvidence, Rankable, Witness, WitnessDomain,
};
use cortex_kernel::db::feedback_ledger::adaptive_policy;
use cortex_kernel::db::promotion::{promote, Promotion, PromotionRule};
use cortex_kernel::db::records;
use cortex_logic::eval::accounting::Accounting;
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::support::{open_file_db, solo_state};
use serde_json::json;

/// Falsifiers held by companion contracts. The names are checked against
/// the test binaries at build time by being real paths in tests/Cargo.toml.
pub const COMPANIONS: &[(&str, &str)] = &[
    ("restore resurrects an erasure", "replication_erasure::erasure_reaches_derived_state_revokes_views_and_survives_restore"),
    ("new exception fails to invalidate", "recipes::new_exception_invalidates_without_touching_the_rule_and_unrelated_scope_preserves_reuse"),
    ("cursor withholds a constraint", "presence::change_cursor_carries_epoch_and_scope_and_requires_resnapshot_on_gaps"),
    ("as_known leaks later knowledge", "temporal::contradiction_closes_old_window_and_as_of_recovers_it"),
    ("archive roundtrip loses bytes", "retention::cold_codec_roundtrips_exact_bytes_including_unicode_and_binaryish_text"),
    ("false durable ack", "crash_durability::acked_stores_survive_sigkill + deposit_durability"),
    ("cross-scope leakage", "admin_acl + trust_boundaries::body_fields_never_select_a_principal_and_denials_disclose_nothing"),
    ("failing run remembered as verified", "threads (obligation verify requires checker pass) + host_events (failed test captured as failed)"),
    ("single path as two witnesses", "lineage::one_traversal_is_one_origin_however_many_counters_it_touched"),
    ("hidden cutoff labeled no_match", "store_conformance::sqlite_reference_is_certified (exact profile completeness) + lineage quota exhaustion"),
    ("cross-project majority overrides local condition", "promotion (typed rules, no global threshold)"),
];

fn rankable(hard: bool, ev: ClockEvidence) -> Rankable {
    Rankable {
        eligible: true,
        hard_anchor: hard,
        evidence: ev,
        strong_lexical: false,
    }
}

#[test]
fn f01_single_path_is_never_two_witnesses() {
    let ev = ClockEvidence {
        write: 1,
        truth: 1,
        task: 1,
        history: 0,
    };
    let one_path = vec![
        Witness::derived(WitnessDomain::Hop, "hop:decision::1", "same_path", 1),
        Witness::derived(WitnessDomain::Hop, "hop:decision::1", "observed_with", 2),
        Witness::derived(WitnessDomain::Entity, "hop:decision::1", "sibling", 1),
    ];
    assert_eq!(independent_support(&one_path), 1);
    assert_eq!(
        admit_with_lineage(rankable(false, ev), &one_path),
        None,
        "three counters from one traversal admit nothing"
    );
}

#[test]
fn f02_copied_claims_cannot_manufacture_quorum() {
    let ev = ClockEvidence {
        write: 2,
        truth: 2,
        task: 0,
        history: 0,
    };
    let copies = vec![
        Witness::derived(WitnessDomain::Lexical, "lexical:decision::7", "retry", 1),
        Witness::derived(WitnessDomain::Anchor, "lexical:decision::7", "PAY-1", 3),
    ];
    assert_eq!(admit_with_lineage(rankable(false, ev), &copies), None);
    let independent = vec![
        Witness::direct(WitnessDomain::Lexical, "lexical:decision::7", "retry", 1),
        Witness::direct(WitnessDomain::Anchor, "anchor:decision::7", "PAY-1", 2),
    ];
    assert_eq!(
        admit_with_lineage(rankable(false, ev), &independent),
        Some("clock_quorum")
    );
}

#[test]
fn f03_hidden_cutoff_is_never_labeled_no_match() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let caller = || Caller {
            owner_id: None,
            agent: "f03",
            principal: "solo".into(),
        };
        for i in 0..40 {
            let t = format!(
                "CUT-{i} distinct {} note about {} under {}",
                ["alpha", "bravo", "charlie", "delta"][i % 4],
                ["ledger", "cache", "auth", "queue", "search"][i % 5],
                ["dawn", "noon", "dusk", "night", "rain", "snow"][i % 6]
            );
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Commit,
                &json!({"decision": t}),
            )
            .await
            .unwrap();
        }
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CUT note", "budget": 300}),
        )
        .await
        .unwrap();
        let status = view["status"].as_str().unwrap_or("");
        let omissions = view["coverage"]["omissions"]
            .as_array()
            .map(|o| o.len())
            .unwrap_or(0);
        let unmet = view["coverage"]["unmet"]
            .as_array()
            .map(|o| o.len())
            .unwrap_or(0);
        if status == "no_match" {
            panic!("a tiny budget over a matching corpus produced no_match: {view}");
        }
        assert!(
            status == "needs_more_budget"
                || status == "partial"
                || omissions > 0
                || unmet > 0,
            "a 300-token query over 40 notes must not claim unlabeled complete ok: {view}"
        );
    });
}

#[test]
fn f04_cross_project_repetition_never_makes_a_global_preference() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let home = support_dir("f04");
        let conn = open_file_db(&home.join("cortex.db"));
        records::ensure_authoritative_schema(&conn).unwrap();
        let ack = "process_crash";
        let seq = records::append_commit(&conn, "solo", None, ack).unwrap();
        // Repetition across projects with no authority, no target scope and no
        // source cases is refused: seen-in-N-repos is not a global preference.
        let repetition = Promotion {
            rule: PromotionRule::CrossProjectLesson,
            principal: "solo",
            agent: "swarm",
            authority: None,
            sources: vec![],
            text: "always use tabs",
            preconditions: json!({"seen_in_repos": 5}),
            target_scope: None,
        };
        assert!(
            promote(&conn, seq, repetition).is_err(),
            "repetition is not authority"
        );
        let scoped_but_unauthorized = Promotion {
            rule: PromotionRule::CrossProjectLesson,
            principal: "solo",
            agent: "swarm",
            authority: None,
            sources: vec!["decision::1".into()],
            text: "always use tabs",
            preconditions: json!({}),
            target_scope: Some("org"),
        };
        assert!(
            promote(&conn, seq, scoped_but_unauthorized).is_err(),
            "a target scope without authority still fails"
        );
        let _ = std::fs::remove_dir_all(&home);
    });
}

#[test]
fn f05_failing_run_is_never_remembered_as_verified() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let payload = json!({"hook_event_name": "PostToolUse", "session_id": "f05", "tool_name": "Bash", "tool_use_id": "t", "tool_input": {"command": "cargo test -p x"}, "tool_response": {"stdout": "test result: FAILED. 3 passed; 2 failed", "exit_code": 101}});
        let frame = cortex_kernel::hook_event::frame_from_host(
            "PostToolUse",
            &payload,
            cortex_logic::adapter::CapabilityManifest::claude_code_plugin(),
        );
        let out = cortex_kernel::hook_event::process(&cx, &runtime, "f05", &frame, &payload)
            .await
            .expect("process host event");
        assert!(out.capture_receipt.is_some());
        let view = dispatch(
            &cx,
            runtime.state(),
            Caller {
                owner_id: None,
                agent: "f05",
                principal: "solo".into(),
            },
            Operation::Query,
            &json!({"need": "cargo test x", "profile": "attempts"}),
        )
        .await
        .unwrap();
        let text = view.to_string();
        assert!(text.contains("test failed"), "{view}");
        assert!(
            !text.contains("checker_verified"),
            "a failed run is never verified: {view}"
        );
    });
}

#[test]
fn f05_string_exit_code_still_captures_a_failed_command() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "session_id": "f05-str",
            "tool_name": "Bash",
            "tool_use_id": "t-str",
            "tool_input": {"command": "false"},
            "tool_response": {"stdout": "", "exit_code": "1"}
        });
        let frame = cortex_kernel::hook_event::frame_from_host(
            "PostToolUse",
            &payload,
            cortex_logic::adapter::CapabilityManifest::claude_code_plugin(),
        );
        let out = cortex_kernel::hook_event::process(&cx, &runtime, "f05-str", &frame, &payload)
            .await
            .expect("process host event");
        assert!(
            out.capture_receipt.is_some(),
            "string exit_code 1 must be material: {out:?}"
        );
    });
}

#[test]
fn f06_savings_are_never_summed_and_bandit_stays_off_without_benefit() {
    let a = Accounting {
        d_task_bytes: 10,
        d_read_bytes: 20,
        d_transport_bytes: 30,
        d_context_bytes: 40,
        d_price: Some(1.0),
        ..Default::default()
    };
    let r = a.report();
    assert!(r.get("total").is_none() && r["never_summed"] == true);
    assert!(
        !adaptive_policy((100, 100), (100, 100)).enabled,
        "retrieval popularity is not benefit"
    );
    assert!(!adaptive_policy((9, 10), (0, 10)).enabled, "under-sampled");
}

#[test]
fn f07_companion_falsifiers_are_named_and_point_at_existing_test_binaries() {
    let cargo =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    for (falsifier, holder) in COMPANIONS {
        let binary = holder
            .split("::")
            .next()
            .unwrap()
            .split(" + ")
            .next()
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        assert!(
            cargo.contains(&format!("name = \"{binary}\"")),
            "falsifier `{falsifier}` points at missing test binary `{binary}`"
        );
    }
}

fn support_dir(label: &str) -> std::path::PathBuf {
    tempfile::Builder::new()
        .prefix(&format!("cortex-fals-{label}-"))
        .tempdir()
        .expect("unique falsifier home")
        .keep()
}
