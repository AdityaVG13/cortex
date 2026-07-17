// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;
use crate::handlers::diary::{write_diary_entry, DiaryRequest};
use crate::handlers::feedback::{build_agent_feedback_stats_payload, recommend_recall_k, record_agent_feedback_from_value};
use crate::handlers::health::{build_digest, build_health_payload};
use crate::handlers::mutate::{forget_keyword_scoped, list_conflicts_payload, parse_conflict_id, resolve_decision, resolve_decision_with_metadata, ConflictListOptions, ConflictStatusFilter, ResolutionMetadata};
use crate::handlers::recall::{execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall, parse_recall_policy_mode, resolve_recall_budget_k, unfold_source, RecallContext};
use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding_and_provenance_retention, validate_explicit_ttl_seconds, DecisionProvenance};
use crate::handlers::{estimate_tokens, now_iso, SourceIdentity};
use crate::api_types::RetentionClass;
use crate::state::RuntimeState;
use crate::{aging, db, indexer};

use super::*;
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
            "name": "cortex_boot_audit",
            "description": "Read recent boot audit rows recorded by /boot and cortex_boot. Use to inspect which boot prompts were served and their token/capsule metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Optional exact agent filter." },
                    "limit": { "type": "number", "description": "Maximum rows to return (default 50, max 500)." }
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
            "description": "Search Cortex brain for memories and decisions. Supports policy modes (fast, balanced, deep) and fail-closed recall latency budgets.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "budget": { "type": "number", "description": "Token budget. If omitted, policyMode defaults are used (fast/balanced/deep)." },
                    "policyMode": { "type": "string", "description": "Optional retrieval policy mode: fast, balanced, deep, or headlines." },
                    "k": { "type": "number", "description": "Retrieval depth hint (default adapts to resolved policy mode/budget)." },
                    "agent": { "type": "string", "description": "Optional agent id for dedup/predictive cache" },
                    "taskClass": { "type": "string", "description": "Optional task class for adaptive retrieval hints (e.g. debug, refactor, docs)" },
                    "adaptive": { "type": "boolean", "description": "When true, tune k using recent agent/task outcomes from telemetry." }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "cortex_recall_policy_explain",
            "description": "Explain why recall returned specific results: selected policy mode, ranking factors, dropped candidates, and budget reasoning.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "budget": { "type": "number", "description": "Token budget used for recall planning (defaults from policyMode)." },
                    "policyMode": { "type": "string", "description": "Optional retrieval policy mode: fast, balanced, deep, or headlines." },
                    "k": { "type": "number", "description": "Requested result count (default adapts to resolved policy mode/budget)." },
                    "pool_k": { "type": "number", "description": "Candidate pool depth for explain diagnostics (default adaptive, max 128)" },
                    "agent": { "type": "string", "description": "Optional agent id for dedup/predictive cache context" }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "cortex_semantic_recall",
            "description": "Semantic-only recall path that skips keyword fusion. Use when you want pure embedding retrieval.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "budget": { "type": "number", "description": "Token budget for returned excerpts" },
                    "k": { "type": "number", "description": "Maximum results to return (default 10)" },
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
                    "confidence": { "type": "number", "description": "Confidence score 0-1 (default: 0.8)" },
                    "reasoning_depth": { "type": "string", "description": "single-shot | multi-step | tool-assisted | chain-of-thought | user-stated" },
                    "ttl_seconds": { "type": "number", "description": "Explicit TTL in seconds; overrides retention-class default TTL" },
                    "retention_class": { "type": "string", "enum": ["durable", "operational", "audit", "ephemeral"], "description": "Retention policy class; default inferred from type/text" }
                },
                "required": ["decision"]
            }
        }),
        json!({
            "name": "cortex_agent_feedback_record",
            "description": "Record task outcome telemetry for any agent (success/partial/failure, quality, latency, retries, tokens).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Agent identifier (defaults to source agent)" },
                    "taskClass": { "type": "string", "description": "Task class label (default: general)" },
                    "outcome": { "type": "string", "enum": ["success", "partial", "failure"], "description": "Task outcome category" },
                    "outcomeScore": { "type": "number", "description": "Outcome score override in [0,1] (defaults from outcome)" },
                    "qualityScore": { "type": "number", "description": "Quality score in [0,1], default 0.7" },
                    "latencyMs": { "type": "number", "description": "Optional latency in milliseconds" },
                    "retries": { "type": "number", "description": "Optional retry count" },
                    "tokensUsed": { "type": "number", "description": "Optional token usage count for this task" },
                    "memorySources": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional memory/decision source ids used during task execution"
                    },
                    "notes": { "type": "string", "description": "Optional operator note" }
                },
                "required": ["outcome"]
            }
        }),
        json!({
            "name": "cortex_agent_feedback_stats",
            "description": "Summarize reliability trends from recorded agent outcome telemetry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "horizonDays": { "type": "number", "description": "Lookback window in days (default 30, max 180)" },
                    "limit": { "type": "number", "description": "Max rows sampled for stats (default 400, max 2000)" },
                    "taskClass": { "type": "string", "description": "Optional task class filter" },
                    "agent": { "type": "string", "description": "Optional agent filter" }
                }
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
            "name": "cortex_conflicts_list",
            "description": "List conflict records with optional status/classification filters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["open", "resolved", "all"], "description": "Filter by conflict lifecycle status (default: open)" },
                    "classification": { "type": "string", "enum": ["AGREES", "CONTRADICTS", "REFINES", "UNRELATED"], "description": "Optional conflict classification filter" },
                    "conflictId": { "type": "string", "description": "Optional conflict id (decision:<id>:<id>) to filter exact record" },
                    "limit": { "type": "number", "description": "Max records per status bucket (default 100, max 500)" }
                }
            }
        }),
        json!({
            "name": "cortex_conflicts_get",
            "description": "Fetch a single conflict record by id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "conflictId": { "type": "string", "description": "Conflict id in decision:<id>:<id> format" }
                },
                "required": ["conflictId"]
            }
        }),
        json!({
            "name": "cortex_conflicts_resolve",
            "description": "Resolve a conflict by selecting a winner and persisting resolution metadata.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "winnerId": { "type": "number", "description": "Decision id to keep as winner (alias: keepId)" },
                    "keepId": { "type": "number", "description": "Alias for winnerId" },
                    "action": { "type": "string", "enum": ["keep", "merge", "archive"], "description": "Resolution action" },
                    "supersededId": { "type": "number", "description": "Decision id to supersede/archive (alias: loserId)" },
                    "loserId": { "type": "number", "description": "Alias for supersededId" },
                    "conflictId": { "type": "string", "description": "Conflict id (decision:<id>:<id>); used for metadata and loser inference" },
                    "classification": { "type": "string", "enum": ["AGREES", "CONTRADICTS", "REFINES", "UNRELATED"], "description": "Final classification override" },
                    "similarity": { "type": "number", "description": "Optional similarity score snapshot for auditability" },
                    "notes": { "type": "string", "description": "Optional operator note for why this resolution was chosen" },
                    "resolvedBy": { "type": "string", "description": "Optional resolver identity (defaults to source agent)" }
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "cortex_consensus_promote",
            "description": "Auto-resolve open disputed decision pairs when trust margin is high enough. Uses trustScore/confidence winner selection.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "number", "description": "Max open conflicts to scan (default 50, max 500)" },
                    "minMargin": { "type": "number", "description": "Minimum trust margin required to auto-promote (default 0.1, range 0-1)" },
                    "dryRun": { "type": "boolean", "description": "When true, report candidates only and do not mutate decisions" }
                }
            }
        }),
        json!({
            "name": "cortex_memory_decay_run",
            "description": "Run one explicit maintenance pass: decay scores, optional aging compression/archive, and optional expired-row cleanup.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "includeAging": { "type": "boolean", "description": "Run aging pass after score decay (default true)" },
                    "cleanupExpired": { "type": "boolean", "description": "Delete expired memory/decision rows (default true)" }
                }
            }
        }),
        json!({
            "name": "cortex_eval_run",
            "description": "Generate a local evaluation snapshot over conflict pressure and resolution throughput for the selected horizon.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "horizonDays": { "type": "number", "description": "Lookback window in days for event-based metrics (default 30, range 1-180)" }
                }
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
        json!({
            "name": "cortex_permissions_list",
            "description": "List MCP client permission grants for the current owner scope.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "cortex_permissions_grant",
            "description": "Grant a client permission (`read`, `write`, `admin`) for a scope (`*` by default).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "client": { "type": "string", "description": "Client id or '*' wildcard" },
                    "permission": { "type": "string", "enum": ["read", "write", "admin"], "description": "Permission level" },
                    "scope": { "type": "string", "description": "Scope key (default '*', tool-name scopes supported)" }
                },
                "required": ["client", "permission"]
            }
        }),
        json!({
            "name": "cortex_permissions_revoke",
            "description": "Revoke a previously granted client permission for a scope.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "client": { "type": "string", "description": "Client id or '*' wildcard" },
                    "permission": { "type": "string", "enum": ["read", "write", "admin"], "description": "Permission level" },
                    "scope": { "type": "string", "description": "Scope key (default '*')" }
                },
                "required": ["client", "permission"]
            }
        }),
        json!({
            "name": "cortex_lastCall",
            "description": "Fetch the latest memory, decision, or event added to Cortex, with optional kind/agent filters.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "description": "Filter by kind: any, memory, decision, or event" },
                    "agent": { "type": "string", "description": "Optional source agent filter" }
                }
            }
        }),
        json!({
            "name": "cortex_reconnect",
            "description": "Re-register this MCP agent session after a daemon restart or transient disconnect. Safe to call mid-session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "agent": { "type": "string", "description": "Agent display name (default: mcp)" },
                    "model": { "type": "string", "description": "Optional model label to append, e.g. '5.3 Codex Extra High'" }
                }
            }
        }),
    ]
}

