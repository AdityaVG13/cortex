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
use super::arg_str;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientPermission {
    Read,
    Write,
    Admin,
}

impl ClientPermission {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ClientPermission::Read => "read",
            ClientPermission::Write => "write",
            ClientPermission::Admin => "admin",
        }
    }
}

pub(crate) fn parse_client_permission(raw: &str) -> Option<ClientPermission> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "read" => Some(ClientPermission::Read),
        "write" => Some(ClientPermission::Write),
        "admin" => Some(ClientPermission::Admin),
        _ => None,
    }
}

pub(crate) fn required_permission_for_tool(tool_name: &str) -> Option<ClientPermission> {
    match tool_name {
        "cortex_boot"
        | "cortex_boot_audit"
        | "cortex_reconnect"
        | "cortex_peek"
        | "cortex_recall"
        | "cortex_recall_policy_explain"
        | "cortex_semantic_recall"
        | "cortex_agent_feedback_stats"
        | "cortex_health"
        | "cortex_digest"
        | "cortex_unfold"
        | "cortex_focus_status"
        | "cortex_lastCall" => Some(ClientPermission::Read),
        "cortex_store"
        | "cortex_agent_feedback_record"
        | "cortex_focus_start"
        | "cortex_focus_end"
        | "cortex_diary" => Some(ClientPermission::Write),
        "cortex_forget"
        | "cortex_resolve"
        | "cortex_conflicts_list"
        | "cortex_conflicts_get"
        | "cortex_conflicts_resolve"
        | "cortex_permissions_list"
        | "cortex_permissions_grant"
        | "cortex_permissions_revoke"
        | "cortex_consensus_promote"
        | "cortex_memory_decay_run"
        | "cortex_eval_run" => Some(ClientPermission::Admin),
        _ => None,
    }
}

pub(crate) fn normalize_permission_client_id(raw: &str) -> String {
    let before_model = raw
        .split('(')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    let normalized: String = before_model
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if normalized.is_empty() {
        "mcp".to_string()
    } else {
        normalized
    }
}

pub(crate) fn source_client_for_permissions(source: Option<&SourceIdentity>, args: &Value) -> String {
    let raw = source
        .map(|identity| identity.agent.as_str())
        .or_else(|| arg_str(args, &["source_agent", "agent"]))
        .unwrap_or("mcp");
    normalize_permission_client_id(raw)
}

pub(crate) fn permission_satisfies(granted: &str, required: ClientPermission) -> bool {
    match required {
        ClientPermission::Read => matches!(granted, "read" | "write" | "admin"),
        ClientPermission::Write => matches!(granted, "write" | "admin"),
        ClientPermission::Admin => granted == "admin",
    }
}

