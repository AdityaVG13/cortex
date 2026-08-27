//! Curated behavioral contract for the server layer.
use cortex_tests::support::{solo_state, team_state};

#[test]
fn server_layer_state_contract() {
    let solo = solo_state();
    let team = team_state(7);
    assert!(!solo.team_mode);
    assert!(team.team_mode);
    assert_eq!(solo.port, team.port);
}
