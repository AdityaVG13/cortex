//! Curated behavioral contract for the MCP layers (transport + handler).
use cortex_tests::support::{solo_state, team_state};

#[test]
fn mcp_layers_construct_solo_and_team_state() {
    let solo = solo_state();
    let team = team_state(42);
    assert!(!solo.team_mode);
    assert!(team.team_mode);
    assert_eq!(team.default_owner_id, Some(42));
}
