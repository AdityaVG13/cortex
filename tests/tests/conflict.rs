//! Conflict-detection contracts.
//!
//! Jaccard expected values come from the set formula, not from calling the
//! production function twice. `detect_conflict` is judged by classification
//! strings against stored decisions.
use cortex_daemon::conflict::{self, ConflictClassification};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::test_conn;
use std::collections::HashSet;

fn independent_jaccard(a: &str, b: &str) -> f64 {
    let tokens = |text: &str| -> HashSet<String> {
        text.split_whitespace()
            .filter(|word| word.len() > 1)
            .map(|word| word.to_lowercase())
            .collect()
    };
    let left = tokens(a);
    let right = tokens(b);
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f64;
    let union = (left.len() + right.len()) as f64 - intersection;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

#[test]
fn jaccard_matches_independent_set_formula() {
    let cases = [
        ("hello world test", "hello world test"),
        ("alpha beta gamma", "x y z"),
        ("the quick brown fox", "the slow brown dog"),
        ("Persist SQLITE wal checkpoints", "persist sqlite WAL checkpoints"),
        ("a", "a"),
        ("", ""),
    ];
    for (left, right) in cases {
        let got = conflict::jaccard_similarity(left, right);
        let expected = independent_jaccard(left, right);
        assert!(
            (got - expected).abs() < 1e-12,
            "jaccard({left:?}, {right:?}) = {got}, independent oracle {expected}"
        );
    }
}

fn store_specific(conn: &mut rusqlite::Connection, decision: &str, agent: &str) {
    let (entry, id) = store_decision_with_ttl(
        conn,
        decision,
        Some("conflict-oracle".into()),
        Some("decision".into()),
        agent.into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("store {decision:?}: {err}"));
    assert_eq!(entry["action"], "inserted", "store must insert {decision:?}, got {entry}");
    assert!(id.is_some(), "store must return an id for {decision:?}");
}

#[test]
fn detect_conflict_classifies_agree_contradict_unrelated() {
    let mut conn = test_conn();
    store_specific(
        &mut conn,
        "Always persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision",
        "oracle-agent",
    );

    let agrees = conflict::detect_conflict(
        &conn,
        "Always persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision",
        "oracle-agent",
        None,
    )
    .expect("detect identical");
    assert_eq!(agrees.classification, ConflictClassification::Agrees);
    assert!(!agrees.is_conflict, "AGREES is not a conflict");
    assert!((agrees.similarity_jaccard - 1.0).abs() < 1e-12);

    let contradicts = conflict::detect_conflict(
        &conn,
        "Never persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision",
        "other-agent",
        None,
    )
    .expect("detect contradiction");
    assert_eq!(contradicts.classification, ConflictClassification::Contradicts);
    assert!(contradicts.is_conflict, "CONTRADICTS must set is_conflict");
    assert_eq!(contradicts.classification.as_str(), "CONTRADICTS");

    let unrelated = conflict::detect_conflict(
        &conn,
        "Hybrid keyword plus semantic recall uses rrf fusion in handlers/recall/engine.rs",
        "oracle-agent",
        None,
    )
    .expect("detect unrelated");
    assert_eq!(unrelated.classification, ConflictClassification::Unrelated);
    assert!(!unrelated.is_conflict);
}
