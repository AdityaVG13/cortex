// SPDX-License-Identifier: MIT
//! MCP permission boundaries only. Wire contracts live in daemon-rs/tests/.

use super::*;
use crate::handlers::mcp::permissions::permission_satisfies;

#[test]
fn conflict_tools_require_admin_permission_scope() {
    assert_eq!(
        required_permission_for_tool("cortex_conflicts_list"),
        Some(ClientPermission::Admin)
    );
    assert_eq!(
        required_permission_for_tool("cortex_recall"),
        Some(ClientPermission::Read)
    );
}

#[test]
fn normalize_permission_client_id_strips_parenthetical_suffix() {
    assert_eq!(
        normalize_permission_client_id("claude(sonnet-4-20250514)"),
        "claude"
    );
    assert_eq!(
        normalize_permission_client_id("claude-sonnet-4-20250514"),
        "claude-sonnet-4-20250514"
    );
}

#[test]
fn parse_client_permission_accepts_known_values() {
    assert_eq!(parse_client_permission("read"), Some(ClientPermission::Read));
    assert_eq!(parse_client_permission("admin"), Some(ClientPermission::Admin));
    assert_eq!(parse_client_permission("unknown"), None);
}

#[test]
fn client_permission_satisfies_admin_implies_read_and_write() {
    assert!(permission_satisfies("admin", ClientPermission::Read));
    assert!(permission_satisfies("admin", ClientPermission::Write));
    assert!(permission_satisfies("write", ClientPermission::Read));
    assert!(!permission_satisfies("read", ClientPermission::Write));
}
