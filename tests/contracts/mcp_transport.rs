//! Retired HTTP proxy claims: attach-only/service-first spawning, remote URL/env
//! targets, health-identity token fallback, reconnect/unavailable responses,
//! wrapper-owned shutdown, and control-center lock arbitration. No network
//! wrapper is recreated: this pins the surviving in-process MCP conversation.
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{Value, json};

#[test]
fn local_mcp_initialize_notifications_and_discovery() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        for id in [json!(42), json!("string-id")] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{}}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(reply["id"], id);
            assert_eq!(reply["jsonrpc"], "2.0");
            assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
            assert_eq!(reply["result"]["serverInfo"]["name"], "cortex");
            assert_eq!(
                reply["result"]["capabilities"]["tools"]["listChanged"],
                true
            );
            assert_eq!(
                reply["result"]["capabilities"]["resources"]["listChanged"],
                true
            );
            assert!(reply.get("error").is_none());
        }
        for method in ["notifications/initialized", "notifications/unknown"] {
            assert!(
                handle_mcp_message_with_caller(
                    &cx,
                    &state,
                    &json!({"jsonrpc":"2.0","method":method}),
                    None,
                    None
                )
                .await
                .is_none()
            );
        }
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        for name in ["cortex_query", "cortex_commit", "cortex_capabilities"] {
            assert!(
                tools.iter().any(|tool| tool["name"] == name),
                "missing {name}: {reply}"
            );
        }
    });
}

fn mcp_tool_text(reply: &Value) -> Value {
    let text = reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("{}");
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
        assert_eq!(
            parsed,
            vec!["memory::1".to_string(), "decision::2".to_string()]
        );
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

#[test]
fn last_call_matches_model_suffixed_source_agent() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        {
            let conn = state.db.lock(&cx).await.expect("database lock");
            conn.execute(
                "INSERT INTO memories (text, type, source_agent, status) VALUES (?1, 'note', ?2, 'active')",
                rusqlite::params!["last-call suffix marker", "cli (opus)"],
            )
            .expect("insert suffixed memory");
        }
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({
                "jsonrpc":"2.0",
                "id":31,
                "method":"tools/call",
                "params":{
                    "name":"cortex_lastCall",
                    "arguments":{"kind":"memory","agent":"cli"}
                }
            }),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let payload = mcp_tool_text(&reply);
        assert_eq!(payload["found"], json!(true), "{payload}");
        assert_eq!(payload["kind"], json!("memory"), "{payload}");
        assert_eq!(
            payload["sourceAgent"],
            json!("cli (opus)"),
            "lastCall agent=cli must include rows stored as cli (opus): {payload}"
        );
        assert_eq!(
            payload["summary"],
            json!("last-call suffix marker"),
            "{payload}"
        );
    });
}

#[test]
fn agent_feedback_stats_match_model_suffixed_agent() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let record = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({
                "jsonrpc":"2.0",
                "id":32,
                "method":"tools/call",
                "params":{
                    "name":"cortex_agent_feedback_record",
                    "arguments":{
                        "agent":"cli (opus)",
                        "outcome":"success",
                        "outcomeScore":1.0
                    }
                }
            }),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(record.get("error").is_none(), "{record}");
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({
                "jsonrpc":"2.0",
                "id":33,
                "method":"tools/call",
                "params":{
                    "name":"cortex_agent_feedback_stats",
                    "arguments":{"agent":"cli","horizonDays":30}
                }
            }),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let payload = mcp_tool_text(&reply);
        assert!(
            payload["sampled"].as_i64().unwrap_or(0) >= 1,
            "stats agent=cli must include rows recorded as cli (opus): {payload}"
        );
    });
}

