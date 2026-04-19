// SPDX-License-Identifier: MIT
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ensure_auth_with_caller_rated, json_response, now_iso};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

// ─── Constants ──────────────────────────────────────────────────────────────

const MAX_FEED: i64 = 200;
const FEED_TTL_SECONDS: i64 = 4 * 60 * 60;

#[allow(clippy::result_large_err)]
fn owner_id_from_request(
    state: &RuntimeState,
    caller_id: Option<i64>,
) -> Result<Option<i64>, Response> {
    if state.team_mode {
        match caller_id {
            Some(owner_id) => Ok(Some(owner_id)),
            None => Err(json_response(
                StatusCode::FORBIDDEN,
                json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
            )),
        }
    } else {
        Ok(None)
    }
}

// ─── Internal feed entry type ───────────────────────────────────────────────

#[derive(Clone)]
struct FeedEntry {
    id: String,
    agent: String,
    kind: String,
    summary: String,
    content: Option<String>,
    files: Value,
    task_id: Option<String>,
    trace_id: Option<String>,
    priority: String,
    timestamp: String,
    tokens: i64,
}

// ─── Request / query types ──────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct FeedRequest {
    pub agent: Option<String>,
    pub kind: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub files: Option<Vec<String>>,
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    #[serde(rename = "traceId")]
    pub trace_id: Option<String>,
    pub priority: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct FeedQuery {
    pub since: Option<String>,
    pub kind: Option<String>,
    pub agent: Option<String>,
    pub unread: Option<bool>,
}

