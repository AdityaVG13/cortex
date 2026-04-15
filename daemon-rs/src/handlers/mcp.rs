// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::diary::{write_diary_entry, DiaryRequest};
use super::feedback::{build_agent_feedback_stats_payload, record_agent_feedback_from_value};
use super::health::{build_digest, build_health_payload};
use super::mutate::{
    forget_keyword, list_conflicts_payload, parse_conflict_id, resolve_decision,
    resolve_decision_with_metadata, ConflictListOptions, ConflictStatusFilter, ResolutionMetadata,
};
use super::recall::{
    execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall, unfold_source,
    RecallContext,
};
use super::store::{
    persist_decision_embedding, store_decision_with_input_embedding_and_provenance,
    DecisionProvenance,
};
use super::{estimate_tokens, now_iso, SourceIdentity};
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

fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn arg_f64(args: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_f64()))
}

fn arg_i64(args: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_i64()))
}

fn arg_usize(args: &Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_u64()))
        .map(|value| value as usize)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientPermission {
    Read,
    Write,
    Admin,
}

impl ClientPermission {
    fn as_str(self) -> &'static str {
        match self {
            ClientPermission::Read => "read",
            ClientPermission::Write => "write",
            ClientPermission::Admin => "admin",
        }
    }
}

fn parse_client_permission(raw: &str) -> Option<ClientPermission> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "read" => Some(ClientPermission::Read),
        "write" => Some(ClientPermission::Write),
        "admin" => Some(ClientPermission::Admin),
        _ => None,
    }
}

fn required_permission_for_tool(tool_name: &str) -> Option<ClientPermission> {
    match tool_name {
        "cortex_boot"
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
        | "cortex_permissions_revoke" => Some(ClientPermission::Admin),
        _ => None,
    }
}