pub(crate) fn has_client_permission(
    conn: &rusqlite::Connection,
    owner_id: i64,
    client_id: &str,
    scope: &str,
    required: ClientPermission,
) -> Result<bool, String> {
    let configured_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM client_permissions WHERE owner_id = ?1",
            rusqlite::params![owner_id],
            |row| row.get(0),
        )
        .map_err(|err| err.to_string())?;

    // Backward-compatible baseline: no policy rows means permissive mode.
    if configured_rows == 0 {
        return Ok(true);
    }

    let mut stmt = conn
        .prepare(
            "SELECT permission FROM client_permissions
             WHERE owner_id = ?1
               AND (client_id = ?2 OR client_id = '*')
               AND (scope = ?3 OR scope = '*')",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![owner_id, client_id, scope], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| err.to_string())?;

    for granted in rows.flatten() {
        if permission_satisfies(granted.trim(), required) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(crate) fn caller_has_team_admin_role(conn: &rusqlite::Connection, caller_id: i64) -> Result<bool, String> {
    let role = conn
        .query_row(
            "SELECT role FROM users WHERE id = ?1",
            rusqlite::params![caller_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?;

    Ok(matches!(role.as_deref(), Some("owner" | "admin")))
}

pub(crate) async fn enforce_client_permission(
    state: &RuntimeState,
    caller_id: Option<i64>,
    tool_name: &str,
    args: &Value,
    source: Option<&SourceIdentity>,
) -> Result<(), String> {
    let Some(required) = required_permission_for_tool(tool_name) else {
        return Ok(());
    };
    let owner_id = if state.team_mode {
        caller_id.unwrap_or_default()
    } else {
        0
    };
    let client_id = source_client_for_permissions(source, args);

    let conn = state.db.lock().await;
    if state.team_mode
        && required == ClientPermission::Admin
        && !caller_has_team_admin_role(&conn, owner_id)?
    {
        return Err(format!(
            "Permission denied: team admin role required for '{tool_name}'"
        ));
    }

    let allowed = has_client_permission(&conn, owner_id, &client_id, tool_name, required)?;
    drop(conn);

    if allowed {
        return Ok(());
    }

    Err(format!(
        "Permission denied: client '{client_id}' lacks '{}' permission for '{tool_name}'",
        required.as_str()
    ))
}

pub(crate) fn source_agent_for_tool(source: Option<&SourceIdentity>, fallback: &str) -> String {
    source
        .map(|identity| identity.agent.clone())
        .unwrap_or_else(|| fallback.to_string())
}

pub(crate) fn source_model_for_tool<'a>(
    source: Option<&'a SourceIdentity>,
    args: &'a Value,
) -> Option<&'a str> {
    source
        .and_then(|identity| identity.model.as_deref())
        .or_else(|| arg_str(args, &["model"]))
}

pub(crate) fn normalize_mcp_agent_label(raw_agent: &str, model: Option<&str>) -> Result<String, String> {
    let mut agent = raw_agent.trim().to_string();
    if agent.is_empty() {
        return Err("Missing required argument: agent".to_string());
    }
    if agent.len() > 160 || agent.chars().any(|ch| ch.is_control()) {
        return Err("Invalid agent label".to_string());
    }
    if !agent.contains('(') {
        if let Some(model_name) = model.map(str::trim).filter(|m| !m.is_empty()) {
            if agent.eq_ignore_ascii_case("droid") {
                agent = format!("DROID ({model_name})");
            } else {
                agent = format!("{agent} ({model_name})");
            }
        }
    }
    if agent.len() > 160 || agent.chars().any(|ch| ch.is_control()) {
        return Err("Invalid agent label".to_string());
    }
    Ok(agent)
}

pub(crate) fn mcp_session_description(description_prefix: &str, model: Option<&str>) -> String {
    model
        .map(|model_name| format!("{description_prefix} · {model_name}"))
        .unwrap_or_else(|| description_prefix.to_string())
}

pub(crate) fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

pub(crate) fn resolve_refresh_presence_agent(
    conn: &rusqlite::Connection,
    owner_id: Option<i64>,
    raw_agent: &str,
    model: Option<&str>,
    normalized_agent: &str,
) -> Result<String, String> {
    let trimmed_agent = raw_agent.trim();
    if model.is_some() || trimmed_agent.contains('(') {
        return Ok(normalized_agent.to_string());
    }

    let modeled_pattern = format!("{} (%)", escape_like_pattern(trimmed_agent));
    let sql_with_owner = "SELECT agent
         FROM sessions
         WHERE owner_id = ?1 AND (agent = ?2 OR agent LIKE ?3 ESCAPE '\\')
         ORDER BY
             CASE WHEN expires_at IS NULL OR expires_at > datetime('now') THEN 0 ELSE 1 END,
             CASE WHEN agent LIKE ?3 ESCAPE '\\' THEN 0 ELSE 1 END,
             COALESCE(last_heartbeat, started_at) DESC
         LIMIT 1";
    let sql_solo = "SELECT agent
         FROM sessions
         WHERE agent = ?1 OR agent LIKE ?2 ESCAPE '\\'
         ORDER BY
             CASE WHEN expires_at IS NULL OR expires_at > datetime('now') THEN 0 ELSE 1 END,
             CASE WHEN agent LIKE ?2 ESCAPE '\\' THEN 0 ELSE 1 END,
             COALESCE(last_heartbeat, started_at) DESC
         LIMIT 1";

    let existing_agent = if let Some(owner_id) = owner_id {
        conn.query_row(
            sql_with_owner,
            rusqlite::params![owner_id, trimmed_agent, modeled_pattern],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?
    } else {
        conn.query_row(
            sql_solo,
            rusqlite::params![trimmed_agent, modeled_pattern],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| err.to_string())?
    };

    Ok(existing_agent.unwrap_or_else(|| normalized_agent.to_string()))
}

pub(crate) fn mcp_session_owner_id(
    state: &RuntimeState,
    caller_id: Option<i64>,
) -> Result<Option<i64>, String> {
    if state.team_mode {
        let caller_id = caller_id.ok_or_else(|| {
            "Team mode requires a caller-scoped API key for MCP session operations".to_string()
        })?;
        Ok(Some(caller_id))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpPresenceDisposition {
    Existing,
    Started,
}

pub(crate) async fn refresh_mcp_session_presence(
    state: &RuntimeState,
    caller_id: Option<i64>,
    raw_agent: &str,
    model: Option<&str>,
    description_prefix: &str,
) -> Result<(String, String, McpPresenceDisposition), String> {
    let normalized_agent = normalize_mcp_agent_label(raw_agent, model)?;
    let owner_id = mcp_session_owner_id(state, caller_id)?;
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let description = mcp_session_description(description_prefix, model);

    let conn = state.db.lock().await;
    let agent =
        resolve_refresh_presence_agent(&conn, owner_id, raw_agent, model, &normalized_agent)?;
    let disposition = if let Some(owner_id) = owner_id {
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET last_heartbeat = ?1,
                     expires_at = ?2,
                     description = CASE
                         WHEN description IS NULL OR trim(description) = '' THEN ?3
                         ELSE description
                     END
                 WHERE owner_id = ?4 AND agent = ?5",
                rusqlite::params![now, expires_at, description, owner_id, agent],
            )
            .map_err(|e| e.to_string())?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
                 VALUES (?1, ?2, ?3, 'mcp', '[]', ?4, ?5, ?5, ?6)",
                rusqlite::params![agent, owner_id, session_id, description, now, expires_at],
            )
            .map_err(|e| e.to_string())?;
            McpPresenceDisposition::Started
        } else {
            McpPresenceDisposition::Existing
        }
    } else {
        let updated = conn
            .execute(
                "UPDATE sessions
                 SET last_heartbeat = ?1,
                     expires_at = ?2,
                     description = CASE
                         WHEN description IS NULL OR trim(description) = '' THEN ?3
                         ELSE description
                     END
                 WHERE agent = ?4",
                rusqlite::params![now, expires_at, description, agent],
            )
            .map_err(|e| e.to_string())?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
                 VALUES (?1, ?2, 'mcp', '[]', ?3, ?4, ?4, ?5)",
                rusqlite::params![agent, session_id, description, now, expires_at],
            )
            .map_err(|e| e.to_string())?;
            McpPresenceDisposition::Started
        } else {
            McpPresenceDisposition::Existing
        }
    };

    crate::db::checkpoint_wal_best_effort(&conn);
    Ok((agent, expires_at, disposition))
}