#[derive(Deserialize, Default)]
pub struct FeedAckRequest {
    pub agent: Option<String>,
    #[serde(rename = "lastSeenId")]
    pub last_seen_id: Option<String>,
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

fn parse_duration_to_seconds(raw: &str) -> i64 {
    if raw.is_empty() {
        return 60 * 60;
    }
    let mut chars = raw.chars();
    let unit = chars.next_back().unwrap_or('h');
    let digits = chars.as_str();
    if digits.is_empty() {
        return 60 * 60;
    }
    let value = digits.parse::<i64>().unwrap_or(1).max(1);
    match unit {
        'm' => value * 60,
        'h' => value * 60 * 60,
        'd' => value * 24 * 60 * 60,
        _ => 60 * 60,
    }
}

fn parse_json_array(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}

fn feed_entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedEntry> {
    Ok(FeedEntry {
        id: row.get(0)?,
        agent: row.get(1)?,
        kind: row.get(2)?,
        summary: row.get(3)?,
        content: row.get(4)?,
        files: parse_json_array(&row.get::<_, String>(5)?),
        task_id: row.get(6)?,
        trace_id: row.get(7)?,
        priority: row.get(8)?,
        timestamp: row.get(9)?,
        tokens: row.get(10)?,
    })
}

fn redact_secrets(text: &str) -> String {
    let bearer = Regex::new(r"Bearer\s+[a-f0-9]{32,}")
        .map(|re| re.replace_all(text, "Bearer [REDACTED]").to_string())
        .unwrap_or_else(|_| text.to_string());
    let hashes = Regex::new(r"[a-f0-9]{40,}")
        .map(|re| re.replace_all(&bearer, "[HASH_REDACTED]").to_string())
        .unwrap_or(bearer);
    Regex::new(r"(?i)(?:token|key|secret|password)\s*[:=]\s*\S+")
        .map(|re| re.replace_all(&hashes, "[CREDENTIAL_REDACTED]").to_string())
        .unwrap_or(hashes)
}

fn feed_to_json(entry: &FeedEntry, include_content: bool) -> Value {
    if include_content {
        json!({
            "id": entry.id,
            "agent": entry.agent,
            "kind": entry.kind,
            "summary": entry.summary,
            "content": entry.content,
            "files": entry.files,
            "taskId": entry.task_id,
            "traceId": entry.trace_id,
            "priority": entry.priority,
            "timestamp": entry.timestamp,
            "tokens": entry.tokens
        })
    } else {
        json!({
            "id": entry.id,
            "agent": entry.agent,
            "kind": entry.kind,
            "summary": entry.summary,
            "files": entry.files,
            "taskId": entry.task_id,
            "traceId": entry.trace_id,
            "priority": entry.priority,
            "timestamp": entry.timestamp,
            "tokens": entry.tokens
        })
    }
}

// ─── Cleanup helpers ────────────────────────────────────────────────────────

fn clean_old_feed(conn: &rusqlite::Connection, owner_id: Option<i64>) -> rusqlite::Result<()> {
    let cutoff = (Utc::now() - Duration::seconds(FEED_TTL_SECONDS)).to_rfc3339();
    if let Some(owner_id) = owner_id {
        conn.execute(
            "DELETE FROM feed WHERE owner_id = ?1 AND timestamp < ?2",
            params![owner_id, cutoff],
        )?;
        conn.execute(
            "DELETE FROM feed
             WHERE owner_id = ?1
               AND id IN (
                 SELECT id
                 FROM feed
                 WHERE owner_id = ?1
                 ORDER BY timestamp DESC
                 LIMIT -1 OFFSET ?2
               )",
            params![owner_id, MAX_FEED],
        )?;
    } else {
        conn.execute("DELETE FROM feed WHERE timestamp < ?1", params![cutoff])?;
        conn.execute(
            "DELETE FROM feed
             WHERE id IN (
               SELECT id
               FROM feed
               ORDER BY timestamp DESC
               LIMIT -1 OFFSET ?1
             )",
            params![MAX_FEED],
        )?;
    }
    Ok(())
}

// ─── Fetch helpers ──────────────────────────────────────────────────────────

fn fetch_feed_since(
    conn: &rusqlite::Connection,
    cutoff: &str,
    owner_id: Option<i64>,
) -> Result<Vec<FeedEntry>, String> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
        (
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed WHERE owner_id = ?1 AND timestamp >= ?2 ORDER BY timestamp ASC",
            vec![Box::new(owner_id), Box::new(cutoff.to_string())],
        )
    } else {
        (
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed WHERE timestamp >= ?1 ORDER BY timestamp ASC",
            vec![Box::new(cutoff.to_string())],
        )
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), feed_entry_from_row)
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn fetch_recent_non_self_feed(
    conn: &rusqlite::Connection,
    for_agent: &str,
    owner_id: Option<i64>,
) -> Result<Vec<FeedEntry>, String> {
    let mut stmt = if owner_id.is_some() {
        conn.prepare(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM (
               SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
               FROM feed
               WHERE owner_id = ?1 AND agent != ?2
               ORDER BY timestamp DESC
               LIMIT ?3
             )
             ORDER BY timestamp ASC",
        )
    } else {
        conn.prepare(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM (
               SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
               FROM feed
               WHERE agent != ?1
               ORDER BY timestamp DESC
               LIMIT ?2
             )
             ORDER BY timestamp ASC",
        )
    }
    .map_err(|e| e.to_string())?;

    let rows = if let Some(owner_id) = owner_id {
        stmt.query_map(params![owner_id, for_agent, MAX_FEED], feed_entry_from_row)
    } else {
        stmt.query_map(params![for_agent, MAX_FEED], feed_entry_from_row)
    }
    .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn fetch_unread_since_anchor(
    conn: &rusqlite::Connection,
    for_agent: &str,
    owner_id: Option<i64>,
    anchor_timestamp: &str,
    anchor_id: &str,
) -> Result<Vec<FeedEntry>, String> {
    let mut stmt = if owner_id.is_some() {
        conn.prepare(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed
             WHERE owner_id = ?1
               AND agent != ?2
               AND (timestamp > ?3 OR (timestamp = ?3 AND id > ?4))
             ORDER BY timestamp ASC",
        )
    } else {
        conn.prepare(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed
             WHERE agent != ?1
               AND (timestamp > ?2 OR (timestamp = ?2 AND id > ?3))
             ORDER BY timestamp ASC",
        )
    }
    .map_err(|e| e.to_string())?;

    let rows = if let Some(owner_id) = owner_id {
        stmt.query_map(
            params![owner_id, for_agent, anchor_timestamp, anchor_id],
            feed_entry_from_row,
        )
    } else {
        stmt.query_map(
            params![for_agent, anchor_timestamp, anchor_id],
            feed_entry_from_row,
        )
    }
    .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn get_unread_feed(
    conn: &rusqlite::Connection,
    for_agent: &str,
    owner_id: Option<i64>,
) -> Result<Vec<FeedEntry>, String> {
    let ack = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT last_seen_id FROM feed_acks WHERE owner_id = ?1 AND agent = ?2",
            params![owner_id, for_agent],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    } else {
        conn.query_row(
            "SELECT last_seen_id FROM feed_acks WHERE agent = ?1",
            params![for_agent],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };

    let Some(ack_id) = ack else {
        return fetch_recent_non_self_feed(conn, for_agent, owner_id);
    };

    let ack_timestamp: Option<String> = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT timestamp FROM feed WHERE owner_id = ?1 AND id = ?2",
            params![owner_id, ack_id.clone()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    } else {
        conn.query_row(
            "SELECT timestamp FROM feed WHERE id = ?1",
            params![ack_id.clone()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };

    // Acks can outlive feed rows because the feed is TTL-pruned. If the saved
    // anchor row no longer exists, fall back to the most recent non-self window.
    let Some(anchor_timestamp) = ack_timestamp else {
        return fetch_recent_non_self_feed(conn, for_agent, owner_id);
    };

    fetch_unread_since_anchor(conn, for_agent, owner_id, &anchor_timestamp, &ack_id)
}

fn insert_feed_entry(conn: &rusqlite::Connection, entry: &FeedEntry) -> Result<(), String> {
    conn.execute(
        "INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            entry.id,
            entry.agent,
            entry.kind,
            entry.summary,
            entry.content,
            entry.files.to_string(),
            entry.task_id,
            entry.trace_id,
            entry.priority,
            entry.timestamp,
            entry.tokens
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ─── POST /feed ─────────────────────────────────────────────────────────────

pub async fn handle_post_feed(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FeedRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            );
        }
    };
    let kind = match body.kind {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            );
        }
    };
    let summary = match body.summary {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            );
        }
    };

    let entry = FeedEntry {
        id: Uuid::new_v4().to_string(),
        agent: agent.clone(),
        kind: kind.clone(),
        summary: redact_secrets(&summary),
        content: body.content.map(|c| redact_secrets(&c)),
        files: serde_json::to_value(body.files.unwrap_or_default()).unwrap_or_else(|_| json!([])),
        task_id: body.task_id,
        trace_id: body.trace_id,
        priority: body.priority.unwrap_or_else(|| "normal".to_string()),
        timestamp: now_iso(),
        tokens: ((summary.len() as f64) / 4.0).ceil() as i64,
    };

    let owner_id = match owner_id_from_request(&state, caller_id) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    let conn = state.db.lock().await;
    let _ = clean_old_feed(&conn, owner_id);
    let inserted = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens, owner_id, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'team')",
            params![
                entry.id.clone(),
                entry.agent.clone(),
                entry.kind.clone(),
                entry.summary.clone(),
                entry.content.clone(),
                entry.files.to_string(),
                entry.task_id.clone(),
                entry.trace_id.clone(),
                entry.priority.clone(),
                entry.timestamp.clone(),
                entry.tokens,
                owner_id
            ],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    } else {
        insert_feed_entry(&conn, &entry)
    };
    match inserted {
        Ok(()) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "feed",
                json!({ "feedId": entry.id, "agent": agent, "kind": kind, "summary": entry.summary }),
            );
            json_response(
                StatusCode::CREATED,
                json!({ "feedId": entry.id, "recorded": true }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Post feed failed: {err}") }),
        ),
    }
}

