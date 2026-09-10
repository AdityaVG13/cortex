//! Wave 9: native in-process entry and adapter equality. Store + orient work
//! with no daemon; the native MCP stdio answerer, direct MCP dispatch, the
//! library runtime and the hook path agree on eligible records, revisions
//! and write results because they share one dispatch.

use cortex_daemon::adapter::CapabilityManifest;
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_daemon::handlers::operations::{Caller, Operation, dispatch};
use cortex_daemon::hook_event::{frame_from_host, process};
use cortex_daemon::mcp_native::answer_line;
use cortex_daemon::runtime::{CortexRuntime, LensInput};
use cortex_tests::support::run_with_cx;
use cortex_tests::support::solo_state;
use serde_json::{Value, json};

fn mcp_tool_text(v: &Value) -> Value {
    let text = v["result"]["content"][0]["text"].as_str().unwrap_or("{}");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
fn store_and_orient_without_a_daemon_through_the_native_mcp_entry() {
    run_with_cx(|cx| async move {
        let dir = tempfile::Builder::new()
            .prefix("cortex-native-")
            .tempdir()
            .unwrap();
        let db = dir.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&db).expect("open brain directly");
        let init = answer_line(
            &cx,
            &runtime,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            "native",
        )
        .await
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "cortex");
        assert!(
            answer_line(
                &cx,
                &runtime,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                "native"
            )
            .await
            .is_none()
        );
        let tools = answer_line(
            &cx,
            &runtime,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "native",
        )
        .await
        .unwrap();
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(
            names.contains(&"cortex_commit") && names.contains(&"cortex_orient"),
            "{names:?}"
        );
        let commit = answer_line(&cx, &runtime, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cortex_commit","arguments":{"decision":"NAT-1 native entry writes past the boundary","idempotency_key":"nat-1"}}}"#, "native").await.unwrap();
        let payload = mcp_tool_text(&commit);
        assert!(
            payload["receipt"]["durability"]["local_commit"].is_object(),
            "durable ack only past the write boundary: {commit}"
        );
        let orient = answer_line(&cx, &runtime, r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cortex_orient","arguments":{"task":"NAT-1 native entry"}}}"#, "native").await.unwrap();
        assert!(
            orient.to_string().contains("native entry writes"),
            "{orient}"
        );
        // A second handle over the same backend sees the same durable state.
        let other = CortexRuntime::open_db(&db).unwrap();
        let view = other
            .lens(
                &cx,
                LensInput {
                    query: "NAT-1".into(),
                    agent: "other".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(view.to_string().contains("native entry writes"), "{view}");
        let bad = answer_line(&cx, &runtime, "not json", "native")
            .await
            .unwrap();
        assert_eq!(bad["error"]["code"], -32700);
    });
}

#[test]
fn five_surfaces_agree_on_records_revisions_and_write_results() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        // 1. library
        let lib = runtime
            .deposit_with_key(
                &cx,
                "eq-lib",
                Some("eq-1#decision"),
                "EQ-1 equality across surfaces holds",
                "lib",
                None,
            )
            .await
            .unwrap();
        // 2. Transport-free MCP dispatch replays the same receipt.
        let message = json!({"jsonrpc":"2.0", "id":8, "method":"tools/call", "params":{
            "name":"cortex_commit", "arguments":{
                "decision":"EQ-1 equality across surfaces holds", "idempotency_key":"eq-1"
            }
        }});
        let rpc = mcp_tool_text(
            &handle_mcp_message_with_caller(&cx, &state, &message, None, None)
                .await
                .unwrap(),
        );
        // 3. native MCP
        let mcp = mcp_tool_text(&answer_line(&cx, &runtime, r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"cortex_commit","arguments":{"decision":"EQ-1 equality across surfaces holds","idempotency_key":"eq-1"}}}"#, "mcp").await.unwrap());
        // 4. direct dispatch (what the legacy proxy and CLI reach)
        let direct = dispatch(
            &cx,
            &state,
            Caller {
                owner_id: None,
                agent: "cli",
                principal: "solo".into(),
            },
            Operation::Commit,
            &json!({"decision": "EQ-1 equality across surfaces holds", "idempotency_key": "eq-1"}),
        )
        .await
        .unwrap();
        let ids: Vec<String> = [&rpc, &mcp, &direct]
            .iter()
            .map(|v| v["receipt"]["entries"].to_string())
            .collect();
        assert!(
            ids.iter().all(|i| i == &ids[0]),
            "same key ⇒ same receipt entries on every surface: {ids:?}"
        );
        let lib_decision = serde_json::to_string(&lib.receipt.entries["decision"]).unwrap();
        assert!(
            ids[0].contains(&format!("\"decision.decision\":{lib_decision}")),
            "library receipt replays on every surface: {ids:?} vs {lib_decision}"
        );
        // 5. hook path reads the same eligible record.
        let turn = json!({"hook_event_name": "UserPromptSubmit", "session_id": "eq", "prompt": "EQ-1 equality"});
        let frame = frame_from_host(
            "UserPromptSubmit",
            &turn,
            CapabilityManifest::claude_code_plugin(),
        );
        let hook = process(&cx, &runtime, "hook", &frame, &turn)
            .await
            .expect("hook context");
        assert!(
            hook.additional_context.contains("equality across surfaces"),
            "{hook:?}"
        );
        // Every read surface returns one record, not four.
        let q = dispatch(
            &cx,
            &state,
            Caller {
                owner_id: None,
                agent: "cli",
                principal: "solo".into(),
            },
            Operation::Query,
            &json!({"need": "EQ-1 equality across surfaces"}),
        )
        .await
        .unwrap();
        let n = q["cards"]
            .as_array()
            .map(|c| {
                c.iter()
                    .filter(|c| {
                        c["statement"]
                            .as_str()
                            .unwrap_or("")
                            .contains("equality across surfaces")
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(n, 1, "{q}");
    });
}