#[test]
fn last_call_does_not_hide_older_agent_behind_newer_others() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        {
            let conn = state.db.lock(&cx).await.expect("database lock");
            conn.execute(
                "INSERT INTO memories (text, type, source_agent, status) VALUES (?1, 'note', ?2, 'active')",
                rusqlite::params!["cli last-call needle", "cli"],
            )
            .expect("insert cli memory");
            for i in 0..40 {
                conn.execute(
                    "INSERT INTO memories (text, type, source_agent, status) VALUES (?1, 'note', ?2, 'active')",
                    rusqlite::params![format!("other last-call decoy {i}"), "other-agent"],
                )
                .expect("insert decoy memory");
            }
        }
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({
                "jsonrpc":"2.0",
                "id":34,
                "method":"tools/call",
                "params":{
                    "name":"cortex_lastCall",
                    "arguments":{"kind":"memory","agent":"cli"}
                }
            }),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let payload = mcp_tool_text(&reply);
        assert_eq!(payload["found"], json!(true), "{payload}");
        assert_eq!(payload["kind"], json!("memory"), "{payload}");
        assert_eq!(payload["sourceAgent"], json!("cli"), "{payload}");
        assert_eq!(
            payload["summary"],
            json!("cli last-call needle"),
            "{payload}"
        );
    });
}

fn modern_meta_with_caps(caps: Value) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": caps,
    })
}

fn elicitation_caps() -> Value {
    json!({"elicitation": {"form": {}}})
}

#[test]
fn mcp_initialize_always_selects_legacy() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        // `initialize` always selects legacy semantics, whatever the client
        // asks for: the handshake revision spoken here is 2024-11-05.
        // Modern clients probe `server/discover` instead of initializing.
        for (id, version) in [(1, "2024-11-05"), (2, "2026-07-28"), (3, "2025-11-25")] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                reply["result"]["protocolVersion"], "2024-11-05",
                "asked {version}: {reply}"
            );
            assert_eq!(
                reply["result"]["capabilities"]["tools"]["listChanged"], true,
                "{reply}"
            );
            assert!(reply["result"].get("instructions").is_none(), "{reply}");
            assert!(reply["result"].get("resultType").is_none(), "{reply}");
        }
    });
}

#[test]
fn mcp_tools_list_has_titles_descriptions_annotations_and_output_schemas() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 8, "{reply}");
        for tool in tools {
            assert!(!tool["title"].as_str().unwrap_or("").is_empty(), "{tool}");
            assert!(
                !tool["description"].as_str().unwrap_or("").is_empty(),
                "{tool}"
            );
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
            assert_eq!(tool["outputSchema"]["type"], "object", "{tool}");
            assert!(tool["annotations"].is_object(), "{tool}");
            assert_eq!(tool["annotations"]["openWorldHint"], false, "{tool}");
        }
        let query = tools.iter().find(|t| t["name"] == "cortex_query").unwrap();
        assert_eq!(query["annotations"]["readOnlyHint"], true);
        let commit = tools.iter().find(|t| t["name"] == "cortex_commit").unwrap();
        assert_eq!(commit["annotations"]["destructiveHint"], false);
        assert!(
            commit["annotations"].get("readOnlyHint").is_none(),
            "{commit}"
        );
        // Deterministic order across calls (spec SHOULD for client caching).
        let again = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":44,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"], again["result"]);
    });
}

#[test]
fn mcp_tool_calls_return_structured_content_and_envelope_errors() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        // Success: structuredContent mirrors the text payload exactly.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"cortex_capabilities","arguments":{}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        assert_eq!(
            mcp_tool_text(&reply),
            reply["result"]["structuredContent"],
            "{reply}"
        );
        // Domain error: result envelope with invalid_request status, mirrored
        // in structuredContent and flagged isError — not a JSON-RPC error.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"cortex_query","arguments":{}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        let payload = mcp_tool_text(&reply);
        assert_eq!(payload["status"], "invalid_request", "{reply}");
        assert_eq!(payload, reply["result"]["structuredContent"], "{reply}");
        assert_eq!(reply["result"]["isError"], true, "{reply}");
        // Honest non-errors stay unflagged: no_match is an answer, not a failure.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":66,"method":"tools/call","params":{"name":"cortex_query","arguments":{"need":"zzz-no-such-memory-zzz"}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(mcp_tool_text(&reply)["status"], "no_match", "{reply}");
        assert!(reply["result"].get("isError").is_none(), "{reply}");
        // Unknown tool: JSON-RPC method-not-found with typed discovery data.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"bogus_tool","arguments":{}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["error"]["code"], -32601);
        assert_eq!(reply["error"]["data"]["errorType"], "UNKNOWN_TOOL");
        assert!(reply["error"]["data"]["suggestions"].is_array(), "{reply}");
    });
}

