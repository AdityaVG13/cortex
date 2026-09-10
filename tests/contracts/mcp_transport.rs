//! Retired HTTP proxy claims: attach-only/service-first spawning, remote URL/env
//! targets, health-identity token fallback, reconnect/unavailable responses,
//! wrapper-owned shutdown, and control-center lock arbitration. No network
//! wrapper is recreated: this pins the surviving in-process MCP conversation.
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::json;

#[test]
fn local_mcp_initialize_notifications_and_discovery() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        for id in [json!(42), json!("string-id")] {
            let reply = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{}}), None, None).await.unwrap();
            assert_eq!(reply["id"], id);
            assert_eq!(reply["jsonrpc"], "2.0");
            assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
            assert_eq!(reply["result"]["serverInfo"]["name"], "cortex");
            assert_eq!(reply["result"]["capabilities"]["tools"]["listChanged"], true);
            assert_eq!(reply["result"]["capabilities"]["resources"]["listChanged"], true);
            assert!(reply.get("error").is_none());
        }
        for method in ["notifications/initialized", "notifications/unknown"] {
            assert!(handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","method":method}), None, None).await.is_none());
        }
        let reply = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}), None, None).await.unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        for name in ["cortex_query", "cortex_commit", "cortex_capabilities"] {
            assert!(tools.iter().any(|tool| tool["name"] == name), "missing {name}: {reply}");
        }
    });
}
