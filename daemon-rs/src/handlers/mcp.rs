use serde_json::{json, Value};

use crate::state::RuntimeState;
use super::{now_iso, estimate_tokens};
use super::store::store_decision;
use super::recall::execute_unified_recall;
use super::mutate::{forget_keyword, resolve_decision};
use super::health::build_digest;

// ─── JSON-RPC helpers ─────────────────────────────────────────────────────────

pub fn mcp_success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn mcp_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn wrap_mcp_tool_result(state: &RuntimeState, data: Value) -> Value {
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
            "description": "Check Cortex system health: DB stats, Ollama status, memory counts.",
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

async fn mcp_dispatch(state: &RuntimeState, tool_name: &str, args: &Value) -> Result<Value, String> {
    match tool_name {
        "cortex_boot" => {
            let profile = args.get("profile").and_then(|v| v.as_str()).map(str::to_string);
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

            let memory_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let decision_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            let identity_text =
                "User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits.";
            let assembled = format!(
                "## Identity\n{identity_text}\n\n## Stats\nMemories: {memory_count} | Decisions: {decision_count}\n\n## Note\nRust daemon boot — full compiler coming in Task 7."
            );
            let token_estimate = estimate_tokens(&assembled);

            let boot_ts = now_iso();
            let _ = conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "agent_boot",
                    serde_json::to_string(&json!({"timestamp": boot_ts, "agent": agent.clone()}))
                        .unwrap_or_default(),
                    agent.clone()
                ],
            );

            state.emit(
                "agent_boot",
                json!({"agent": agent.clone(), "profile": profile_str.clone()}),
            );

            Ok(json!({
                "bootPrompt": assembled,
                "tokenEstimate": token_estimate,
                "profile": if profile_str == "full" { "capsules" } else { &profile_str },
                "capsules": [
                    { "name": "identity", "tokens": estimate_tokens(identity_text), "freshness": "stable", "truncated": false }
                ]
            }))
        }

        "cortex_peek" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let results = execute_unified_recall(state, query, 0, limit, "mcp").await?;
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

            execute_unified_recall(state, query, budget, 10, agent).await
        }

        "cortex_store" => {
            let decision = args
                .get("decision")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: decision".to_string())?;
            let context = args.get("context").and_then(|v| v.as_str()).map(str::to_string);
            let entry_type = args.get("type").and_then(|v| v.as_str()).map(str::to_string);
            let source_agent = args
                .get("source_agent")
                .and_then(|v| v.as_str())
                .unwrap_or("mcp")
                .to_string();
            let confidence = args.get("confidence").and_then(|v| v.as_f64());

            let mut conn = state.db.lock().await;
            let (entry, _id) = store_decision(&mut conn, decision, context, entry_type, source_agent, confidence, None)?;
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
                    "ollama": "offline",
                    "home": home
                }
            }))
        }

        "cortex_digest" => {
            let conn = state.db.lock().await;
            build_digest(&conn)
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

            let mut lines: Vec<String> = vec![
                format!("# Session State — {today}"),
                String::new(),
            ];

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

pub async fn handle_mcp_message(state: &RuntimeState, msg: &Value) -> Option<Value> {
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
                "serverInfo": { "name": "cortex", "version": "2.1.0" }
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

            match mcp_dispatch(state, tool_name, &args).await {
                Ok(result) => Some(mcp_success(id, wrap_mcp_tool_result(state, result))),
                Err(err) => Some(mcp_success(
                    id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": json!({
                                "error": err,
                                "_liveness": true,
                                "_ts": now_iso(),
                                "_calls": state.next_mcp_call()
                            }).to_string()
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
    if text.is_empty() { None } else { Some(text) }
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
