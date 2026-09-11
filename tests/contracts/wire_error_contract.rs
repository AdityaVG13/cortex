//! Retired HTTP claims: JSON Content-Type, 400/405/413/415/422/500 status,
//! Allow, CORS preflight, query/path/body extractors, and 202 notification replies.
//! The local MCP boundary takes parsed Value, so raw-byte -32700 parsing is not
//! claimed here. JSON-RPC request errors and domain invalid_request survive.
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};

#[test]
fn local_rpc_request_errors_and_domain_errors_are_distinct() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        for request in [json!([]), json!([{"jsonrpc":"2.0","id":1,"method":"initialize"}]), json!({"method":"initialize"}), json!({"jsonrpc":"1.0","method":"initialize"}), json!({"jsonrpc":"2.0"})] {
            let reply = handle_mcp_message_with_caller(&cx, &state, &request, None, None).await.unwrap();
            assert_eq!(reply["jsonrpc"], "2.0");
            assert_eq!(reply["id"], Value::Null);
            assert_eq!(reply["error"]["code"], -32600);
            assert!(!reply["error"]["message"].as_str().unwrap().is_empty());
            assert!(reply.get("result").is_none(), "no partial batch results");
        }
        let invalid = dispatch(&cx, &state, Caller { owner_id: None, agent: "errors", principal: "solo".into() }, Operation::Query, &json!({})).await.unwrap();
        assert_eq!(invalid, json!({"status":"invalid_request","error":"need is required","field":"need"}));
        let unknown = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":"missing","method":"absent"}), None, None).await.unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
        assert_eq!(unknown["id"], "missing");
    });
}
