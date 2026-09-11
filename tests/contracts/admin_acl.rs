//! Retired: all 13 /admin routes, Bearer/SSRF gates, HTTP status/extractor
//! precedence, team TLS boot refusal, and removed admin CLI payloads.
//! Surviving authority contract: MCP administration requires a global owner/admin
//! role; team membership cannot elevate a member.
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_tests::support::{run_with_cx, team_state};
use serde_json::json;

#[test]
fn admin_acl_team_mode_matrix() {
    run_with_cx(|cx| async move {
        let state = team_state(1);
        {
            let conn = state.db.lock(&cx).await.unwrap();
            cortex_kernel::db::create_team_mode_tables(&conn).unwrap();
            for (id, role) in [(1, "owner"), (2, "admin"), (3, "member")] {
                conn.execute("INSERT INTO users (id, username, display_name, api_key_hash, role) VALUES (?1, ?2, ?2, ?2, ?3)", rusqlite::params![id, format!("user-{id}"), role]).unwrap();
            }
            conn.execute("INSERT INTO teams (id, name) VALUES (1, 'core')", []).unwrap();
            conn.execute("INSERT INTO team_members (team_id, user_id, role) VALUES (1, 3, 'admin')", []).unwrap();
        }
        for caller in [Some(1), Some(2), Some(3), None] {
            let request = json!({"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"cortex_permissions_grant","arguments":{"client":"probe","permission":"read","scope":"*"}}});
            let reply = handle_mcp_message_with_caller(&cx, &state, &request, caller, None).await.unwrap();
            assert_eq!(reply["id"], 17);
            if matches!(caller, Some(1 | 2)) {
                assert_ne!(reply["result"]["isError"], true, "{reply}");
            } else {
                assert_eq!(reply["result"]["isError"], true, "{reply}");
                let text = reply["result"]["content"][0]["text"].as_str().unwrap();
                assert!(text.contains(if caller.is_some() { "team admin role required" } else { "caller-scoped" }), "{reply}");
            }
        }
        let conn = state.db.lock(&cx).await.unwrap();
        let owners: Vec<i64> = conn.prepare("SELECT owner_id FROM client_permissions ORDER BY owner_id").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(owners, vec![1, 2], "denied calls must not grant permissions");
    });
}