fn normalize_permission_client_id(raw: &str) -> String {
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

fn source_client_for_permissions(source: Option<&SourceIdentity>, args: &Value) -> String {
    let raw = source
        .map(|identity| identity.agent.as_str())
        .or_else(|| arg_str(args, &["source_agent", "agent"]))
        .unwrap_or("mcp");
    normalize_permission_client_id(raw)
}

fn permission_satisfies(granted: &str, required: ClientPermission) -> bool {
    match required {
        ClientPermission::Read => matches!(granted, "read" | "write" | "admin"),
        ClientPermission::Write => matches!(granted, "write" | "admin"),
        ClientPermission::Admin => granted == "admin",
    }
}

fn has_client_permission(
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

async fn enforce_client_permission(
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

fn source_agent_for_tool(source: Option<&SourceIdentity>, fallback: &str) -> String {
    source
        .map(|identity| identity.agent.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn source_model_for_tool<'a>(
    source: Option<&'a SourceIdentity>,
    args: &'a Value,
) -> Option<&'a str> {
    source
        .and_then(|identity| identity.model.as_deref())
        .or_else(|| arg_str(args, &["model"]))
}

fn recall_owner_scope(ctx: &RecallContext) -> String {
    if !ctx.team_mode {
        return "solo".to_string();
    }
    match ctx.caller_id {
        Some(owner_id) => format!("team:{owner_id}"),
        None => "team:none".to_string(),
    }
}

async fn clear_served_scope_for_boot(state: &RuntimeState, agent: &str, ctx: &RecallContext) {
    let scope_prefix = format!("{}::{agent}::", recall_owner_scope(ctx));
    let mut served = state.served_content.lock().await;
    served.retain(|key, _| {
        !key.starts_with(&scope_prefix) && !key.starts_with(&format!("{agent}::")) && key != agent
    });
}

fn can_view_last_call(
    owner_id: Option<i64>,
    visibility: Option<&str>,
    ctx: &RecallContext,
) -> bool {
    if !ctx.team_mode {
        return true;
    }
    let Some(caller_id) = ctx.caller_id else {
        return false;
    };
    let Some(owner_id) = owner_id else {
        return false;
    };
    owner_id == caller_id || matches!(visibility, Some("shared") | Some("team"))
}

fn table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = match conn.prepare(&pragma) {
        Ok(stmt) => stmt,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    let found = rows.flatten().any(|name| name == column);
    drop(stmt);
    found
}

fn fetch_last_call(
    conn: &rusqlite::Connection,
    kind: Option<&str>,
    agent_filter: Option<&str>,
    ctx: &RecallContext,
) -> Result<Value, String> {
    let normalized_kind = kind
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("any");
    let agent_filter = agent_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase);

    let owner_scoped_entries = table_has_column(conn, "memories", "owner_id")
        && table_has_column(conn, "memories", "visibility")
        && table_has_column(conn, "decisions", "owner_id")
        && table_has_column(conn, "decisions", "visibility");

    let sql = if owner_scoped_entries {
        "
            SELECT kind, id, created_at, source_agent, summary, detail, owner_id, visibility
            FROM (
              SELECT 'memory' AS kind, id, created_at, source_agent,
                     substr(text, 1, 240) AS summary,
                     json_object('text', text, 'source', source, 'type', type) AS detail,
                     owner_id, visibility
              FROM memories
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     owner_id, visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
            ORDER BY CAST(strftime('%s', created_at) AS INTEGER) DESC, id DESC
            LIMIT 32
        "
    } else {
        "
            SELECT kind, id, created_at, source_agent, summary, detail, owner_id, visibility
            FROM (
              SELECT 'memory' AS kind, id, created_at, source_agent,
                     substr(text, 1, 240) AS summary,
                     json_object('text', text, 'source', source, 'type', type) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM memories
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
            ORDER BY CAST(strftime('%s', created_at) AS INTEGER) DESC, id DESC
            LIMIT 32
        "
    };

    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![normalized_kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|err| err.to_string())?;

    for row in rows.flatten() {
        let (row_kind, id, created_at, source_agent, summary, detail, owner_id, visibility) = row;
        if let Some(filter) = agent_filter.as_deref() {
            let current = source_agent
                .as_deref()
                .map(str::to_lowercase)
                .unwrap_or_default();
            if current != filter {
                continue;
            }
        }
        if row_kind != "event" && !can_view_last_call(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        return Ok(json!({
            "found": true,
            "kind": row_kind,
            "id": id,
            "createdAt": created_at,
            "sourceAgent": source_agent,
            "summary": summary,
            "detail": serde_json::from_str::<Value>(&detail).unwrap_or(Value::String(detail)),
        }));
    }

    Ok(json!({ "found": false }))
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::{
        fetch_last_call, has_client_permission, mcp_dispatch, normalize_permission_client_id,
        required_permission_for_tool, ClientPermission,
    };
    use crate::db;
    use crate::handlers::recall::RecallContext;
    use crate::handlers::SourceIdentity;
    use crate::state::RuntimeState;
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::configure(&conn).unwrap();
        db::initialize_schema(&conn).unwrap();
        db::run_pending_migrations(&conn);
        conn
    }

    fn test_state() -> RuntimeState {
        let write_conn = test_conn();
        let read_conn = test_conn();
        let (events, _) = broadcast::channel(8);
        RuntimeState {
            db: Arc::new(Mutex::new(write_conn)),
            db_read: Arc::new(Mutex::new(read_conn)),
            token: Arc::new("test-token".to_string()),
            events,
            mcp_calls: Arc::new(AtomicU64::new(0)),
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            recall_history: Arc::new(Mutex::new(HashMap::new())),
            pre_cache: Arc::new(Mutex::new(HashMap::new())),
            served_content: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            home: PathBuf::from("."),
            db_path: PathBuf::from(":memory:"),
            token_path: PathBuf::from("cortex.token"),
            pid_path: PathBuf::from("cortex.pid"),
            port: 7437,
            embedding_engine: None,
            rate_limiter: crate::rate_limit::RateLimiter::new(),
            team_mode: false,
            default_owner_id: None,
            team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
            degraded_mode: Arc::new(AtomicBool::new(false)),
            db_corrupted: Arc::new(AtomicBool::new(false)),
            readiness: Arc::new(AtomicBool::new(true)),
            write_buffer_path: PathBuf::from("write_buffer.jsonl"),
        }
    }

    async fn seed_disputed_pair(state: &RuntimeState) -> (i64, i64) {
        let conn = state.db.lock().await;
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'claude', 'claude', 0.71, 0.73, 'active')",
            rusqlite::params!["Use sqlite for local projects", "storage policy"],
        )
        .unwrap();
        let first = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'codex', 'codex', 0.93, 0.95, 'active')",
            rusqlite::params!["Use postgres for production workloads", "storage policy"],
        )
        .unwrap();
        let second = conn.last_insert_rowid();

        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            rusqlite::params![second, first],
        )
        .unwrap();
        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            rusqlite::params![first, second],
        )
        .unwrap();
        (first, second)
    }

    #[test]
    fn fetch_last_call_supports_solo_schema_without_owner_columns() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, created_at)
             VALUES (?1, ?2, ?3, 'active', datetime('now'))",
            rusqlite::params!["semantic recall added", "thread focus", "codex"],
        )
        .unwrap();

        let payload =
            fetch_last_call(&conn, Some("decision"), None, &RecallContext::solo()).unwrap();

        assert_eq!(payload["found"].as_bool(), Some(true));
        assert_eq!(payload["kind"].as_str(), Some("decision"));
        assert_eq!(payload["sourceAgent"].as_str(), Some("codex"));
        assert_eq!(
            payload["detail"]["decision"].as_str(),
            Some("semantic recall added")
        );
    }

    #[test]
    fn normalize_permission_client_id_strips_model_suffix_and_symbols() {
        assert_eq!(normalize_permission_client_id("Codex (gpt-5.4)"), "codex");
        assert_eq!(
            normalize_permission_client_id("  Claude Code / Desktop  "),
            "claudecodedesktop"
        );
        assert_eq!(normalize_permission_client_id(""), "mcp");
    }

    #[test]
    fn parse_client_permission_accepts_known_values() {
        assert_eq!(
            super::parse_client_permission("read"),
            Some(ClientPermission::Read)
        );
        assert_eq!(
            super::parse_client_permission("WRITE"),
            Some(ClientPermission::Write)
        );
        assert_eq!(
            super::parse_client_permission(" admin "),
            Some(ClientPermission::Admin)
        );
        assert_eq!(super::parse_client_permission("owner"), None);
    }

    #[test]
    fn client_permission_allows_by_default_when_no_policy_rows_exist() {
        let conn = test_conn();
        let allowed =
            has_client_permission(&conn, 0, "codex", "cortex_store", ClientPermission::Write)
                .unwrap();
        assert!(
            allowed,
            "empty policy table should preserve legacy permissive mode"
        );
    }

    #[test]
    fn client_permission_enforces_explicit_policy_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
             VALUES (0, 'claude', 'read', '*', 'test')",
            [],
        )
        .unwrap();

        let claude_read =
            has_client_permission(&conn, 0, "claude", "cortex_recall", ClientPermission::Read)
                .unwrap();
        let claude_write =
            has_client_permission(&conn, 0, "claude", "cortex_store", ClientPermission::Write)
                .unwrap();
        let codex_read =
            has_client_permission(&conn, 0, "codex", "cortex_recall", ClientPermission::Read)
                .unwrap();

        assert!(claude_read);
        assert!(!claude_write);
        assert!(!codex_read);
    }

    #[test]
    fn client_permission_supports_wildcard_grants() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
             VALUES (42, '*', 'write', 'cortex_store', 'test')",
            [],
        )
        .unwrap();

        let allowed =
            has_client_permission(&conn, 42, "gemini", "cortex_store", ClientPermission::Write)
                .unwrap();
        let denied_admin = has_client_permission(
            &conn,
            42,
            "gemini",
            "cortex_forget",
            ClientPermission::Admin,
        )
        .unwrap();

        assert!(allowed);
        assert!(!denied_admin);
    }

    #[test]
    fn conflict_tools_require_admin_permission_scope() {
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_list"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_get"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_resolve"),
            Some(ClientPermission::Admin)
        );
    }

    #[test]
    fn recall_explain_tool_requires_read_permission_scope() {
        assert_eq!(
            required_permission_for_tool("cortex_recall_policy_explain"),
            Some(ClientPermission::Read)
        );
    }

    #[test]
    fn agent_feedback_tools_require_expected_permission_scopes() {
        assert_eq!(
            required_permission_for_tool("cortex_agent_feedback_record"),
            Some(ClientPermission::Write)
        );
        assert_eq!(
            required_permission_for_tool("cortex_agent_feedback_stats"),
            Some(ClientPermission::Read)
        );
    }

    #[tokio::test]
    async fn conflict_list_denies_non_admin_client_permission() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: None,
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'read', '*', 'test')",
                [],
            )
            .unwrap();
        }

        let result = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_list",
            &json!({"status": "open"}),
            Some(&source),
        )
        .await;

        let err = result.expect_err("list should require admin permission");
        assert!(
            err.contains("Permission denied"),
            "expected permission denied error, got: {err}"
        );
    }

    #[tokio::test]
    async fn conflict_tools_list_and_resolve_success_path() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'admin', '*', 'test')",
                [],
            )
            .unwrap();
        }
        let (first, second) = seed_disputed_pair(&state).await;
        let conflict_id = format!("decision:{}:{}", first.min(second), first.max(second));

        let listed = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_list",
            &json!({"status": "open", "conflictId": conflict_id}),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(listed["count"].as_u64(), Some(1));
        assert_eq!(listed["conflicts"][0]["status"].as_str(), Some("open"));
        assert_eq!(
            listed["conflicts"][0]["classification"].as_str(),
            Some("CONTRADICTS")
        );

        let resolved = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_resolve",
            &json!({
                "conflictId": conflict_id,
                "winnerId": second,
                "action": "keep",
                "classification": "CONTRADICTS",
                "notes": "codex winner",
                "similarity": 0.62
            }),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(resolved["resolved"].as_bool(), Some(true));
        assert_eq!(resolved["winnerId"].as_i64(), Some(second));
        assert_eq!(resolved["supersededId"].as_i64(), Some(first));

        let fetched = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_get",
            &json!({"conflictId": format!("decision:{}:{}", first.min(second), first.max(second))}),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(fetched["found"].as_bool(), Some(true));
        assert_eq!(fetched["conflict"]["status"].as_str(), Some("resolved"));
        assert_eq!(
            fetched["conflict"]["resolution"]["notes"].as_str(),
            Some("codex winner")
        );
    }
}