#[test]
fn mcp_server_discover_advertises_versions_caps_and_instructions() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":8,"method":"server/discover"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            reply["result"]["supportedVersions"],
            json!(["2026-07-28", "2024-11-05"])
        );
        for surface in ["tools", "resources", "prompts", "completions"] {
            assert!(
                reply["result"]["capabilities"].get(surface).is_some(),
                "{reply}"
            );
        }
        assert!(
            reply["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("cortex_query"),
            "{reply}"
        );
        // Discovery implies modern framing even without request `_meta`.
        assert_eq!(reply["result"]["resultType"], "complete");
        assert_eq!(
            reply["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "cortex"
        );
        assert_eq!(reply["result"]["ttlMs"], 3_600_000);
        assert_eq!(reply["result"]["cacheScope"], "public");
    });
}

#[test]
fn mcp_prompts_list_get_and_completion_complete() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":9,"method":"prompts/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let names: Vec<&str> = reply["result"]["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"recall-briefing"), "{names:?}");
        assert!(names.contains(&"capture-decision"), "{names:?}");
        assert!(names.contains(&"session-recap"), "{names:?}");
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":10,"method":"prompts/get","params":{"name":"capture-decision","arguments":{"decision":"ship it","retention":"durable"}}}),
            None,
            None,
        )
        .await
        .unwrap();
        let text = reply["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("ship it"), "{reply}");
        assert!(text.contains("durable"), "{reply}");
        // Missing required arg names itself so the model can self-correct.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":11,"method":"prompts/get","params":{"name":"capture-decision","arguments":{}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            reply["result"]["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("decision"),
            "{reply}"
        );
        // Unknown prompt is a typed error, not a silent empty.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":12,"method":"prompts/get","params":{"name":"nope","arguments":{}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["data"]["errorType"], "UNKNOWN_PROMPT");
        // Completions narrow the recall-briefing profile argument by prefix.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":13,"method":"completion/complete","params":{"ref":{"type":"ref/prompt","name":"recall-briefing"},"argument":{"name":"profile","value":"a"}}}),
            None,
            None,
        )
        .await
        .unwrap();
        let values = reply["result"]["completion"]["values"].as_array().unwrap();
        assert!(!values.is_empty(), "{reply}");
        for value in values {
            assert!(value.as_str().unwrap().starts_with("a"), "{reply}");
        }
        assert_eq!(reply["result"]["completion"]["hasMore"], false);
    });
}

#[test]
fn mcp_resources_list_templates_and_read() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":14,"method":"resources/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let uris: Vec<&str> = reply["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert_eq!(
            uris,
            vec!["cortex://tooling/capabilities", "cortex://tooling/tools"]
        );
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":15,"method":"resources/templates/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let templates: Vec<&str> = reply["result"]["resourceTemplates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["uriTemplate"].as_str().unwrap())
            .collect();
        assert_eq!(templates, vec!["cortex://tooling/{doc}"]);
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":16,"method":"resources/read","params":{"uri":"cortex://tooling/tools"}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            reply["result"]["contents"][0]["uri"],
            "cortex://tooling/tools"
        );
        assert_eq!(
            reply["result"]["contents"][0]["mimeType"],
            "application/json"
        );
        let payload: Value =
            serde_json::from_str(reply["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
        assert!(payload.is_object(), "{payload}");
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":17,"method":"resources/read","params":{"uri":"cortex://nope/x"}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["error"]["code"], -32602);
        assert_eq!(reply["error"]["data"]["errorType"], "UNKNOWN_RESOURCE");
    });
}

#[test]
fn mcp_ping_unsupported_methods_and_standalone_notifications() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":18,"method":"ping"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"], json!({}));
        for (id, method, error_type) in [
            (19, "logging/setLevel", "LOGGING_LEVEL_UNSUPPORTED"),
            (22, "subscriptions/listen", "SUBSCRIPTIONS_UNSUPPORTED"),
            (20, "resources/subscribe", "SUBSCRIPTIONS_UNSUPPORTED"),
            (21, "resources/unsubscribe", "SUBSCRIPTIONS_UNSUPPORTED"),
        ] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":{}}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(reply["error"]["code"], -32601, "{reply}");
            assert_eq!(reply["error"]["data"]["errorType"], error_type, "{reply}");
        }
        for method in [
            "notifications/cancelled",
            "notifications/progress",
            "notifications/roots/list_changed",
        ] {
            assert!(
                handle_mcp_message_with_caller(
                    &cx,
                    &state,
                    &json!({"jsonrpc":"2.0","method":method}),
                    None,
                    None
                )
                .await
                .is_none(),
                "{method} must be accepted silently"
            );
        }
    });
}