// ─── GET /feed ──────────────────────────────────────────────────────────────

pub async fn handle_get_feed(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<FeedQuery>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };

    let owner_id = match owner_id_from_request(&state, caller_id) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    let conn = state.db_read.lock().await;
    let since = query.since.unwrap_or_else(|| "1h".to_string());
    let cutoff = (Utc::now() - Duration::seconds(parse_duration_to_seconds(&since))).to_rfc3339();

    let mut entries = if query.unread.unwrap_or(false) {
        if let Some(agent) = query.agent.as_deref() {
            get_unread_feed(&conn, agent, owner_id).unwrap_or_default()
        } else {
            vec![]
        }
    } else {
        fetch_feed_since(&conn, &cutoff, owner_id).unwrap_or_default()
    };

    if let Some(kind) = query.kind {
        entries.retain(|e| e.kind == kind);
    }

    let slim = entries
        .iter()
        .map(|e| feed_to_json(e, false))
        .collect::<Vec<_>>();
    json_response(StatusCode::OK, json!({ "entries": slim }))
}

// ─── GET /feed/{id} ─────────────────────────────────────────────────────────

pub async fn handle_get_feed_by_id(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Path(feed_id): Path<String>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };

    let conn = state.db_read.lock().await;
    let owner_id = match owner_id_from_request(&state, caller_id) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    let entry = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens FROM feed WHERE owner_id = ?1 AND id = ?2",
            params![owner_id, feed_id],
            |row| {
                Ok(FeedEntry {
                    id: row.get(0)?,
                    agent: row.get(1)?,
                    kind: row.get(2)?,
                    summary: row.get(3)?,
                    content: row.get(4)?,
                    files: parse_json_array(&row.get::<_, String>(5)?),
                    task_id: row.get(6)?,
                    trace_id: row.get(7)?,
                    priority: row.get(8)?,
                    timestamp: row.get(9)?,
                    tokens: row.get(10)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens FROM feed WHERE id = ?1",
            params![feed_id],
            |row| {
                Ok(FeedEntry {
                    id: row.get(0)?,
                    agent: row.get(1)?,
                    kind: row.get(2)?,
                    summary: row.get(3)?,
                    content: row.get(4)?,
                    files: parse_json_array(&row.get::<_, String>(5)?),
                    task_id: row.get(6)?,
                    trace_id: row.get(7)?,
                    priority: row.get(8)?,
                    timestamp: row.get(9)?,
                    tokens: row.get(10)?,
                })
            },
        )
        .optional()
        .ok()
        .flatten()
    };

    match entry {
        Some(entry) => json_response(StatusCode::OK, feed_to_json(&entry, true)),
        None => json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "feed_entry_not_found" }),
        ),
    }
}

