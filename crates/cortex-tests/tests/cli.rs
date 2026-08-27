//! Curated behavioral contract for the CLI layer.
use cortex_tests::support::solo_state;

#[test]
fn cli_layer_state_contract() {
    let state = solo_state();
    assert!(!state.team_mode);
    assert_eq!(state.port, 7437);
}