#[test]
fn mcp_modern_requests_carry_result_type_identity_and_cacheability() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let meta = modern_meta_with_caps(json!({}));
        // Modern catalog: framing plus cacheability.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":30,"method":"tools/list","params":{"_meta":meta}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "complete");
        assert_eq!(
            reply["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "cortex"
        );
        assert_eq!(reply["result"]["ttlMs"], 3_600_000);
        assert_eq!(reply["result"]["cacheScope"], "public");
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 8);
        // Legacy-era catalog: same data, no modern framing.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":31,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 8);
        assert!(reply["result"].get("resultType").is_none(), "{reply}");
        assert!(reply["result"].get("_meta").is_none(), "{reply}");
        assert!(reply["result"].get("ttlMs").is_none(), "{reply}");
        // Modern tool call: framing wraps the normal envelope.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"cortex_capabilities","arguments":{},"_meta":meta}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "complete");
        assert_eq!(
            mcp_tool_text(&reply),
            reply["result"]["structuredContent"],
            "{reply}"
        );
        // Modern prompts, completions, and resource reads frame too.
        for (id, method, params) in [
            (33, "prompts/list", json!({"_meta":meta})),
            (
                34,
                "completion/complete",
                json!({"ref":{"type":"ref/prompt","name":"recall-briefing"},"argument":{"name":"profile","value":""},"_meta":meta}),
            ),
            (
                35,
                "resources/read",
                json!({"uri":"cortex://tooling/tools","_meta":meta}),
            ),
        ] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                reply["result"]["resultType"], "complete",
                "{method}: {reply}"
            );
        }
    });
}

#[test]
fn mcp_unsupported_protocol_version_rejected_with_exact_shape() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        for (id, method, params) in [
            (
                40,
                "tools/list",
                json!({"_meta":{"io.modelcontextprotocol/protocolVersion":"2025-11-25"}}),
            ),
            (
                41,
                "tools/call",
                json!({"name":"cortex_capabilities","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01"}}),
            ),
        ] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(reply["error"]["code"], -32022, "{reply}");
            assert_eq!(reply["error"]["message"], "Unsupported protocol version");
            assert_eq!(
                reply["error"]["data"],
                json!({"supported":["2026-07-28","2024-11-05"],"requested":params["_meta"]["io.modelcontextprotocol/protocolVersion"]}),
                "{reply}"
            );
        }
        // An explicit legacy version in `_meta` stays legacy, never an error.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":42,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2024-11-05"}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        assert!(reply["result"].get("resultType").is_none(), "{reply}");
    });
}

#[test]
fn mcp_mrtr_elicitation_round_trip_for_missing_need() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let meta = modern_meta_with_caps(elicitation_caps());
        // Missing flat field becomes a form request, with no requestState:
        // retries are self-contained.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":50,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":meta}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "input_required", "{reply}");
        assert!(reply["result"].get("requestState").is_none(), "{reply}");
        let asked = &reply["result"]["inputRequests"]["missing_need"];
        assert_eq!(asked["method"], "elicitation/create");
        assert_eq!(asked["params"]["mode"], "form");
        assert!(
            asked["params"]["message"]
                .as_str()
                .unwrap()
                .contains("need"),
            "{reply}"
        );
        assert_eq!(
            asked["params"]["requestedSchema"]["required"],
            json!(["need"])
        );
        assert_eq!(
            asked["params"]["requestedSchema"]["properties"]["need"]["type"],
            "string"
        );
        // Retry with the elicited value completes the original call.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":51,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":meta,
                "inputResponses":{"missing_need":{"action":"accept","content":{"need":"MRTR-1 memory probe"}}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
        assert_eq!(mcp_tool_text(&reply)["status"], "no_match", "{reply}");
        // Refusal ends the call with a terminal envelope, not a re-prompt.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":52,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":meta,
                "inputResponses":{"missing_need":{"action":"decline"}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        let payload = mcp_tool_text(&reply);
        assert_eq!(payload["status"], "invalid_request", "{reply}");
        assert!(
            payload["error"].as_str().unwrap().contains("declined"),
            "{reply}"
        );
        assert_eq!(reply["result"]["isError"], true);
        // Accepted-but-empty re-prompts: missing info asks again.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":53,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":meta,
                "inputResponses":{"missing_need":{"action":"accept","content":{"need":"   "}}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "input_required", "{reply}");
    });
}

