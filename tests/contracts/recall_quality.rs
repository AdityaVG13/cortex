//! Recall quality harness (cortex-qjpd.3).
//!
//! Deterministic View fixtures for admit / abstain / as-of / conflict /
//! paraphrase-lite. Pass-fail situational accuracy only — no token claims,
//! no golden snapshots of whatever the engine currently emits.
//!
//! Empty-agree tautology: an abstain fixture is not a pass just because both
//! sides are empty. The same corpus must still admit a positive-control handle.

use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::runtime::{BootInput, CortexRuntime, LensInput};
use cortex_kernel::state::RuntimeState;
use cortex_tests::support::solo_state;
use serde_json::{json, Value};

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "rq-harness",
        principal: "solo".into(),
    }
}

async fn commit(cx: &asupersync::Cx, state: &RuntimeState, text: &str) -> Value {
    commit_with(cx, state, text, None).await
}

async fn commit_with(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    text: &str,
    confidence: Option<f64>,
) -> Value {
    let mut body = json!({"decision": text});
    if let Some(c) = confidence {
        body["confidence"] = json!(c);
    }
    let r = dispatch(cx, state, caller(), Operation::Commit, &body)
        .await
        .unwrap();
    assert_eq!(r["status"], "ok", "commit {text:?}: {r}");
    r
}

async fn query(cx: &asupersync::Cx, state: &RuntimeState, args: Value) -> Value {
    dispatch(cx, state, caller(), Operation::Query, &args)
        .await
        .unwrap()
}

fn statements(view: &Value) -> Vec<String> {
    view["cards"]
        .as_array()
        .unwrap_or_else(|| panic!("cards missing: {view}"))
        .iter()
        .filter_map(|c| c["statement"].as_str().map(str::to_string))
        .collect()
}

fn has_statement(view: &Value, needle: &str) -> bool {
    statements(view).iter().any(|s| s.contains(needle))
}

fn cards_matching<'a>(view: &'a Value, needle: &str) -> Vec<&'a Value> {
    view["cards"]
        .as_array()
        .unwrap_or_else(|| panic!("cards missing: {view}"))
        .iter()
        .filter(|c| {
            c["statement"]
                .as_str()
                .is_some_and(|s| s.contains(needle))
        })
        .collect()
}

fn decision_id(committed: &Value) -> i64 {
    let entries = &committed["receipt"]["entries"];
    for key in ["decision.decision", "entry.decision"] {
        if let Some(id) = entries[key]["value"]
            .as_str()
            .and_then(|s| s.parse().ok())
        {
            return id;
        }
    }
    panic!("decision id missing: {committed}");
}

/// A situation that expects emptiness is only valid when a control query on
/// the same corpus still admits. Two empties agreeing is not evidence.
fn reject_empty_agree(label: &str, situation_empty: bool, control_hit: bool, detail: &Value) {
    assert!(
        control_hit,
        "{label}: empty-agree tautology — control handle must admit on the same corpus: {detail}"
    );
    assert!(
        situation_empty,
        "{label}: situation must abstain, not fabricate a hit: {detail}"
    );
}

const ADMIT: &str = "RQ-ADMIT-77 never retry after the ledger commit succeeded in src/pay/retry.rs";
const DISTRACTOR: &str = "The office snack policy prefers salted almonds on Fridays";
const MORPH: &str = "Redis is our cache layer in payments RQ-MORPH-15";
const ASOF_OLD: &str = "Always use Redis for caching in payments RQ-ASOF across all deployments";
const ASOF_NEW: &str = "Never use Redis for caching in payments RQ-ASOF across all deployments";
const CONFLICT_A: &str = "RQ-CF-9 deployment default is blue";
const CONFLICT_B: &str = "RQ-CF-9 deployment default is green";

