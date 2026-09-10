//! Baseline fixtures and pinned counterexamples.
//!
//! Two roles:
//! 1. `baseline_six_arm_recall_matches_golden` freezes the current recall
//!    behavior on a fixed corpus (excerpts + admitting arms per query) so later
//!    refactors must state which changes are deliberate. Regenerate with
//!    `UPDATE_GOLDENS=1 cargo test -p cortex-tests --test baseline_fixtures`
//!    and review the diff; the golden also records the known-wrong outputs
//!    listed in `tests/golden/baseline/KNOWN_WRONG.md`.
//! 2. Two failure-first counterexamples that document defects the design
//!    corrects. They are expected to fail until their fixing beads land:
//!    - synthetic hop quorum (fixed by lineage-aware admission);
//!    - delta cursor withholding a durable decision from a fresh context
//!      (fixed by presence-safe orientation).

use cortex_daemon::compiler::compile;
use cortex_daemon::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::solo_state;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const AGENT: &str = "baseline-agent";

async fn store(cx: &asupersync::Cx, state: &cortex_daemon::state::RuntimeState, text: &str) -> i64 {
    let mut conn = state.db.lock(&cx).await.expect("lock");
    let (entry, id) = store_decision_with_ttl(
        &mut conn,
        text,
        None,
        Some("decision".into()),
        AGENT.into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("store {text:?}: {err}"));
    id.or_else(|| entry.get("id").and_then(|v| v.as_i64()))
        .expect("stored id")
}

async fn recall(
    cx: &asupersync::Cx,
    state: &cortex_daemon::state::RuntimeState,
    query: &str,
) -> Vec<Value> {
    let payload = execute_unified_recall(
        &cx,
        state,
        query,
        320,
        8,
        AGENT,
        &RecallContext::solo(),
        None,
    )
    .await
    .unwrap_or_else(|err| panic!("recall {query:?}: {err}"));
    payload["results"]
        .as_array()
        .unwrap_or_else(|| panic!("results missing: {payload}"))
        .clone()
}

fn excerpts(results: &[Value]) -> Vec<String> {
    results
        .iter()
        .filter_map(|r| r["excerpt"].as_str().map(str::to_string))
        .collect()
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("golden")
        .join(format!("{name}.golden"))
}

fn assert_golden(name: &str, actual: &str) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(path.parent().expect("golden parent")).expect("mkdir golden");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let expected = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "golden missing: {}\n{err}\nrun UPDATE_GOLDENS=1 cargo test -p cortex-tests --test baseline_fixtures",
            path.display()
        )
    });
    assert_eq!(
        expected.trim_end(),
        actual.trim_end(),
        "baseline drift for {name}; if deliberate, regenerate and explain in KNOWN_WRONG.md"
    );
}

/// Fixed corpus exercising every collector arm: lexical (quoted phrase),
/// anchor (path), truth (entity), task (explicit path context is not used
/// here on purpose so the golden stays adapter-neutral), history (as-of
/// cue) and hop (shared-anchor neighbor).
const CORPUS: &[&str] = &[
    "PAY-77 retry incident: never retry after the ledger commit succeeded in src/pay/retry.rs",
    "src/pay/retry.rs handles idempotent replay for the payments gateway",
    "Jake proposed the DB move",
    "the DB move is the Postgres migration",
    "billing invoices use Stripe Billing API in payments",
    "design review used stripe patterns in the UI kit",
];

const QUERIES: &[&str] = &[
    "PAY-77",
    "\"ledger commit\"",
    "src/pay/retry.rs",
    "what did Jake propose",
    "Stripe Billing API",
    "payments gateway as of 2020-01-01",
];

