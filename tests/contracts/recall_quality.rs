//! Recall quality harness (cortex-qjpd.3).
//!
//! Deterministic View fixtures for admit / abstain / as-of / conflict /
//! paraphrase-lite. Pass-fail situational accuracy only — no token claims,
//! no golden snapshots of whatever the engine currently emits.
//!
//! Empty-agree tautology: an abstain fixture is not a pass just because both
//! sides are empty. The same corpus must still admit a positive-control handle.

use cortex_daemon::handlers::operations::{dispatch, Caller, Operation};
use cortex_daemon::state::RuntimeState;
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
