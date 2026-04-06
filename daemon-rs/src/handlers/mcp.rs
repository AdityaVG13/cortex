// SPDX-License-Identifier: MIT
use serde_json::{json, Value};

use super::health::build_digest;
use super::mutate::{forget_keyword, resolve_decision};
use super::recall::{execute_unified_recall, unfold_source, RecallContext};
use super::store::store_decision;
use super::{estimate_tokens, now_iso};
use crate::state::RuntimeState;

// ─── JSON-RPC helpers ─────────────────────────────────────────────────────────

pub fn mcp_success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn mcp_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn wrap_mcp_tool_result(_state: &RuntimeState, data: Value) -> Value {
    let text = match &data {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    json!({
        "content": [{
            "type": "text",
            "text": text
        }]
    })
}

fn wrap_mcp_tool_result_verbose(state: &RuntimeState, data: Value) -> Value {
    let calls = state.next_mcp_call();
    let decorated = match data {
        Value::Object(mut map) => {
            map.insert("_liveness".to_string(), Value::Bool(true));
            map.insert("_ts".to_string(), Value::String(now_iso()));
            map.insert("_calls".to_string(), Value::Number(calls.into()));
            Value::Object(map)
        }
        other => json!({
            "value": other,
            "_liveness": true,
            "_ts": now_iso(),
            "_calls": calls
        }),
    };

    json!({
        "content": [{
            "type": "text",
            "text": decorated.to_string()
        }]
    })
}

// ─── MCP tool definitions ─────────────────────────────────────────────────────

pub fn mcp_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "cortex_boot",
            "description": "Get compiled boot prompt with session context. Uses capsule system: identity (stable) + delta (what changed since your last boot). Call once at session start.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": { "type": "string", "description": "Legacy profile name. Ignored when agent is set." },
                    "agent": { "type": "string", "description": "Your agent ID (e.g. claude-opus, gemini, codex). Enables delta tracking." },
                    "budget": { "type": "number", "description": "Max token budget for boot prompt (default: 600)" }
                }
            }
        }),
        json!({
            "name": "cortex_peek",
            "description": "Lightweight check: returns source names and relevance scores only (no excerpts). Use BEFORE cortex_recall to check if relevant memories exist. Saves ~80% tokens vs full recall.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "limit": { "type": "number", "description": "Max results (default 10)" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "cortex_recall",
            "description": "Search Cortex brain for memories and decisions. Adapts detail level to token budget: 0=headlines, 200=balanced, 500+=full.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "budget": { "type": "number", "description": "Token budget. 0=headlines only, 200=balanced, 500+=full detail" },
                    "agent": { "type": "string", "description": "Optional agent id for dedup/predictive cache" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "cortex_store",
            "description": "Store a decision or insight with conflict detection and dedup.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "decision": { "type": "string", "description": "The decision or insight text" },
                    "context": { "type": "string", "description": "Optional context about where/why" },
                    "type": { "type": "string", "description": "Entry type (default: decision)" },
                    "source_agent": { "type": "string", "description": "Agent that produced this" },
                    "confidence": { "type": "number", "description": "Confidence score 0-1 (default: 0.8)" }
                },
                "required": ["decision"]
            }
        }),
        json!({
            "name": "cortex_health",
            "description": "Check Cortex system health: DB stats, memory counts.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "cortex_digest",
            "description": "Daily health digest: memory counts, today's activity, top recalls, decay stats, agent boots. Use to check if the brain is compounding.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "cortex_forget",
            "description": "Decay matching memories/decisions by keyword (multiply score by 0.3).",
            "inputSchema": {
                "type": "object",
                "properties": { "source": { "type": "string", "description": "Keyword to match for decay" } },
                "required": ["source"]
            }
        }),
        json!({
            "name": "cortex_resolve",
            "description": "Resolve a disputed decision pair.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keepId": { "type": "number", "description": "ID of the decision to keep" },
                    "action": { "type": "string", "enum": ["keep", "merge"], "description": "Resolution action" },
                    "supersededId": { "type": "number", "description": "ID of the decision to supersede (for keep action)" }
                },
                "required": ["keepId", "action"]
            }
        }),
        json!({
            "name": "cortex_unfold",
            "description": "Get full text of specific memory/decision nodes by source string. Use AFTER cortex_peek to drill into selected items. Progressive disclosure: peek (headlines) -> unfold (full text of 2-3 items you need).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "sources": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Source strings from cortex_peek results (e.g. [\"memory::project_cortex_plan.md\", \"decision::28\"])"
                    }
                },
                "required": ["sources"]
            }
        }),
        json!({
            "name": "cortex_focus_start",
            "description": "Start a focus session (context checkpoint). Entries stored during focus are tracked. Call focus_end to consolidate into a summary. Implements the sawtooth pattern for token reduction.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "Name for this focus block (e.g. 'auth-refactor', 'bug-investigation')" },
                    "agent": { "type": "string", "description": "Agent ID" }
                },
                "required": ["label"]
            }
        }),
        json!({
            "name": "cortex_focus_end",
            "description": "End a focus session. Summarizes all entries captured during the session, stores the summary, discards raw traces. Returns token savings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "Label of the focus session to close" },
                    "agent": { "type": "string", "description": "Agent ID" }
                },
                "required": ["label"]
            }
        }),
        json!({
            "name": "cortex_focus_status",
            "description": "Check focus session state: current open session (if any) and recent closed sessions with summaries and token savings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Agent ID (default: mcp)" }
                }
            }
        }),
        json!({
            "name": "cortex_diary",
            "description": "Write session state to state.md for cross-session continuity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "accomplished": { "type": "string", "description": "What was done this session" },
                    "nextSteps": { "type": "string", "description": "What to do next session" },
                    "decisions": { "type": "string", "description": "Key decisions made" },
                    "pending": { "type": "string", "description": "Pending work items" },
                    "knownIssues": { "type": "string", "description": "Known issues to address" }
                }
            }
        }),
    ]
}

