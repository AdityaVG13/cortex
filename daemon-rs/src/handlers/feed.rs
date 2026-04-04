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

use super::{ensure_auth, json_response, now_iso};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

// ─── Constants ──────────────────────────────────────────────────────────────

const MAX_FEED: i64 = 200;
const FEED_TTL_SECONDS: i64 = 4 * 60 * 60;

fn owner_id_from_state(state: &RuntimeState) -> Option<i64> {
    if state.team_mode {
        state.default_owner_id
    } else {
        None
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
    } else {
        conn.execute("DELETE FROM feed WHERE timestamp < ?1", params![cutoff])?;
    }
    let count: i64 = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT COUNT(*) FROM feed WHERE owner_id = ?1",
            params![owner_id],
            |r| r.get(0),
        )?
    } else {
        conn.query_row("SELECT COUNT(*) FROM feed", [], |r| r.get(0))?
    };
    if count > MAX_FEED {
        if let Some(owner_id) = owner_id {
            conn.execute(
                "DELETE FROM feed WHERE owner_id = ?1 AND id IN (
                    SELECT id FROM feed WHERE owner_id = ?1 ORDER BY timestamp ASC LIMIT ?2
                 )",
                params![owner_id, count - MAX_FEED],
            )?;
        } else {
            conn.execute(
                "DELETE FROM feed WHERE id IN (SELECT id FROM feed ORDER BY timestamp ASC LIMIT ?1)",
                params![count - MAX_FEED],
            )?;
        }
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
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
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
        })
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

    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
        (
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed WHERE owner_id = ?1 ORDER BY timestamp ASC",
            vec![Box::new(owner_id)],
        )
    } else {
        (
            "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
             FROM feed ORDER BY timestamp ASC",
            vec![],
        )
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
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
        })
        .map_err(|e| e.to_string())?;
    let mut all = Vec::new();
    for row in rows.flatten() {
        all.push(row);
    }

    if ack.is_none() {
        return Ok(all
            .into_iter()
            .filter(|entry| entry.agent != for_agent)
            .collect::<Vec<_>>());
    }

    let ack_id = ack.unwrap();
    let mut past_ack = false;
    let mut unread = Vec::new();
    for entry in all {
        if entry.id == ack_id {
            past_ack = true;
            continue;
        }
        if past_ack && entry.agent != for_agent {
            unread.push(entry);
        }
    }
    Ok(unread)
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            )
        }
    };
    let kind = match body.kind {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            )
        }
    };
    let summary = match body.summary {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, kind, summary" }),
            )
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

    let owner_id = owner_id_from_state(&state);
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_old_feed(&conn, owner_id);
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let conn = state.db.lock().await;
    let owner_id = owner_id_from_state(&state);
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, lastSeenId" }),
            )
        }
    };
    let last_seen_id = match body.last_seen_id {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, lastSeenId" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
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