async fn upsert_mcp_session(
    state: &RuntimeState,
    caller_id: Option<i64>,
    raw_agent: &str,
    model: Option<&str>,
    description_prefix: &str,
) -> Result<(String, String), String> {
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

    let owner_id = if state.team_mode {
        let caller_id = caller_id.ok_or_else(|| {
            "Team mode requires a caller-scoped API key for MCP session operations".to_string()
        })?;
        Some(caller_id)
    } else {
        None
    };
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let description = model
        .map(|m| format!("{description_prefix} · {m}"))
        .unwrap_or_else(|| description_prefix.to_string());

    let conn = state.db.lock().await;
    if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, 'mcp', '[]', ?4, ?5, ?5, ?6)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![agent, owner_id, session_id, description, now, expires_at],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, 'mcp', '[]', ?3, ?4, ?4, ?5)
             ON CONFLICT(agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![agent, session_id, description, now, expires_at],
        )
        .map_err(|e| e.to_string())?;
    }

    crate::db::checkpoint_wal_best_effort(&conn);
    Ok((agent, expires_at))
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
                    "k": { "type": "number", "description": "Retrieval depth hint (default adapts to budget for low-token recall)" },
                    "agent": { "type": "string", "description": "Optional agent id for dedup/predictive cache" }
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
                    "budget": { "type": "number", "description": "Token budget used for recall planning (default 200)" },
                    "k": { "type": "number", "description": "Requested result count (default adapts to budget)" },
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
                    "reasoning_depth": { "type": "string", "description": "single-shot | multi-step | tool-assisted | chain-of-thought | user-stated" }
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

