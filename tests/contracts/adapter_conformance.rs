//! Adapter conformance now compares MCP with the direct operation/kernel seam.
//! Retired with the listener: HTTP status/header/auth matrices, GET/POST parity,
//! export/import routes and CLI-vs-HTTP golden summaries. The old HTTP scenario
//! coverage document is historical, not proof of the current local surface.
//! Legacy cortex_boot is the alias of cortex_orient (operations dispatch).
use cortex_daemon::handlers::{
    mcp::handle_mcp_message_with_caller,
    operations::{self, Caller, Operation},
};
use cortex_daemon::{runtime::LensInput, CortexRuntime};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn tool_payload(response: &Value) -> Value {
    assert_eq!(response["jsonrpc"], "2.0");
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    assert_eq!(response["result"]["content"][0]["type"], "text");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn mcp_protocol_adapter_maps_requests_and_errors_exactly() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let state = runtime.state();
        let init = handle_mcp_message_with_caller(
            &cx,
            state,
            &json!({"jsonrpc":"2.0","id":"init","method":"initialize"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(init["id"], "init");
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(init["result"]["serverInfo"]["name"], "cortex");
        assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], true);
        assert!(handle_mcp_message_with_caller(
            &cx,
            state,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            None,
            None
        )
        .await
        .is_none());
        for (msg, id, message) in [
            (
                json!({"id":101,"method":"tools/list"}),
                json!(101),
                "Missing JSON-RPC version",
            ),
            (
                json!({"jsonrpc":"1.0","id":102,"method":"tools/list"}),
                json!(102),
                "Invalid JSON-RPC version",
            ),
            (
                json!({"jsonrpc":"2.0","id":103}),
                json!(103),
                "Missing JSON-RPC method",
            ),
            (
                json!({"jsonrpc":"2.0","id":104,"method":104}),
                json!(104),
                "Missing JSON-RPC method",
            ),
            (json!([]), Value::Null, "Invalid JSON-RPC request"),
        ] {
            let actual = handle_mcp_message_with_caller(&cx, state, &msg, None, None)
                .await
                .unwrap();
            assert_eq!(
                actual,
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":message}})
            );
        }
        let parse = cortex_daemon::mcp_native::answer_line(
            &cx,
            &runtime,
            "not json {{{",
            "adapter-contract",
        )
        .await
        .unwrap();
        assert_eq!(
            parse,
            json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})
        );
        let unknown = handle_mcp_message_with_caller(
            &cx,
            state,
            &json!({"jsonrpc":"2.0","id":105,"method":"not-a-method"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            unknown,
            json!({"jsonrpc":"2.0","id":105,"error":{"code":-32601,"message":"Method not found: not-a-method"}})
        );
        let tools = handle_mcp_message_with_caller(
            &cx,
            state,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let actual: BTreeSet<_> = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let schemas = operations::tool_schemas();
        let expected: BTreeSet<_> = schemas
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(actual, expected);
        for tool in schemas {
            assert_eq!(
                Operation::from_tool_name(tool["name"].as_str().unwrap())
                    .unwrap()
                    .tool_name(),
                tool["name"].as_str().unwrap()
            );
        }
    });
}

#[test]
fn mcp_and_direct_operations_preserve_domain_payloads() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let state = runtime.state();
        let direct = operations::dispatch(
            &cx,
            state,
            Caller {
                owner_id: None,
                agent: "adapter-contract",
                principal: "solo".into(),
            },
            Operation::Capabilities,
            &json!({}),
        )
        .await
        .unwrap();
        let response = handle_mcp_message_with_caller(&cx, state, &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cortex_capabilities","arguments":{}}}), None, None).await.unwrap();
        let adapted = tool_payload(&response);
        for (key, value) in direct.as_object().unwrap() {
            assert_eq!(&adapted[key], value, "adapter changed {key}");
        }
        let text = "Adapter conformance sentinel memory: shard failover requires three independent quorum votes.";
        let stored = handle_mcp_message_with_caller(&cx, state, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"cortex_commit","arguments":{"decision":text,"agent":"adapter-contract","source_model":"gpt-5.4","reasoning_depth":"high","confidence":0.93}}}), None, None).await.unwrap();
        let stored = tool_payload(&stored);
        assert_eq!(stored["status"], "ok");
        assert_eq!(stored["stored"], 1);
        assert_eq!(stored["legacy_entries"][0]["action"], "inserted");
        assert_eq!(stored["legacy_entries"][0]["status"], "active");
        assert!(stored["receipt"].is_object());
        let direct = runtime
            .lens(
                &cx,
                LensInput {
                    query: text.into(),
                    budget: 200,
                    k: 5,
                    agent: "adapter-contract".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for tool in ["cortex_recall", "cortex_peek"] {
            let response = handle_mcp_message_with_caller(&cx, state, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":tool,"arguments":{"query":text,"budget":200,"k":5,"agent":"adapter-contract"}}}), None, None).await.unwrap();
            let payload = tool_payload(&response);
            let excerpts = |value: &Value| -> BTreeSet<String> {
                value["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| r["excerpt"].as_str().unwrap().to_string())
                    .collect()
            };
            assert_eq!(excerpts(&payload), excerpts(&direct));
            assert!(excerpts(&payload).contains(text));
        }
        let health = handle_mcp_message_with_caller(&cx, state, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cortex_health","arguments":{}}}), None, None).await.unwrap();
        let health = tool_payload(&health);
        let direct = cortex_daemon::handlers::health::build_health_payload(&cx, state, false)
            .await
            .unwrap();
        for key in ["status", "degraded", "db_corrupted"] {
            assert_eq!(health[key], direct[key]);
        }
        // Legacy boot is orient, not a dead special-case shape.
        let direct_orient = operations::dispatch(
            &cx,
            state,
            Caller {
                owner_id: None,
                agent: "adapter-contract",
                principal: "solo".into(),
            },
            Operation::Orient,
            &json!({"task":"adapter boot alias","budget":2000}),
        )
        .await
        .unwrap();
        let boot = handle_mcp_message_with_caller(&cx, state, &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"cortex_boot","arguments":{"task":"adapter boot alias","budget":2000}}}), None, None).await.unwrap();
        let boot = tool_payload(&boot);
        assert_eq!(boot["profile"], "orient");
        assert_eq!(boot["status"], direct_orient["status"]);
    });
}