#[test]
fn admit_hard_handle_is_served() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        commit(&cx, &state, ADMIT).await;
        commit(&cx, &state, DISTRACTOR).await;
        let view = query(
            &cx,
            &state,
            json!({"need": "RQ-ADMIT-77", "profile": "answer", "budget": 4000}),
        )
        .await;
        assert!(
            has_statement(&view, "never retry after the ledger commit"),
            "hard ticket must admit the stored rule: {view}"
        );
        assert!(
            !has_statement(&view, "salted almonds"),
            "unrelated distractor must not ride the handle: {view}"
        );
    });
}

#[test]
fn abstain_unconstrained_paraphrase_is_empty_and_control_still_hits() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        commit(&cx, &state, ADMIT).await;
        commit(&cx, &state, DISTRACTOR).await;
        let abstain = query(
            &cx,
            &state,
            json!({
                "need": "how should we authenticate the payments webhook",
                "profile": "answer",
                "budget": 4000
            }),
        )
        .await;
        let control = query(
            &cx,
            &state,
            json!({"need": "RQ-ADMIT-77", "profile": "answer", "budget": 4000}),
        )
        .await;
        let fabricated = has_statement(&abstain, "salted almonds")
            || has_statement(&abstain, "never retry after the ledger commit");
        reject_empty_agree(
            "abstain",
            !fabricated && statements(&abstain).is_empty(),
            has_statement(&control, "never retry after the ledger commit"),
            &json!({"abstain": abstain, "control": control}),
        );
    });
}

#[test]
fn as_of_recovers_closed_fact_and_current_does_not_leak_it() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let old = commit_with(&cx, &state, ASOF_OLD, Some(0.65)).await;
        let old_id = decision_id(&old);
        let old_from: String = {
            let conn = state.db.lock(&cx).await.expect("lock");
            conn.query_row(
                "SELECT valid_from FROM decisions WHERE id = ?1",
                [old_id],
                |r| r.get(0),
            )
            .expect("old valid_from")
        };
        asupersync::time::sleep(cx.now(), std::time::Duration::from_millis(20)).await;
        let new = commit_with(&cx, &state, ASOF_NEW, Some(0.99)).await;
        let class = new["legacy_entries"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|e| e["classification"].as_str());
        assert_eq!(
            class,
            Some("CONTRADICTS"),
            "replacement must close the old window: {new}"
        );
        let current = query(
            &cx,
            &state,
            json!({"need": "RQ-ASOF Redis caching", "profile": "answer", "budget": 4000}),
        )
        .await;
        assert!(
            has_statement(&current, "Never use Redis"),
            "current must serve the replacement: {current}"
        );
        assert!(
            !has_statement(&current, "Always use Redis"),
            "current must not leak the closed fact: {current}"
        );
        let historical = query(
            &cx,
            &state,
            json!({
                "need": "RQ-ASOF Redis caching",
                "profile": "answer",
                "budget": 4000,
                "as_of": old_from
            }),
        )
        .await;
        assert!(
            has_statement(&historical, "Always use Redis"),
            "as-of must recover the closed fact: {historical}"
        );
        assert!(
            !has_statement(&historical, "Never use Redis"),
            "as-of must not leak later knowledge: {historical}"
        );
    });
}

#[test]
fn conflict_open_contradicts_is_never_asserted() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let a = commit(&cx, &state, CONFLICT_A).await;
        let b = commit(&cx, &state, CONFLICT_B).await;
        let a_id = decision_id(&a);
        let b_id = decision_id(&b);
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            conn.execute(
                "INSERT INTO decision_conflicts (source_decision_id, target_decision_id, classification, similarity_jaccard, status) VALUES (?1, ?2, 'CONTRADICTS', 0.9, 'open')",
                [a_id, b_id],
            )
            .unwrap();
        }
        let view = query(
            &cx,
            &state,
            json!({"need": "RQ-CF-9 deployment default", "profile": "answer", "budget": 8000}),
        )
        .await;
        let hits = cards_matching(&view, "RQ-CF-9");
        assert!(
            !hits.is_empty(),
            "conflict fixture must surface the contested heads: {view}"
        );
        for card in hits {
            assert_eq!(
                card["epistemic"], "contested",
                "open CONTRADICTS head must not be asserted: {view}"
            );
            assert!(
                card["exceptions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e.as_str().unwrap().contains("contrary head")),
                "contrary side must travel: {view}"
            );
        }
    });
}

