use cortex_logic::conflict::{self, ConflictClassification};
use cortex_kernel::handlers::store::store_decision_with_ttl;
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
        (
            "Persist SQLITE wal checkpoints",
            "persist sqlite WAL checkpoints",
        ),
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
    assert_eq!(
        entry["action"], "inserted",
        "store must insert {decision:?}, got {entry}"
    );
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
    assert_eq!(
        contradicts.classification,
        ConflictClassification::Contradicts
    );
    let contracted = conflict::detect_conflict(
        &conn,
        "Don't persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision",
        "other-agent",
        None,
    )
    .expect("detect contracted contradiction");
    assert_eq!(
        contracted.classification,
        ConflictClassification::Contradicts,
        "contracted negation must contradict like 'never'"
    );
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

#[test]
fn typed_policy_is_a_separate_observation_not_a_jaccard_collapse() {
    let mut conn = test_conn();
    let text = "Always require TLS 1.3 on the payments webhook in src/payments/webhook.rs including retry headers";
    let (first, id1) = store_decision_with_ttl(
        &mut conn,
        text,
        Some("typed-kind-oracle".into()),
        Some("policy".into()),
        "oracle-agent".into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("first policy store: {err}"));
    assert_eq!(first["action"], "inserted", "first policy JSON: {first}");
    let id1 = id1.expect("first policy id");
    let (second, id2) = store_decision_with_ttl(
        &mut conn,
        text,
        Some("typed-kind-oracle".into()),
        Some("policy".into()),
        "oracle-agent".into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("second policy store: {err}"));
    assert_eq!(
        second["action"], "inserted",
        "a second policy must insert, not merge or duplicate-reject: {second}"
    );
    let id2 = id2.expect("second policy id");
    assert_ne!(id1, id2, "each typed policy is its own row");
    let stored: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE type = 'policy' AND decision = ?1",
            [text],
            |row| row.get(0),
        )
        .expect("count policies");
    assert_eq!(stored, 2, "both policy rows must remain");
}

#[test]
fn later_decision_does_not_jaccard_collapse_an_existing_policy() {
    let mut conn = test_conn();
    let text = "Always persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision";
    let (policy, policy_id) = store_decision_with_ttl(
        &mut conn,
        text,
        Some("typed-kind-oracle".into()),
        Some("policy".into()),
        "oracle-agent".into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("policy store: {err}"));
    assert_eq!(policy["action"], "inserted", "policy JSON: {policy}");
    let policy_id = policy_id.expect("policy id");
    let (note, note_id) = store_decision_with_ttl(
        &mut conn,
        text,
        Some("typed-kind-oracle".into()),
        Some("decision".into()),
        "oracle-agent".into(),
        Some(0.9),
        None,
        None,
    )
    .unwrap_or_else(|err| panic!("decision store: {err}"));
    assert_eq!(
        note["action"], "inserted",
        "a later decision must not merge into or duplicate-reject the policy: {note}"
    );
    let note_id = note_id.expect("decision id");
    assert_ne!(policy_id, note_id, "policy and decision must be distinct rows");
    let policy_status: String = conn
        .query_row(
            "SELECT status FROM decisions WHERE id = ?1",
            [policy_id],
            |row| row.get(0),
        )
        .expect("policy status");
    assert_eq!(
        policy_status, "active",
        "a later note must not supersede the policy"
    );
    let kinds: Vec<String> = conn
        .prepare("SELECT type FROM decisions WHERE id IN (?1, ?2) ORDER BY type")
        .unwrap()
        .query_map([policy_id, note_id], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(kinds, vec!["decision".to_string(), "policy".to_string()]);
}