#[test]
fn baseline_six_arm_recall_matches_golden() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        for text in CORPUS {
            store(&cx, &state, text).await;
        }
        let mut lines = Vec::new();
        for query in QUERIES {
            let results = recall(&cx, &state, query).await;
            lines.push(format!("Q: {query}"));
            for r in &results {
                let arms: Vec<String> = r["why"]["clockVotes"]["admittedArms"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                lines.push(format!(
                    "  {} | admittedBy={} arms={} hops={}",
                    r["excerpt"].as_str().unwrap_or("?"),
                    r["why"]["admittedBy"].as_str().unwrap_or("?"),
                    arms.join("+"),
                    r["why"]["tieBreak"]["hops"].as_u64().unwrap_or(0)
                ));
            }
        }
        assert_golden("baseline/six_arm_recall", &lines.join("\n"));
    });
}

/// Counterexample: a row reachable only through one `clock_links` hop has no
/// direct witness for the query, yet the hop collector assigns it write=1 and
/// truth=1 and `admit()` accepts two nonzero clocks. One derived route must
/// count once toward relevance; it cannot mint two independent votes.
#[test]
fn counterexample_single_hop_is_not_a_quorum() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        // Texts share only the path anchor so the store-time Jaccard conflict
        // check classifies them UNRELATED and both rows persist separately.
        let seed = "PAY-77 outage postmortem filed under src/pay/retry.rs";
        let neighbor = "src/pay/retry.rs owns idempotent replay for gateway timeouts";
        store(&cx, &state, seed).await;
        store(&cx, &state, neighbor).await;
        let results = recall(&cx, &state, "PAY-77").await;
        let got = excerpts(&results);
        assert!(
            got.iter().any(|e| e == seed),
            "seed must be admitted, got {got:?}"
        );
        let admitted_neighbor = results
            .iter()
            .find(|r| r["excerpt"].as_str() == Some(neighbor));
        assert!(
            admitted_neighbor.is_none(),
            "hop-only neighbor must not be admitted by a synthetic two-clock quorum: {}",
            admitted_neighbor
                .map(|r| r["why"].to_string())
                .unwrap_or_default()
        );
    });
}

/// Counterexample: the boot delta cursor (`agent_boot` event) narrows the
/// second boot to "New decisions" since the last boot and suppresses the
/// "Recent decisions" section; TRUTH is recency-ranked top-N. A durable
/// constraint stored before the first boot therefore disappears from a fresh
/// context once enough unrelated activity follows, though nothing about it
/// changed. Delivery is not presence.
#[test]
fn counterexample_cursor_must_not_withhold_unchanged_decision() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let constraint = "never retry a payment request after the ledger commit succeeded";
        store(&cx, &state, constraint).await;
        asupersync::time::sleep(cx.now(), std::time::Duration::from_millis(5)).await;
        store(&cx, &state, "rename the staging bucket to cortex-stage").await;
        let first = {
            let conn = state.db.lock(&cx).await.expect("lock");
            compile(&conn, &state.home, AGENT, 4000)
        };
        assert!(
            first.boot_prompt.contains(constraint),
            "first boot must deliver the constraint: {}",
            first.boot_prompt
        );
        // Distinct sentences so store-time conflict classification keeps them
        // as separate rows (similar texts would be merged as REFINES).
        let later = [
            "switch the desktop palette to the high-contrast theme",
            "Kubernetes ingress uses the nginx controller for the docs site",
            "weekly backups run on Sunday at 03:00 UTC",
            "Grafana dashboards live under ops/monitoring",
            "Python SDK publishes wheels for arm64 only",
            "release notes are generated by cliff from conventional commits",
        ];
        for text in later {
            asupersync::time::sleep(cx.now(), std::time::Duration::from_millis(5)).await;
            store(&cx, &state, text).await;
        }
        let second = {
            let conn = state.db.lock(&cx).await.expect("lock");
            compile(&conn, &state.home, AGENT, 4000)
        };
        assert!(
    second.boot_prompt.contains(constraint),
    "a fresh context after the cursor advanced must still receive the unchanged constraint; got:\n{}",
    second.boot_prompt
);
    });
}