#[test]
fn paraphrase_lite_morphology_admits_shared_stem() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        commit(&cx, &state, MORPH).await;
        commit(&cx, &state, DISTRACTOR).await;
        let view = query(
            &cx,
            &state,
            json!({
                "need": "what do we use for caching RQ-MORPH-15",
                "profile": "answer",
                "budget": 4000
            }),
        )
        .await;
        assert!(
            has_statement(&view, "cache layer in payments RQ-MORPH-15"),
            "cache↔caching must admit the stored fact: {view}"
        );
        assert!(
            !has_statement(&view, "salted almonds"),
            "morph admit must not drag in the distractor: {view}"
        );
    });
}

const SCOPE_A: &str =
    "Payment ledger retries must stay idempotent after a crash in the write path";
const SCOPE_B: &str = "Cache warming must run before the first user request on boot";
const SCOPE_GLOBAL: &str = "Team signing keys rotate every quarter without exception";
const REPO_A: &str = "/Users/x/repoa";
const REPO_B: &str = "/Users/x/repob";

#[test]
fn scoped_commit_admits_in_repo_and_not_in_sibling_repo() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let a = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": SCOPE_A, "paths": [REPO_A]}),
        )
        .await
        .unwrap();
        assert_eq!(a["status"], "ok", "{a}");
        let b = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": SCOPE_B, "paths": [REPO_B]}),
        )
        .await
        .unwrap();
        assert_eq!(b["status"], "ok", "{b}");

        let in_a = query(
            &cx,
            &state,
            json!({"need": "Payment ledger retries", "profile": "answer", "budget": 4000, "paths": [REPO_A]}),
        )
        .await;
        assert!(
            has_statement(&in_a, "Payment ledger retries must stay idempotent"),
            "same-repo query must admit the scoped fact: {in_a}"
        );
        assert!(
            !has_statement(&in_a, "Cache warming must run"),
            "sibling repo fact must not admit: {in_a}"
        );

        let in_b = query(
            &cx,
            &state,
            json!({"need": "Payment ledger retries", "profile": "answer", "budget": 4000, "paths": [REPO_B]}),
        )
        .await;
        assert!(
            !has_statement(&in_b, "Payment ledger retries must stay idempotent"),
            "foreign cwd must not retrieve this project's fact: {in_b}"
        );

        let orient = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Orient,
            &json!({"paths": [REPO_A], "budget": 4000}),
        )
        .await
        .unwrap();
        assert!(
            has_statement(&orient, "Payment ledger retries must stay idempotent"),
            "cwd-only orient must retrieve this repo's fact: {orient}"
        );
        assert!(
            !has_statement(&orient, "Cache warming must run"),
            "cwd-only orient must not retrieve a sibling repo: {orient}"
        );
    });
}

#[test]
fn unscoped_fact_survives_a_project_path_query() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": SCOPE_GLOBAL}),
        )
        .await
        .unwrap();
        let view = query(
            &cx,
            &state,
            json!({"need": "Team signing keys rotate", "profile": "answer", "budget": 4000, "paths": [REPO_B]}),
        )
        .await;
        assert!(
            has_statement(&view, "Team signing keys rotate"),
            "a fact with no stored root stays eligible when the caller names a project: {view}"
        );
    });
}

