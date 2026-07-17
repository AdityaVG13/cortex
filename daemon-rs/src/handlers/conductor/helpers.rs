// SPDX-License-Identifier: MIT
use super::*;
use crate::handlers::{json_response, now_iso, parse_json_array, parse_timestamp_ms, resolve_caller_id};
use crate::state::RuntimeState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{Duration, Utc};
use rusqlite::params;
use serde_json::{json, Value};
pub(crate) fn owner_id_from_headers(headers: &HeaderMap, state: &RuntimeState) -> Option<i64> {
    if !state.team_mode {
        return None;
    }
    resolve_caller_id(headers, state).or(state.default_owner_id)
}
pub(crate) fn is_valid_agent_label(agent: &str) -> bool {
    let trimmed = agent.trim();
    !trimmed.is_empty() && trimmed.len() <= 160 && !trimmed.chars().any(|ch| ch.is_control())
}
pub(crate) fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}
pub(crate) fn bounded_ttl_seconds(raw: Option<i64>, default_seconds: i64) -> i64 {
    raw.unwrap_or(default_seconds).clamp(1, MAX_REQUEST_TTL_SECONDS)
}
pub(crate) fn missing_field_response(error: &'static str) -> Response {
    json_response(StatusCode::BAD_REQUEST, json!({ "error": error }))
}
pub(crate) fn query_json_rows(conn: &rusqlite::Connection, sql: &str, params: &[SqlParam], row_to_json: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Value>) -> Result<Vec<Value>, String> {
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(rusqlite::params_from_iter(param_refs), row_to_json).map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}
pub(crate) fn task_row_to_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "taskId": row.get::<_, String>(0)?,
        "title": row.get::<_, String>(1)?,
        "description": row.get::<_, Option<String>>(2)?,
        "project": row.get::<_, Option<String>>(3)?,
        "files": parse_json_array(&row.get::<_, String>(4)?),
        "priority": row.get::<_, String>(5)?,
        "requiredCapability": row.get::<_, String>(6)?,
        "status": row.get::<_, String>(7)?,
        "claimedBy": row.get::<_, Option<String>>(8)?,
        "createdAt": row.get::<_, String>(9)?,
        "claimedAt": row.get::<_, Option<String>>(10)?,
        "completedAt": row.get::<_, Option<String>>(11)?,
        "summary": row.get::<_, Option<String>>(12)?
    }))
}
pub(crate) fn is_unique_constraint(err: &rusqlite::Error) -> bool {
    match err {
        rusqlite::Error::SqliteFailure(code, _) => {
            code.code == rusqlite::ErrorCode::ConstraintViolation
                && (code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE || code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY)
        }
        _ => false,
    }
}
pub(crate) fn clean_expired_locks(conn: &rusqlite::Connection, owner_id: Option<i64>) -> rusqlite::Result<()> {
    if let Some(owner_id) = owner_id {
        conn.execute("DELETE FROM locks WHERE owner_id = ?1 AND expires_at < ?2", params![owner_id, now_iso()])?;
    } else {
        conn.execute("DELETE FROM locks WHERE expires_at < ?1", params![now_iso()])?;
    }
    Ok(())
}
pub(crate) fn clean_old_activities(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM activities
         WHERE id IN (
           SELECT id
           FROM activities
           ORDER BY timestamp DESC
           LIMIT -1 OFFSET ?1
         )",
        params![MAX_ACTIVITIES],
    )?;
    Ok(())
}
pub(crate) fn clean_old_messages(conn: &rusqlite::Connection, recipient: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM messages
         WHERE recipient = ?1
           AND id IN (
             SELECT id
             FROM messages
             WHERE recipient = ?1
             ORDER BY timestamp DESC
             LIMIT -1 OFFSET ?2
           )",
        params![recipient, MAX_MESSAGES_PER_AGENT],
    )?;
    Ok(())
}
pub(crate) fn clean_expired_sessions(conn: &rusqlite::Connection, owner_id: Option<i64>) -> rusqlite::Result<()> {
    if let Some(owner_id) = owner_id {
        conn.execute("DELETE FROM sessions WHERE owner_id = ?1 AND expires_at < ?2", params![owner_id, now_iso()])?;
    } else {
        conn.execute("DELETE FROM sessions WHERE expires_at < ?1", params![now_iso()])?;
    }
    Ok(())
}
pub(crate) fn session_freshness_idle_seconds() -> i64 {
    std::env::var("CORTEX_SESSION_FRESHNESS_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(SESSION_FRESHNESS_IDLE_SECONDS)
        .max(60)
}
pub(crate) fn last_session_heartbeat_ms(conn: &rusqlite::Connection, owner_id: Option<i64>) -> rusqlite::Result<Option<i64>> {
    let last: Option<String> = if let Some(owner_id) = owner_id {
        conn.query_row("SELECT MAX(last_heartbeat) FROM sessions WHERE owner_id = ?1", params![owner_id], |row| row.get(0))?
    } else {
        conn.query_row("SELECT MAX(last_heartbeat) FROM sessions", [], |row| row.get(0))?
    };
    Ok(last.as_deref().map(parse_timestamp_ms))
}
pub(crate) fn should_run_session_freshen(conn: &rusqlite::Connection, owner_id: Option<i64>, now: chrono::DateTime<Utc>) -> bool {
    let Ok(last_heartbeat_ms) = last_session_heartbeat_ms(conn, owner_id) else {
        return true;
    };
    let Some(last_heartbeat_ms) = last_heartbeat_ms else {
        return false;
    };
    if last_heartbeat_ms <= 0 {
        return true;
    }
    let idle_secs = (now.timestamp_millis() - last_heartbeat_ms) / 1000;
    idle_secs >= session_freshness_idle_seconds()
}
pub(crate) fn run_session_freshen(conn: &rusqlite::Connection, state: &RuntimeState, owner_id: Option<i64>) {
    let _ = clean_expired_sessions(conn, owner_id);
    let _ = crate::db::delete_expired_entries(conn);
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;");
    let quick_ok = crate::db::quick_check(conn);
    state.db_corrupted.store(!quick_ok, std::sync::atomic::Ordering::SeqCst);
}
pub(crate) fn clean_old_tasks(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM tasks
         WHERE status = 'completed'
           AND task_id IN (
             SELECT task_id
             FROM tasks
             WHERE status = 'completed'
             ORDER BY COALESCE(completed_at, created_at) DESC
             LIMIT -1 OFFSET ?1
           )",
        params![MAX_TASKS],
    )?;
    Ok(())
}
pub(crate) fn fetch_locks(conn: &rusqlite::Connection, owner_id: Option<i64>) -> Result<Vec<Value>, String> {
    let now = now_iso();
    let (sql, params_vec): (&str, Vec<SqlParam>) = if let Some(owner_id) = owner_id {
        (
            "SELECT id, path, agent, locked_at, expires_at
             FROM locks
             WHERE owner_id = ?1 AND (expires_at IS NULL OR expires_at >= ?2)
             ORDER BY locked_at ASC",
            vec![Box::new(owner_id), Box::new(now.clone())],
        )
    } else {
        (
            "SELECT id, path, agent, locked_at, expires_at
             FROM locks
             WHERE expires_at IS NULL OR expires_at >= ?1
             ORDER BY locked_at ASC",
            vec![Box::new(now)],
        )
    };
    query_json_rows(conn, sql, &params_vec, |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "path": row.get::<_, String>(1)?,
            "agent": row.get::<_, String>(2)?,
            "lockedAt": row.get::<_, String>(3)?,
            "expiresAt": row.get::<_, String>(4)?
        }))
    })
}
pub(crate) fn fetch_messages_for_agent(conn: &rusqlite::Connection, agent: &str, owner_id: Option<i64>) -> Result<Vec<Value>, String> {
    let (sql, params_vec): (&str, Vec<SqlParam>) = if let Some(owner_id) = owner_id {
        (
            "SELECT id, sender, recipient, message, timestamp FROM messages WHERE owner_id = ?1 AND recipient = ?2 ORDER BY timestamp ASC",
            vec![Box::new(owner_id), Box::new(agent.to_string())],
        )
    } else {
        (
            "SELECT id, sender, recipient, message, timestamp FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC",
            vec![Box::new(agent.to_string())],
        )
    };
    query_json_rows(conn, sql, &params_vec, |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "from": row.get::<_, String>(1)?,
            "to": row.get::<_, String>(2)?,
            "message": row.get::<_, String>(3)?,
            "timestamp": row.get::<_, String>(4)?
        }))
    })
}
pub(crate) fn fetch_sessions(conn: &rusqlite::Connection, owner_id: Option<i64>) -> Result<Vec<Value>, String> {
    let heartbeat_cutoff = (Utc::now() - Duration::seconds(ACTIVE_SESSION_WINDOW_SECONDS)).to_rfc3339();
    let now = now_iso();
    let (sql, params_vec): (&str, Vec<SqlParam>) = if let Some(owner_id) = owner_id {
        (
            "SELECT session_id, agent, project, files_json, description, started_at, last_heartbeat, expires_at
             FROM sessions
             WHERE owner_id = ?1
               AND last_heartbeat >= ?2
               AND (expires_at IS NULL OR expires_at >= ?3)
             ORDER BY last_heartbeat DESC",
            vec![Box::new(owner_id), Box::new(heartbeat_cutoff.clone()), Box::new(now.clone())],
        )
    } else {
        (
            "SELECT session_id, agent, project, files_json, description, started_at, last_heartbeat, expires_at
             FROM sessions
             WHERE last_heartbeat >= ?1
               AND (expires_at IS NULL OR expires_at >= ?2)
             ORDER BY last_heartbeat DESC",
            vec![Box::new(heartbeat_cutoff), Box::new(now)],
        )
    };
    query_json_rows(conn, sql, &params_vec, |row| {
        Ok(json!({
            "sessionId": row.get::<_, String>(0)?,
            "agent": row.get::<_, String>(1)?,
            "project": row.get::<_, Option<String>>(2)?,
            "files": parse_json_array(&row.get::<_, String>(3)?),
            "description": row.get::<_, Option<String>>(4)?,
            "startedAt": row.get::<_, String>(5)?,
            "lastHeartbeat": row.get::<_, String>(6)?,
            "expiresAt": row.get::<_, String>(7)?
        }))
    })
}
pub(crate) fn fetch_tasks(conn: &rusqlite::Connection, status_filter: &str, project: Option<&str>, owner_id: Option<i64>, limit: usize, offset: usize) -> Result<Vec<Value>, String> {
    let base = "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary FROM tasks";
    let mut conditions = Vec::new();
    let mut params: Vec<SqlParam> = Vec::new();
    if status_filter != "all" {
        params.push(Box::new(status_filter.to_string()));
        conditions.push(format!("status = ?{}", params.len()));
    }
    if let Some(owner_id) = owner_id {
        params.push(Box::new(owner_id));
        conditions.push(format!("owner_id = ?{}", params.len()));
    }
    if let Some(proj) = project {
        params.push(Box::new(proj.to_string()));
        conditions.push(format!("project = ?{}", params.len()));
    }
    let sql = if conditions.is_empty() {
        format!("{} ORDER BY created_at ASC LIMIT ?{} OFFSET ?{}", base, params.len() + 1, params.len() + 2)
    } else {
        format!(
            "{} WHERE {} ORDER BY created_at ASC LIMIT ?{} OFFSET ?{}",
            base,
            conditions.join(" AND "),
            params.len() + 1,
            params.len() + 2
        )
    };
    params.push(Box::new(limit as i64));
    params.push(Box::new(offset as i64));
    query_json_rows(conn, &sql, &params, task_row_to_json)
}