// ─── Dispatch ─────────────────────────────────────────────────────────────────

async fn mcp_dispatch(
    state: &RuntimeState,
    caller_id: Option<i64>,
    tool_name: &str,
    args: &Value,
) -> Result<Value, String> {
    match tool_name {
        "cortex_boot" => {
            let profile = args
                .get("profile")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let agent = args
                .get("agent")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("source_agent").and_then(|v| v.as_str()))
                .unwrap_or("mcp")
                .to_string();
            let _budget = args.get("budget").and_then(|v| v.as_u64()).unwrap_or(600) as usize;
            let profile_str = profile.unwrap_or_else(|| "full".to_string());

            // Clear served content for this agent on boot
            {
                let mut served = state.served_content.lock().await;
                served.remove(&agent);
            }

            let conn = state.db.lock().await;

            // Use the full capsule compiler (same as HTTP /boot).
            let result = crate::compiler::compile(&conn, &state.home, &agent, _budget);

            // Auto-ack feed on boot: advance last_seen_id to latest feed entry.
            if let Ok(latest_id) = conn.query_row(
                "SELECT id FROM feed ORDER BY timestamp DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            ) {
                if state.team_mode {
                    if let Some(owner_id) = state.default_owner_id {
                        let _ = conn.execute(
                            "INSERT INTO feed_acks (owner_id, agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3, datetime('now')) \
                             ON CONFLICT(owner_id, agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
                            rusqlite::params![owner_id, agent, latest_id],
                        );
                    }
                } else {
                    let _ = conn.execute(
                        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, datetime('now')) \
                         ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
                        rusqlite::params![agent, latest_id],
                    );
                }
            }

            crate::db::checkpoint_wal_best_effort(&conn);

            state.emit(
                "agent_boot",
                json!({"agent": agent.clone(), "profile": profile_str.clone()}),
            );

            Ok(json!({
                "bootPrompt": result.boot_prompt,
                "tokenEstimate": result.token_estimate,
                "profile": if profile_str == "full" { "capsules" } else { &profile_str },
                "capsules": result.capsules,
                "savings": result.savings
            }))
        }

        "cortex_peek" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let ctx = RecallContext::from_caller(caller_id, state);
            let results = execute_unified_recall(state, query, 0, limit, "mcp", &ctx).await?;
            Ok(results)
        }

        "cortex_recall" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let budget = args
                .get("budget")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(200);
            let agent = args
                .get("source_agent")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("agent").and_then(|v| v.as_str()))
                .unwrap_or("mcp");

            let ctx = RecallContext::from_caller(caller_id, state);
            execute_unified_recall(state, query, budget, 10, agent, &ctx).await
        }

        "cortex_store" => {
            let decision = args
                .get("decision")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: decision".to_string())?;
            let context = args
                .get("context")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let entry_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let source_agent = args
                .get("source_agent")
                .and_then(|v| v.as_str())
                .unwrap_or("mcp")
                .to_string();
            let confidence = args.get("confidence").and_then(|v| v.as_f64());

            let mut conn = state.db.lock().await;
            let (entry, _id) = store_decision(
                &mut conn,
                decision,
                context,
                entry_type,
                source_agent.clone(),
                confidence,
                None,
                caller_id,
            )?;

            // Auto-append to active focus session (sawtooth pattern)
            crate::focus::focus_append(&conn, &source_agent, decision);

            Ok(entry)
        }

        "cortex_health" => {
            let conn = state.db.lock().await;

            let memories: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
                .unwrap_or(0);
            let decisions: i64 = conn
                .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
                .unwrap_or(0);
            let embeddings: i64 = conn
                .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
                .unwrap_or(0);
            let events: i64 = conn
                .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
                .unwrap_or(0);

            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();

            Ok(json!({
                "status": "ok",
                "stats": {
                    "memories": memories,
                    "decisions": decisions,
                    "embeddings": embeddings,
                    "events": events,
                    "home": home
                }
            }))
        }

        "cortex_digest" => {
            let conn = state.db.lock().await;
            build_digest(&conn)
        }

        "cortex_unfold" => {
            let sources: Vec<String> = match args.get("sources") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                Some(Value::String(s)) => s
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect(),
                _ => {
                    return Err(
                        "Missing required argument: sources (array of source strings)".to_string(),
                    )
                }
            };
            if sources.is_empty() {
                return Err("sources array is empty".to_string());
            }
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("mcp");
            let ctx = RecallContext::from_caller(caller_id, state);
            let conn = state.db.lock().await;
            let mut results: Vec<Value> = Vec::new();
            let mut total_tokens = 0usize;
            let mut found_sources: Vec<String> = Vec::new();
            for source in &sources {
                // Crystal unfold: expand to member sources
                if source.starts_with("crystal::") {
                    if let Some(id_str) = source.split("::").nth(1) {
                        if let Ok(crystal_id) = id_str.parse::<i64>() {
                            let members = crate::crystallize::unfold_crystal(&conn, crystal_id);
                            let crystal_text = conn
                                .query_row(
                                    "SELECT consolidated_text FROM memory_clusters WHERE id = ?1",
                                    rusqlite::params![crystal_id],
                                    |row| row.get::<_, String>(0),
                                )
                                .unwrap_or_default();
                            let tokens = estimate_tokens(&crystal_text);
                            total_tokens += tokens;
                            found_sources.push(source.clone());
                            results.push(json!({
                                "source": source,
                                "text": crystal_text,
                                "type": "crystal",
                                "tokens": tokens,
                                "members": members,
                            }));
                            continue;
                        }
                    }
                }
                if let Some(item) = unfold_source(&conn, source, &ctx) {
                    let tokens = estimate_tokens(item["text"].as_str().unwrap_or(""));
                    total_tokens += tokens;
                    found_sources.push(source.clone());
                    results.push(json!({
                        "source": source,
                        "text": item["text"],
                        "type": item["type"],
                        "tokens": tokens,
                    }));
                } else {
                    results.push(json!({
                        "source": source,
                        "text": null,
                        "type": "not_found",
                        "tokens": 0,
                    }));
                }
            }

            // Implicit positive feedback: unfolding = "this result was useful"
            if !found_sources.is_empty() {
                super::feedback::record_unfold_feedback(
                    &conn,
                    &found_sources,
                    agent,
                    state.embedding_engine.as_deref(),
                    None,
                );
            }

            Ok(json!({
                "results": results,
                "totalTokens": total_tokens,
                "count": results.iter().filter(|r| r["type"] != "not_found").count(),
                "feedbackRecorded": found_sources.len(),
            }))
        }

        "cortex_forget" => {
            let keyword = args
                .get("source")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: source".to_string())?;
            let mut conn = state.db.lock().await;
            let affected = forget_keyword(&mut conn, keyword)?;
            Ok(json!({ "affected": affected }))
        }

        "cortex_resolve" => {
            let keep_id = args
                .get("keepId")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "Missing required argument: keepId".to_string())?;
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: action".to_string())?;
            let superseded_id = args.get("supersededId").and_then(|v| v.as_i64());
            let mut conn = state.db.lock().await;
            resolve_decision(&mut conn, keep_id, action, superseded_id)?;
            Ok(json!({ "resolved": true }))
        }

        "cortex_focus_start" => {
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: label".to_string())?;
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("mcp");
            let conn = state.db.lock().await;
            crate::focus::focus_start(&conn, label, agent)
        }

        "cortex_focus_end" => {
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: label".to_string())?;
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("mcp");
            let conn = state.db.lock().await;
            crate::focus::focus_end(&conn, label, agent, caller_id)
        }

        "cortex_focus_status" => {
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("mcp");
            let conn = state.db.lock().await;

            let current = crate::focus::focus_current(&conn, agent);

            // Recent closed sessions
            let mut recent: Vec<Value> = Vec::new();
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, label, summary, tokens_before, tokens_after, started_at, ended_at \
                 FROM focus_sessions WHERE agent = ?1 AND status = 'closed' \
                 ORDER BY ended_at DESC LIMIT 5",
            ) {
                if let Ok(rows) = stmt.query_map(rusqlite::params![agent], |row| {
                    Ok(json!({
                        "id": row.get::<_, i64>(0)?,
                        "label": row.get::<_, String>(1)?,
                        "summary": row.get::<_, Option<String>>(2)?,
                        "tokensBefore": row.get::<_, Option<i64>>(3)?,
                        "tokensAfter": row.get::<_, Option<i64>>(4)?,
                        "startedAt": row.get::<_, String>(5)?,
                        "endedAt": row.get::<_, Option<String>>(6)?
                    }))
                }) {
                    for row in rows.flatten() {
                        recent.push(row);
                    }
                }
            }

            Ok(json!({
                "active": current,
                "recent": recent,
                "count": recent.len()
            }))
        }

        "cortex_diary" => {
            use std::fs;

            let state_path = state.home.join(".claude").join("state.md");

            if let Some(parent) = state_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {e}"))?;
            }

            let existing = fs::read_to_string(&state_path).unwrap_or_default();
            let permanent = extract_permanent_sections(&existing);
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

            let mut lines: Vec<String> = vec![format!("# Session State — {today}"), String::new()];

            if !permanent.is_empty() {
                lines.push("## DO NOT REMOVE".to_string());
                lines.push(permanent);
                lines.push(String::new());
            }

            if let Some(text) = args.get("accomplished").and_then(|v| v.as_str()) {
                let safe = sanitize_markdown(text);
                if !safe.is_empty() {
                    lines.push("## What Was Done This Session".to_string());
                    lines.push(safe);
                    lines.push(String::new());
                }
            }

            let next_steps = args
                .get("nextSteps")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("next_steps").and_then(|v| v.as_str()));
            if let Some(text) = next_steps {
                let safe = sanitize_markdown(text);
                if !safe.is_empty() {
                    lines.push("## Next Session".to_string());
                    lines.push(safe);
                    lines.push(String::new());
                }
            }

            if let Some(text) = args.get("pending").and_then(|v| v.as_str()) {
                let safe = sanitize_markdown(text);
                if !safe.is_empty() {
                    lines.push("## Pending".to_string());
                    lines.push(safe);
                    lines.push(String::new());
                }
            }

            let known_issues = args
                .get("knownIssues")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("known_issues").and_then(|v| v.as_str()));
            if let Some(text) = known_issues {
                let safe = sanitize_markdown(text);
                if !safe.is_empty() {
                    lines.push("## Known Issues".to_string());
                    lines.push(safe);
                    lines.push(String::new());
                }
            }

            let decisions_text = args
                .get("decisions")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("keyDecisions").and_then(|v| v.as_str()));
            if let Some(text) = decisions_text {
                let safe = sanitize_markdown(text);
                if !safe.is_empty() {
                    lines.push("## Key Decisions".to_string());
                    lines.push(safe);
                    lines.push(String::new());
                }
            } else {
                let existing_decisions = extract_section(&existing, "## Key Decisions");
                if let Some(content) = existing_decisions {
                    lines.push("## Key Decisions".to_string());
                    lines.push(content);
                    lines.push(String::new());
                }
            }

            let content = lines.join("\n");
            fs::write(&state_path, &content)
                .map_err(|e| format!("Failed to write state.md: {e}"))?;

            Ok(json!({ "written": true }))
        }

        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

