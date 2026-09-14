//! Retired HTTP proxy claims: attach-only/service-first spawning, remote URL/env
//! targets, health-identity token fallback, reconnect/unavailable responses,
//! wrapper-owned shutdown, and control-center lock arbitration. No network
//! wrapper is recreated: this pins the surviving in-process MCP conversation.
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};

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

fn mcp_tool_text(reply: &Value) -> Value {
    let text = reply["result"]["content"][0]["text"].as_str().unwrap_or("{}");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[test]
fn agent_feedback_record_parses_stringified_scores_and_sources() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({
                "jsonrpc":"2.0",
                "id":11,
                "method":"tools/call",
                "params":{
                    "name":"cortex_agent_feedback_record",
                    "arguments":{
                        "outcome":"success",
                        "outcomeScore":"0.2",
                        "qualityScore":"0.3",
                        "memorySources":"memory::1, decision::2",
                        "latencyMs":"12",
                        "retries":"1"
                    }
                }
            }),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let payload = mcp_tool_text(&reply);
        assert_eq!(payload["stored"], json!(true), "{payload}");
        assert_eq!(payload["outcomeScore"], json!(0.2), "{payload}");
        assert_eq!(payload["qualityScore"], json!(0.3), "{payload}");
        assert_eq!(
            payload["memorySources"],
            json!(["memory::1", "decision::2"]),
            "{payload}"
        );
        let conn = state.db.lock(&cx).await.expect("database lock");
        let (outcome_score, quality_score, sources): (f64, f64, String) = conn
            .query_row(
                "SELECT outcome_score, quality_score, memory_sources_json FROM agent_feedback ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("agent_feedback row");
        assert!(
            (outcome_score - 0.2).abs() < 1e-12,
            "stored outcome_score={outcome_score}"
        );
        assert!(
            (quality_score - 0.3).abs() < 1e-12,
            "stored quality_score={quality_score}"
        );
        let parsed: Vec<String> = serde_json::from_str(&sources).expect("sources json");
        assert_eq!(parsed, vec!["memory::1".to_string(), "decision::2".to_string()]);
    });
}

#[test]
fn query_budget_parses_json_float_and_decimal_string() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        for (id, budget) in [(21, json!(100.0)), (22, json!("100"))] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "method":"tools/call",
                    "params":{
                        "name":"cortex_query",
                        "arguments":{"need":"pass64 budget parse","budget":budget}
                    }
                }),
                None,
                None,
            )
            .await
            .unwrap();
            assert!(reply.get("error").is_none(), "{reply}");
            let payload = mcp_tool_text(&reply);
            assert_eq!(
                payload["budget"]["bytes"],
                json!(100),
                "budget={budget} must not fall through to the 2000 default: {payload}"
            );
        }
    });
}
