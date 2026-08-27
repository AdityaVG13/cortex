//! Curated behavioral contract for the store / recall path.
//!
//! Exercises the promoted test-support harness (`solo_state`, `test_conn`)
//! which is the seam the in-tree store/recall tests previously relied on.
use cortex_tests::support::{solo_state, test_conn};

#[test]
fn solo_state_has_expected_defaults() {
    let state = solo_state();
    assert!(!state.team_mode, "solo state must not be team mode");
    assert_eq!(state.port, 7437);
    assert_eq!(state.token.as_str(), "test-token");
}

#[test]
fn test_conn_builds_migrated_in_memory_db() {
    // Building + migrating the in-memory connection must not panic.
    let _conn = test_conn();
}
