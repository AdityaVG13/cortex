//! Curated behavioral contract for the compiler layer.
//!
//! The compiler's inline tests lived behind `#[cfg(test)]`; this contract
//! keeps the construction seam alive via the promoted harness.
use cortex_tests::support::solo_state;

#[test]
fn solo_state_supports_compiler_contract() {
    let state = solo_state();
    assert!(!state.team_mode);
}