// ─── POST /feed/ack ─────────────────────────────────────────────────────────

pub async fn handle_feed_ack(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FeedAckRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, lastSeenId" }),
            );
        }
    };
    let last_seen_id = match body.last_seen_id {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, lastSeenId" }),
            );
        }
    };

    let owner_id = match owner_id_from_request(&state, caller_id) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    let conn = state.db.lock().await;
    let acked = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO feed_acks (owner_id, agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(owner_id, agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
            params![owner_id, agent, last_seen_id, now_iso()],
        )
    } else {
        conn.execute(
            "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
            params![agent, last_seen_id, now_iso()],
        )
    };
    match acked {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            json_response(StatusCode::OK, json!({ "acked": true }))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Feed ack failed: {err}") }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn
    }

    fn insert_entry(conn: &Connection, id: &str, agent: &str, timestamp: &str) {
        insert_feed_entry(
            conn,
            &FeedEntry {
                id: id.to_string(),
                agent: agent.to_string(),
                kind: "task_complete".to_string(),
                summary: format!("{agent} summary"),
                content: None,
                files: json!([]),
                task_id: None,
                trace_id: None,
                priority: "medium".to_string(),
                timestamp: timestamp.to_string(),
                tokens: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn unread_feed_falls_back_when_ack_anchor_is_missing() {
        let conn = setup_conn();
        insert_entry(&conn, "a1", "alpha", "2026-04-10T00:00:00Z");
        insert_entry(&conn, "b1", "beta", "2026-04-10T00:01:00Z");
        insert_entry(&conn, "g1", "gamma", "2026-04-10T00:02:00Z");

        conn.execute(
            "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)",
            params!["alpha", "missing-anchor", "2026-04-10T00:03:00Z"],
        )
        .unwrap();

        let unread = get_unread_feed(&conn, "alpha", None).unwrap();
        let ids = unread.into_iter().map(|entry| entry.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["b1".to_string(), "g1".to_string()]);
    }

    #[test]
    fn unread_feed_starts_after_ack_and_skips_self_entries() {
        let conn = setup_conn();
        insert_entry(&conn, "a1", "alpha", "2026-04-10T00:00:00Z");
        insert_entry(&conn, "b1", "beta", "2026-04-10T00:01:00Z");
        insert_entry(&conn, "a2", "alpha", "2026-04-10T00:02:00Z");
        insert_entry(&conn, "g1", "gamma", "2026-04-10T00:03:00Z");

        conn.execute(
            "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)",
            params!["alpha", "b1", "2026-04-10T00:04:00Z"],
        )
        .unwrap();

        let unread = get_unread_feed(&conn, "alpha", None).unwrap();
        let ids = unread.into_iter().map(|entry| entry.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["g1".to_string()]);
    }
}