#[test]
fn mcp_mrtr_falls_back_without_form_support_and_rejects_malformed_retries() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        // Modern request, no elicitation capability: plain envelope, no prompt.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":60,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":modern_meta_with_caps(json!({}))}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
        assert!(reply["result"].get("inputRequests").is_none(), "{reply}");
        assert_eq!(mcp_tool_text(&reply)["status"], "invalid_request");
        // Malformed retries are protocol errors, not prompts.
        let meta = modern_meta_with_caps(elicitation_caps());
        for (id, responses) in [
            (61, json!("nope")),
            (62, json!({"missing_need": {"action": "maybe"}})),
            (
                63,
                json!({"missing_need": {"action": "accept", "content": {"need": 42}}}),
            ),
        ] {
            let reply = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"cortex_query","arguments":{},"_meta":meta,"inputResponses":responses}}),
                None,
                None,
            )
            .await
            .unwrap();
            assert_eq!(reply["error"]["code"], -32602, "{reply}");
            assert_eq!(
                reply["error"]["data"]["errorType"],
                "INVALID_INPUT_RESPONSES"
            );
        }
        // Unknown response keys are ignored, never errors.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":64,"method":"tools/call","params":{"name":"cortex_query","arguments":{"need":"MRTR-2"},"_meta":meta,
                "inputResponses":{"bogus_key":{"action":"accept","content":{"x":1}}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(mcp_tool_text(&reply)["status"], "no_match", "{reply}");
    });
}

#[test]
fn mcp_mrtr_elicits_checkpoint_fields_with_one_prompt_each() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let meta = modern_meta_with_caps(elicitation_caps());
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":70,"method":"tools/call","params":{"name":"cortex_checkpoint","arguments":{},"_meta":meta}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "input_required", "{reply}");
        assert!(
            reply["result"]["inputRequests"]
                .get("missing_thread")
                .is_some(),
            "{reply}"
        );
        // Same field answered yet still failing falls back to the envelope:
        // an unknown action elicits the action enum, and answering with the
        // same unknown value ends the round-trip instead of re-prompting.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":71,"method":"tools/call","params":{"name":"cortex_checkpoint",
                "arguments":{"thread":"mrtr-t","action":"frobnicate"},"_meta":meta}}),
            None,
            None,
        )
        .await
        .unwrap();
        let asked = &reply["result"]["inputRequests"]["missing_action"];
        assert_eq!(
            asked["params"]["requestedSchema"]["properties"]["action"]["enum"],
            json!([
                "checkpoint",
                "status",
                "obligation",
                "transition",
                "verify",
                "revalidate",
                "attempt"
            ]),
            "{reply}"
        );
        // Answering the enum with an off-enum value violates the requested
        // schema: a protocol error, not a re-prompt.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":72,"method":"tools/call","params":{"name":"cortex_checkpoint",
                "arguments":{"thread":"mrtr-t","action":"frobnicate"},"_meta":meta,
                "inputResponses":{"missing_action":{"action":"accept","content":{"action":"frobnicate"}}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["error"]["code"], -32602, "{reply}");
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .contains("action"),
            "{reply}"
        );
        // Answering with a valid action completes the round-trip chain: the
        // default checkpoint write runs against the elicited thread.
        let reply = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":73,"method":"tools/call","params":{"name":"cortex_checkpoint",
                "arguments":{"action":"status"},"_meta":meta,
                "inputResponses":{"missing_thread":{"action":"accept","content":{"thread":"mrtr-t2"}}}}}),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(reply["result"]["resultType"], "complete", "{reply}");
        assert_eq!(mcp_tool_text(&reply)["status"], "ok", "{reply}");
    });
}
