//! Recall-positive eval suite (roadmap Phase 0): the mirror of
//! `falsifiers.rs`. A fixed corpus plus fixed queries with expected
//! admittances. Gated categories must hold on every run; the paraphrase gap
//! is measured and reported, not gated, until the use-grown bridge lands.
//! Run with `-- --nocapture` to see the per-category report.

use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_tests::support::solo_state;
use serde_json::{json, Value};

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "recall-positive",
        principal: "solo".into(),
    }
}

fn statements(view: &Value) -> Vec<String> {
    view["cards"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|c| c["statement"].as_str().map(str::to_string))
        .collect()
}

/// Fixed corpus. Order is stable; texts are short enough to survive
/// statement truncation verbatim.
async fn seed(cx: &asupersync::Cx, state: &cortex_kernel::state::RuntimeState) {
    for (i, text) in [
        "ledger commits are never retried after acknowledgement",
        "post-confirmation wire duplicates are prohibited",
        "THREAD-9 deploy freeze covers schema migrations on Fridays",
        "THREAD-9 hotfix lane bypasses the freeze with two approvals",
        "THREAD-9 freeze calendar is published quarterly by release crew",
        "office plants need watering every Tuesday",
        "cache TTL is 60 seconds",
        "cache TTL is 300 seconds",
    ]
    .iter()
    .enumerate()
    {
        let committed = dispatch(
            cx,
            state,
            caller(),
            Operation::Commit,
            &json!({"entries": [{"local_id": format!("rp{i}"), "text": text, "kind": "decision"}]}),
        )
        .await
        .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
    }
}

async fn ask(
    cx: &asupersync::Cx,
    state: &cortex_kernel::state::RuntimeState,
    need: &str,
) -> Value {
    dispatch(
        cx,
        state,
        caller(),
        Operation::Query,
        &json!({"need": need, "budget": 4000}),
    )
    .await
    .unwrap()
}

#[test]
fn r01_lexical_recall_admits_exact_match() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "are ledger commits retried after acknowledgement").await;
        let got = statements(&view);
        eprintln!("r01 lexical: {} cards", got.len());
        assert!(
            got.iter().any(|s| s.contains("never retried")),
            "exact wording must admit its doc: {view}"
        );
    });
}

#[test]
fn r02_update_chain_surfaces_current_value() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "what is the cache TTL").await;
        let got = statements(&view);
        eprintln!("r02 update-current: {got:?}");
        assert!(
            got.iter().any(|s| s.contains("300 seconds")),
            "the current value must be recalled: {view}"
        );
    });
}

#[test]
fn r03_thread_episodes_recall_as_a_group() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "THREAD-9 freeze policy").await;
        let got = statements(&view);
        let hits = got.iter().filter(|s| s.contains("THREAD-9")).count();
        eprintln!("r03 thread-group: {hits}/3 episodes admitted");
        assert_eq!(hits, 3, "all three THREAD-9 episodes must admit: {view}");
    });
}

#[test]
fn r04_true_unknown_abstains() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let view = ask(&cx, &state, "quasar polarity inversion protocols").await;
        let got = statements(&view);
        eprintln!(
            "r04 abstention: status={} cards={}",
            view["status"].as_str().unwrap_or("?"),
            got.len()
        );
        assert!(got.is_empty(), "a true unknown must admit nothing: {view}");
    });
}

#[test]
fn r05_paraphrase_gap_is_measured() {
    // The two ledger docs share no content words. Until the use-grown
    // bridge lands, cross-wording recall is expected to MISS; this test
    // pins the corpus well-formedness (each doc recalls under its own
    // wording) and reports the gap. When the bridge lands, the miss
    // report visibly flips and this test gets gated.
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        seed(&cx, &state).await;
        let own_a = ask(&cx, &state, "ledger commits retried acknowledgement").await;
        let own_b = ask(&cx, &state, "post-confirmation wire duplicates").await;
        assert!(
            statements(&own_a).iter().any(|s| s.contains("never retried")),
            "corpus doc A must recall under its own wording: {own_a}"
        );
        assert!(
            statements(&own_b)
                .iter()
                .any(|s| s.contains("duplicates are prohibited")),
            "corpus doc B must recall under its own wording: {own_b}"
        );
        let cross = ask(&cx, &state, "are ledger commits retried after acknowledgement").await;
        let bridged = statements(&cross)
            .iter()
            .any(|s| s.contains("duplicates are prohibited"));
        eprintln!("r05 paraphrase-gap: cross-wording bridge admitted={bridged} (want true)");
    });
}