#[test]
fn runtime_scope_on_write_reaches_lens_and_boot() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let a_paths = vec![REPO_A.to_string()];
        let b_paths = vec![REPO_B.to_string()];
        runtime
            .deposit_with_scope(
                &cx,
                "scope-a",
                None,
                SCOPE_A,
                "rq-harness",
                None,
                &a_paths,
                Some("session:alpha"),
            )
            .await
            .unwrap();
        runtime
            .deposit_with_scope(
                &cx,
                "scope-b",
                None,
                SCOPE_B,
                "rq-harness",
                None,
                &b_paths,
                None,
            )
            .await
            .unwrap();

        let view = runtime
            .lens(
                &cx,
                LensInput {
                    query: "Payment ledger retries".into(),
                    budget: 4000,
                    k: 8,
                    agent: "rq-harness".into(),
                    paths: a_paths.clone(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let excerpts: Vec<&str> = view["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["excerpt"].as_str())
            .collect();
        assert!(
            excerpts.iter().any(|e| e.contains("Payment ledger retries")),
            "scoped lens must admit: {view}"
        );
        assert!(
            excerpts.iter().all(|e| !e.contains("Cache warming must run")),
            "scoped lens must drop the sibling: {view}"
        );

        {
            let conn = runtime.state().db_read.lock(&cx).await.unwrap();
            let sessions: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM clock_anchors a
                     JOIN clock_anchor_evidence e ON e.anchor_id = a.id
                     WHERE a.kind = 'session' AND a.value = 'session:alpha'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(sessions, 1, "thread must be projected as a session anchor");
        }

        let boot = runtime
            .boot(
                &cx,
                BootInput {
                    agent: "reader".into(),
                    max_tokens: 2000,
                    paths: a_paths,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            boot.boot_prompt.contains("Payment ledger retries"),
            "scoped boot must pack this repo: {}",
            boot.boot_prompt
        );
        assert!(
            !boot.boot_prompt.contains("Cache warming must run"),
            "scoped boot must omit the sibling repo: {}",
            boot.boot_prompt
        );
    });
}

#[test]
fn boot_does_not_cut_a_constraint_headline_away_from_its_qualifier() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let state = runtime.state();
        let rule = "BAR-BOOT-1 always enable TLS on the payments listener and never skip certificate pinning even in long-named local development environments with extra hostnames";
        let exception = "BAR-BOOT-1 exception: localhost dev listener may stay plaintext";
        let first = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": rule, "retention_class": "durable", "type": "constraint"}),
        )
        .await
        .unwrap();
        assert_eq!(first["status"], "ok", "{first}");
        let second = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": exception, "retention_class": "durable", "type": "constraint"}),
        )
        .await
        .unwrap();
        assert_eq!(second["status"], "ok", "{second}");
        {
            use cortex_kernel::db::records::{append_commit, heads};
            use cortex_kernel::handlers::operations::{add_relation, DependencyRole};
            let rule_id = decision_id(&first);
            let exception_id = decision_id(&second);
            let conn = state.db.lock(&cx).await.expect("lock");
            let rule_head = heads(&conn, &format!("decision:{rule_id}")).unwrap().remove(0);
            let exc_head = heads(&conn, &format!("decision:{exception_id}"))
                .unwrap()
                .remove(0);
            let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
            add_relation(
                &conn,
                seq,
                &rule_head,
                &exc_head,
                DependencyRole::RequiredQualifier.as_str(),
                Some("closure/1"),
            )
            .unwrap();
        }
        let full = runtime
            .boot(
                &cx,
                BootInput {
                    agent: "reader".into(),
                    max_tokens: 2000,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            full.boot_prompt.contains("always enable TLS")
                && full.boot_prompt.contains("localhost dev listener"),
            "full boot must pack the constraint and its qualifier: {}",
            full.boot_prompt
        );
        let packed = cortex_kernel::compiler::pack_context_items_greedy(
            std::slice::from_ref(&cortex_kernel::compiler::ContextItem::new(
                "## Constraints",
                format!(
                    "## Constraints\n- [constraint d1] {rule}\n- [constraint d2] {exception}"
                ),
                0.99,
            )),
            40,
        );
        let assembled = packed.assembled_parts.join("\n\n");
        assert!(
            packed.admitted.iter().all(|item| item["truncated"] != true),
            "constraint capsule must not be cut mid-statement: {assembled:?} admitted={:?}",
            packed.admitted
        );
        assert!(
            !assembled.contains("always enable TLS") || assembled.contains("localhost dev listener"),
            "greedy packing must not serve the headline without its qualifier: {assembled:?}"
        );
    });
}