// ─── Main MCP message handler ─────────────────────────────────────────────────

pub async fn handle_mcp_message_with_caller(
    state: &RuntimeState,
    msg: &Value,
    caller_id: Option<i64>,
) -> Option<Value> {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let method = msg
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    // Validate JSON-RPC version
    if let Some(ver) = msg.get("jsonrpc").and_then(|v| v.as_str()) {
        if ver != "2.0" {
            if msg.get("id").is_some() {
                return Some(mcp_error(id, -32600, "Invalid JSON-RPC version"));
            }
            return None;
        }
    }

    match method {
        "initialize" => Some(mcp_success(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": "cortex", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),

        "notifications/initialized" => None,

        "tools/list" => Some(mcp_success(id, json!({ "tools": mcp_tools() }))),

        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            if tool_name.is_empty() {
                return Some(mcp_error(id, -32602, "Missing tool name"));
            }

            let known = mcp_tools().iter().any(|tool| {
                tool.get("name")
                    .and_then(|v| v.as_str())
                    .map(|name| name == tool_name)
                    .unwrap_or(false)
            });
            if !known {
                return Some(mcp_error(id, -32601, &format!("Unknown tool: {tool_name}")));
            }

            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match mcp_dispatch(state, caller_id, tool_name, &args).await {
                Ok(result) => {
                    let wrapped = if tool_name == "cortex_health" || tool_name == "cortex_digest" {
                        wrap_mcp_tool_result_verbose(state, result)
                    } else {
                        wrap_mcp_tool_result(state, result)
                    };
                    Some(mcp_success(id, wrapped))
                }
                Err(err) => Some(mcp_success(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": json!({"error": err}).to_string()
                        }],
                        "isError": true
                    }),
                )),
            }
        }

        _ => {
            if msg.get("id").is_some() {
                Some(mcp_error(
                    id,
                    -32601,
                    &format!("Method not found: {method}"),
                ))
            } else {
                None
            }
        }
    }
}

// ─── Diary helpers (duplicated from handlers/diary.rs to avoid pub re-export) ─

fn extract_permanent_sections(content: &str) -> String {
    extract_section(content, "## DO NOT REMOVE").unwrap_or_default()
}

fn extract_section(content: &str, header: &str) -> Option<String> {
    let idx = content.find(header)?;
    let start = idx + header.len();
    let rest = &content[start..];
    let end = rest.find("\n## ").map(|i| i + 1).unwrap_or(rest.len());
    let text = rest[..end].trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn sanitize_markdown(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3 {
                return line.to_string();
            }
            if trimmed.starts_with("##") {
                return format!("<!-- {line} -->");
            }
            line.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