// ─── Dispatch ─────────────────────────────────────────────────────────────────

async fn mcp_dispatch(
    state: &RuntimeState,
    caller_id: Option<i64>,
    tool_name: &str,
    args: &Value,
    source: Option<&SourceIdentity>,
) -> Result<Value, String> {
    if state.team_mode && caller_id.is_none() {
        return Err("Team mode MCP calls require a caller-scoped ctx_ API key".to_string());
    }
    enforce_client_permission(state, caller_id, tool_name, args, source).await?;

    match tool_name {
        "cortex_boot" => {
            let profile = args
                .get("profile")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let raw_agent = arg_str(args, &["agent", "source_agent"])
                .map(str::to_string)
                .unwrap_or_else(|| source_agent_for_tool(source, "mcp"));
            let model = source_model_for_tool(source, args);
            let _budget = args.get("budget").and_then(|v| v.as_u64()).unwrap_or(600) as usize;
            let profile_str = profile.unwrap_or_else(|| "full".to_string());
            let (agent, _expires_at) =
                upsert_mcp_session(state, caller_id, &raw_agent, model, "MCP boot session").await?;
            let ctx = RecallContext::from_caller(caller_id, state);

            // Clear served content for this agent on boot
            clear_served_scope_for_boot(state, &agent, &ctx).await;

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
                    if let Some(owner_id) = ctx.caller_id {
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

        "cortex_reconnect" => {
            let agent = arg_str(args, &["agent"])
                .map(str::to_string)
                .unwrap_or_else(|| source_agent_for_tool(source, "mcp"));
            let model = source_model_for_tool(source, args);
            let (display_agent, expires_at) =
                upsert_mcp_session(state, caller_id, &agent, model, "MCP reconnect").await?;
            state.emit(
                "session",
                json!({"action": "reconnected", "agent": display_agent}),
            );
            Ok(json!({
                "reconnected": true,
                "agent": display_agent,
                "expiresAt": expires_at
            }))
        }

        "cortex_peek" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

            let ctx = RecallContext::from_caller(caller_id, state);
            let results = execute_unified_recall(state, query, 0, limit, "mcp", &ctx, None).await?;
            Ok(results)
        }

        "cortex_recall" => {
            let query = arg_str(args, &["query", "q"])
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let budget = arg_usize(args, &["budget", "b"]).unwrap_or(200);
            let k = arg_usize(args, &["k", "limit"]).unwrap_or({
                if budget <= 220 {
                    16
                } else if budget <= 400 {
                    12
                } else {
                    10
                }
            });
            let agent = arg_str(args, &["agent", "source_agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));

            let ctx = RecallContext::from_caller(caller_id, state);
            execute_unified_recall(state, query, budget, k, agent, &ctx, None).await
        }

        "cortex_recall_policy_explain" => {
            let query = arg_str(args, &["query", "q"])
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let budget = arg_usize(args, &["budget", "b"]).unwrap_or(200);
            let k = arg_usize(args, &["k", "limit"]).unwrap_or({
                if budget <= 220 {
                    16
                } else if budget <= 400 {
                    12
                } else {
                    10
                }
            });
            let pool_k = arg_usize(args, &["pool_k", "poolK", "candidate_pool"])
                .unwrap_or((k.max(8) * 3).min(64));
            let agent = arg_str(args, &["agent", "source_agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));

            let ctx = RecallContext::from_caller(caller_id, state);
            execute_recall_policy_explain(state, query, budget, k, agent, &ctx, None, pool_k).await
        }

        "cortex_semantic_recall" => {
            let query = arg_str(args, &["query", "q"])
                .ok_or_else(|| "Missing required argument: query".to_string())?;
            let budget = arg_usize(args, &["budget", "b"]).unwrap_or(200);
            let k = arg_usize(args, &["k", "limit"]).unwrap_or({
                if budget <= 220 {
                    14
                } else {
                    10
                }
            });
            let agent = arg_str(args, &["agent", "source_agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));

            let ctx = RecallContext::from_caller(caller_id, state);
            execute_semantic_recall(state, query, budget, k, agent, &ctx, None).await
        }

        "cortex_store" => {
            let decision = arg_str(args, &["decision", "d"])
                .ok_or_else(|| "Missing required argument: decision".to_string())?;
            let context = arg_str(args, &["context", "c"]).map(str::to_string);
            let entry_type = arg_str(args, &["type", "t"]).map(str::to_string);
            let source_agent =
                source_agent_for_tool(source, arg_str(args, &["source_agent"]).unwrap_or("mcp"));
            let source_model = source_model_for_tool(source, args);
            let reasoning_depth = arg_str(args, &["reasoning_depth", "reasoningDepth"]);
            let provenance =
                DecisionProvenance::from_fields(&source_agent, source_model, reasoning_depth);
            let confidence = arg_f64(args, &["confidence", "conf"]);
            let ttl_seconds = arg_i64(args, &["ttl_seconds", "ttl"]);
            let decision_embedding = state
                .embedding_engine
                .as_ref()
                .and_then(|engine| engine.embed(decision));

            let mut conn = state.db.lock().await;
            let (entry, new_id) = store_decision_with_input_embedding_and_provenance(
                &mut conn,
                decision,
                context,
                entry_type,
                source_agent.clone(),
                provenance,
                confidence,
                ttl_seconds,
                decision_embedding.as_deref(),
                caller_id,
            )
            .map_err(|err| err.to_string())?;

            if let (Some(id), Some(vec)) = (new_id, decision_embedding.as_deref()) {
                let _ = persist_decision_embedding(&conn, id, vec);
            }

            // Auto-append to active focus session (sawtooth pattern)
            crate::focus::focus_append(&conn, &source_agent, decision);

            Ok(json!({
                "stored": true,
                "id": new_id,
                "sourceAgent": source_agent,
                "kind": entry.get("kind").cloned().unwrap_or(Value::Null),
                "action": entry.get("action").cloned().unwrap_or_else(|| json!("stored")),
            }))
        }

        "cortex_agent_feedback_record" => {
            let owner_id = if state.team_mode {
                caller_id.unwrap_or_default()
            } else {
                0
            };
            let fallback_agent = source
                .as_ref()
                .map(|identity| identity.agent.as_str())
                .unwrap_or("mcp");
            let conn = state.db.lock().await;
            record_agent_feedback_from_value(&conn, owner_id, args, fallback_agent)
        }

        "cortex_agent_feedback_stats" => {
            let owner_id = if state.team_mode {
                caller_id.unwrap_or_default()
            } else {
                0
            };
            let horizon_days = arg_i64(args, &["horizonDays", "horizon_days"]).unwrap_or(30);
            let limit = arg_usize(args, &["limit"]).unwrap_or(400);
            let task_class = arg_str(args, &["taskClass", "task_class"]);
            let agent = arg_str(args, &["agent", "source_agent"]);
            let conn = state.db.lock().await;
            build_agent_feedback_stats_payload(
                &conn,
                owner_id,
                horizon_days,
                limit,
                task_class,
                agent,
            )
        }

        "cortex_health" => Ok(build_health_payload(state).await),

        "cortex_digest" => {
            let conn = state.db.lock().await;
            build_digest(&conn)
        }

        "cortex_unfold" => {
            const MAX_UNFOLD_SOURCES: usize = 50;
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
                    );
                }
            };
            if sources.is_empty() {
                return Err("sources array is empty".to_string());
            }
            if sources.len() > MAX_UNFOLD_SOURCES {
                return Err(format!("Too many sources (max {MAX_UNFOLD_SOURCES})"));
            }
            let agent = args.get("agent").and_then(|v| v.as_str()).unwrap_or("mcp");
            let ctx = RecallContext::from_caller(caller_id, state);
            let conn = state.db_read.lock().await;
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
            drop(conn);

            // Implicit positive feedback: unfolding = "this result was useful"
            if !found_sources.is_empty() {
                let conn = state.db.lock().await;
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

        "cortex_conflicts_list" => {
            let status = ConflictStatusFilter::parse(arg_str(args, &["status"]))?;
            let classification = arg_str(args, &["classification"])
                .map(str::trim)
                .map(str::to_string);
            let conflict_id = arg_str(args, &["conflictId", "conflict_id", "id"])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let limit = arg_usize(args, &["limit"]).unwrap_or(100).clamp(1, 500);

            let options = ConflictListOptions {
                status,
                classification,
                conflict_id,
                limit,
            };
            let conn = state.db.lock().await;
            list_conflicts_payload(&conn, &options)
        }

        "cortex_conflicts_get" => {
            let conflict_id = arg_str(args, &["conflictId", "conflict_id", "id"])
                .ok_or_else(|| "Missing required argument: conflictId".to_string())?
                .to_string();

            let options = ConflictListOptions {
                status: ConflictStatusFilter::All,
                classification: None,
                conflict_id: Some(conflict_id.clone()),
                limit: 200,
            };
            let conn = state.db.lock().await;
            let payload = list_conflicts_payload(&conn, &options)?;
            let found = payload
                .get("count")
                .and_then(|value| value.as_u64())
                .map(|value| value > 0)
                .unwrap_or(false);
            Ok(json!({
                "found": found,
                "conflictId": conflict_id,
                "conflict": payload.get("conflict").cloned().unwrap_or(Value::Null),
            }))
        }

        "cortex_conflicts_resolve" => {
            let action = arg_str(args, &["action"])
                .ok_or_else(|| "Missing required argument: action".to_string())?;
            let mut winner_id = arg_i64(args, &["winnerId", "keepId"]);
            let mut superseded_id = arg_i64(args, &["supersededId", "loserId"]);
            let conflict_id = arg_str(args, &["conflictId", "conflict_id", "id"])
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);

            if let Some((left, right)) = conflict_id.as_deref().and_then(parse_conflict_id) {
                if winner_id.is_none() {
                    winner_id = Some(left);
                }
                if superseded_id.is_none() {
                    superseded_id = winner_id.map(|winner| {
                        if winner == left {
                            right
                        } else if winner == right {
                            left
                        } else {
                            right
                        }
                    });
                }
            }

            let winner_id = winner_id
                .ok_or_else(|| "Missing required argument: winnerId (or keepId)".to_string())?;
            let resolved_by = arg_str(args, &["resolvedBy", "resolved_by"])
                .map(str::to_string)
                .unwrap_or_else(|| source_agent_for_tool(source, "mcp"));
            let metadata = ResolutionMetadata {
                conflict_id,
                classification: arg_str(args, &["classification"]).map(str::to_string),
                notes: arg_str(args, &["notes"]).map(str::to_string),
                resolved_by: Some(resolved_by),
                similarity: arg_f64(args, &["similarity"]),
            };

            let mut conn = state.db.lock().await;
            resolve_decision_with_metadata(&mut conn, winner_id, action, superseded_id, metadata)
        }

        "cortex_focus_start" => {
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: label".to_string())?;
            let agent = arg_str(args, &["agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));
            let conn = state.db.lock().await;
            crate::focus::focus_start(&conn, label, agent)
        }

        "cortex_focus_end" => {
            let label = args
                .get("label")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Missing required argument: label".to_string())?;
            let agent = arg_str(args, &["agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));
            let conn = state.db.lock().await;
            crate::focus::focus_end(&conn, label, agent, caller_id)
        }

        "cortex_focus_status" => {
            let agent = arg_str(args, &["agent"])
                .unwrap_or_else(|| source.as_ref().map(|s| s.agent.as_str()).unwrap_or("mcp"));
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
            let body = DiaryRequest {
                accomplished: arg_str(args, &["accomplished", "done"]).map(str::to_string),
                next_steps: arg_str(args, &["nextSteps", "next_steps", "next"]).map(str::to_string),
                decisions: arg_str(args, &["decisions", "dec"]).map(str::to_string),
                key_decisions: arg_str(args, &["keyDecisions"]).map(str::to_string),
                pending: arg_str(args, &["pending", "pend"]).map(str::to_string),
                known_issues: arg_str(args, &["knownIssues", "known_issues", "issues"])
                    .map(str::to_string),
            };
            let source_agent = source_agent_for_tool(source, "mcp");
            let path = write_diary_entry(state, &body, &source_agent).await?;

            Ok(json!({ "written": true, "agent": source_agent, "path": path }))
        }

        "cortex_lastCall" => {
            let kind = arg_str(args, &["kind"]);
            let agent_filter = arg_str(args, &["agent", "source_agent"]);
            let ctx = RecallContext::from_caller(caller_id, state);
            let conn = state.db.lock().await;
            fetch_last_call(&conn, kind, agent_filter, &ctx)
        }

        "cortex_permissions_list" => {
            let owner_id = if state.team_mode {
                caller_id.unwrap_or_default()
            } else {
                0
            };
            let conn = state.db.lock().await;
            let mut stmt = conn
                .prepare(
                    "SELECT client_id, permission, scope, granted_by, granted_at
                     FROM client_permissions
                     WHERE owner_id = ?1
                     ORDER BY client_id ASC, permission ASC, scope ASC",
                )
                .map_err(|err| err.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![owner_id], |row| {
                    Ok(json!({
                        "client": row.get::<_, String>(0)?,
                        "permission": row.get::<_, String>(1)?,
                        "scope": row.get::<_, String>(2)?,
                        "grantedBy": row.get::<_, String>(3)?,
                        "grantedAt": row.get::<_, String>(4)?,
                    }))
                })
                .map_err(|err| err.to_string())?;
            let grants: Vec<Value> = rows.filter_map(Result::ok).collect();
            Ok(json!({
                "ownerId": owner_id,
                "count": grants.len(),
                "grants": grants
            }))
        }

        "cortex_permissions_grant" => {
            let owner_id = if state.team_mode {
                caller_id.unwrap_or_default()
            } else {
                0
            };
            let client = arg_str(args, &["client", "client_id"])
                .ok_or_else(|| "Missing required argument: client".to_string())?;
            let client = if client.trim() == "*" {
                "*".to_string()
            } else {
                normalize_permission_client_id(client)
            };
            let permission_raw = arg_str(args, &["permission"])
                .ok_or_else(|| "Missing required argument: permission".to_string())?;
            let permission = parse_client_permission(permission_raw)
                .ok_or_else(|| "Invalid permission; expected read, write, or admin".to_string())?;
            let scope = arg_str(args, &["scope"])
                .map(str::to_string)
                .unwrap_or_else(|| "*".to_string());
            let granted_by = source_client_for_permissions(source, args);

            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by, granted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(owner_id, client_id, permission, scope)
                 DO UPDATE SET granted_by = excluded.granted_by, granted_at = excluded.granted_at",
                rusqlite::params![owner_id, client, permission.as_str(), scope, granted_by],
            )
            .map_err(|err| err.to_string())?;

            Ok(json!({
                "granted": true,
                "ownerId": owner_id,
                "client": client,
                "permission": permission.as_str(),
                "scope": scope,
            }))
        }

        "cortex_permissions_revoke" => {
            let owner_id = if state.team_mode {
                caller_id.unwrap_or_default()
            } else {
                0
            };
            let client = arg_str(args, &["client", "client_id"])
                .ok_or_else(|| "Missing required argument: client".to_string())?;
            let client = if client.trim() == "*" {
                "*".to_string()
            } else {
                normalize_permission_client_id(client)
            };
            let permission_raw = arg_str(args, &["permission"])
                .ok_or_else(|| "Missing required argument: permission".to_string())?;
            let permission = parse_client_permission(permission_raw)
                .ok_or_else(|| "Invalid permission; expected read, write, or admin".to_string())?;
            let scope = arg_str(args, &["scope"])
                .map(str::to_string)
                .unwrap_or_else(|| "*".to_string());

            let conn = state.db.lock().await;
            let deleted = conn
                .execute(
                    "DELETE FROM client_permissions
                     WHERE owner_id = ?1 AND client_id = ?2 AND permission = ?3 AND scope = ?4",
                    rusqlite::params![owner_id, client, permission.as_str(), scope],
                )
                .map_err(|err| err.to_string())?;

            Ok(json!({
                "revoked": deleted > 0,
                "deleted": deleted,
                "ownerId": owner_id,
                "client": client,
                "permission": permission.as_str(),
                "scope": scope,
            }))
        }

        _ => Err(format!("Unknown tool: {tool_name}")),
    }
}

// ─── Main MCP message handler ─────────────────────────────────────────────────

pub async fn handle_mcp_message_with_caller(
    state: &RuntimeState,
    msg: &Value,
    caller_id: Option<i64>,
    source: Option<&SourceIdentity>,
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

            match mcp_dispatch(state, caller_id, tool_name, &args, source).await {
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
