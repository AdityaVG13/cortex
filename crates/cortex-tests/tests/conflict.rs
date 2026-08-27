//! Curated behavioral contract for the conflict-detection layer.
//!
//! `jaccard_similarity` is a small, pure, public function — the ideal seam to
//! re-home from the deleted inline conflict tests.
use cortex_daemon::conflict;

#[test]
fn jaccard_identical_is_one() {
    let sim = conflict::jaccard_similarity("hello world test", "hello world test");
    assert!((sim - 1.0).abs() < 1e-9, "identical inputs -> similarity 1.0, got {sim}");
}

#[test]
fn jaccard_disjoint_is_zero() {
    let sim = conflict::jaccard_similarity("alpha beta gamma", "x y z");
    assert_eq!(sim, 0.0, "disjoint inputs -> similarity 0.0, got {sim}");
}

#[test]
fn jaccard_partial_between_zero_and_one() {
    let sim = conflict::jaccard_similarity("the quick brown fox", "the slow brown dog");
    assert!(sim > 0.0 && sim < 1.0, "partial overlap -> 0 < sim < 1, got {sim}");
}
