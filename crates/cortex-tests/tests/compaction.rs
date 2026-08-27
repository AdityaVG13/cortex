//! Curated behavioral contract for the compaction layer.
//!
//! Compaction historically had its own `tests/` module inside the daemon; this
//! thin contract re-homes the construction seam via the promoted harness.
use cortex_tests::support::solo_state;

#[test]
fn solo_state_is_compaction_ready() {
    let state = solo_state();
    assert!(!state.team_mode);
    assert_eq!(state.port, 7437);
}
