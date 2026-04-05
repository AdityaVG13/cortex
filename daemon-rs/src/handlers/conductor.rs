// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, TimeZone, Utc};
use regex::Regex;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ensure_auth, json_response, now_iso};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

// ─── Constants ──────────────────────────────────────────────────────────────

const SESSION_TTL_SECONDS: i64 = 120;
const MAX_ACTIVITIES: i64 = 1000;
const MAX_MESSAGES_PER_AGENT: i64 = 100;
const MAX_TASKS: i64 = 500;

// ─── Request / query types ──────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct LockRequest {
    pub path: Option<String>,
    pub agent: Option<String>,
    pub ttl: Option<i64>,
}

#[derive(Deserialize, Default)]
pub struct ActivityRequest {
    pub agent: Option<String>,
    pub description: Option<String>,
    pub files: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
pub struct SinceQuery {
    pub since: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct MessageRequest {
    pub from: Option<String>,
    pub to: Option<String>,
    pub message: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct MessagesQuery {
    pub agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SessionStartRequest {
    pub agent: Option<String>,
    pub project: Option<String>,
    pub files: Option<Vec<String>>,
    pub description: Option<String>,
    pub ttl: Option<i64>,
}

#[derive(Deserialize, Default)]
pub struct SessionHeartbeatRequest {
    pub agent: Option<String>,
    pub files: Option<Vec<String>>,
    pub description: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct SessionEndRequest {
    pub agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TaskCreateRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub project: Option<String>,
    pub files: Option<Vec<String>>,
    pub priority: Option<String>,
    #[serde(rename = "requiredCapability")]
    pub required_capability: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TaskQuery {
    pub status: Option<String>,
    pub project: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TaskClaimRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TaskCompleteRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
    pub summary: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct TaskAbandonRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct NextTaskQuery {
    pub agent: Option<String>,
    pub capability: Option<String>,
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

fn owner_id_from_state(state: &RuntimeState) -> Option<i64> {
    if state.team_mode {
        state.default_owner_id
    } else {
        None
    }
}

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

fn parse_timestamp_ms(value: &str) -> i64 {
    if value.trim().is_empty() {
        return 0;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    0
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

// ─── Cleanup helpers ────────────────────────────────────────────────────────

fn clean_expired_locks(conn: &rusqlite::Connection, owner_id: Option<i64>) -> rusqlite::Result<()> {
    if let Some(owner_id) = owner_id {
        conn.execute(
            "DELETE FROM locks WHERE owner_id = ?1 AND expires_at < ?2",
            params![owner_id, now_iso()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM locks WHERE expires_at < ?1",
            params![now_iso()],
        )?;
    }
    Ok(())
}

fn clean_old_activities(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))?;
    if count > MAX_ACTIVITIES {
        conn.execute(
            "DELETE FROM activities WHERE id IN (SELECT id FROM activities ORDER BY timestamp ASC LIMIT ?1)",
            params![count - MAX_ACTIVITIES],
        )?;
    }
    Ok(())
}

fn clean_old_messages(conn: &rusqlite::Connection, recipient: &str) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE recipient = ?1",
        params![recipient],
        |r| r.get(0),
    )?;
    if count > MAX_MESSAGES_PER_AGENT {
        conn.execute(
            "DELETE FROM messages WHERE id IN (SELECT id FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC LIMIT ?2)",
            params![recipient, count - MAX_MESSAGES_PER_AGENT],
        )?;
    }
    Ok(())
}

fn clean_expired_sessions(
    conn: &rusqlite::Connection,
    owner_id: Option<i64>,
) -> rusqlite::Result<()> {
    if let Some(owner_id) = owner_id {
        conn.execute(
            "DELETE FROM sessions WHERE owner_id = ?1 AND expires_at < ?2",
            params![owner_id, now_iso()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM sessions WHERE expires_at < ?1",
            params![now_iso()],
        )?;
    }
    Ok(())
}

fn clean_old_tasks(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
    if count > MAX_TASKS {
        conn.execute(
            "DELETE FROM tasks WHERE task_id IN (SELECT task_id FROM tasks WHERE status = 'completed' ORDER BY completed_at ASC LIMIT ?1)",
            params![count - MAX_TASKS],
        )?;
    }
    Ok(())
}

// ─── Fetch helpers ──────────────────────────────────────────────────────────

fn fetch_locks(conn: &rusqlite::Connection, owner_id: Option<i64>) -> Result<Vec<Value>, String> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
        (
            "SELECT id, path, agent, locked_at, expires_at FROM locks WHERE owner_id = ?1 ORDER BY locked_at ASC",
            vec![Box::new(owner_id)],
        )
    } else {
        (
            "SELECT id, path, agent, locked_at, expires_at FROM locks ORDER BY locked_at ASC",
            vec![],
        )
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "path": row.get::<_, String>(1)?,
                "agent": row.get::<_, String>(2)?,
                "lockedAt": row.get::<_, String>(3)?,
                "expiresAt": row.get::<_, String>(4)?
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn fetch_messages_for_agent(
    conn: &rusqlite::Connection,
    agent: &str,
    owner_id: Option<i64>,
) -> Result<Vec<Value>, String> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
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
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "from": row.get::<_, String>(1)?,
                "to": row.get::<_, String>(2)?,
                "message": row.get::<_, String>(3)?,
                "timestamp": row.get::<_, String>(4)?
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn fetch_sessions(
    conn: &rusqlite::Connection,
    owner_id: Option<i64>,
) -> Result<Vec<Value>, String> {
    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
        (
            "SELECT session_id, agent, project, files_json, description, started_at, last_heartbeat, expires_at
             FROM sessions WHERE owner_id = ?1 ORDER BY started_at ASC",
            vec![Box::new(owner_id)],
        )
    } else {
        (
            "SELECT session_id, agent, project, files_json, description, started_at, last_heartbeat, expires_at
             FROM sessions ORDER BY started_at ASC",
            vec![],
        )
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
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
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

fn fetch_tasks(
    conn: &rusqlite::Connection,
    status_filter: &str,
    project: Option<&str>,
    owner_id: Option<i64>,
) -> Result<Vec<Value>, String> {
    // Build parameterized query -- never interpolate user input into SQL.
    let base = "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary FROM tasks";
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

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
        format!("{} ORDER BY created_at ASC", base)
    } else {
        format!(
            "{} WHERE {} ORDER BY created_at ASC",
            base,
            conditions.join(" AND ")
        )
    };

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
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
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        out.push(row);
    }
    Ok(out)
}

// ─── POST /lock ─────────────────────────────────────────────────────────────

pub async fn handle_lock(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<LockRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let path = match body.path {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: path, agent" }),
            )
        }
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: path, agent" }),
            )
        }
    };

    let ttl = body.ttl.unwrap_or(300).max(1);
    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_expired_locks(&conn, owner_id);

    let existing = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT id, agent, expires_at FROM locks WHERE owner_id = ?1 AND path = ?2",
            params![owner_id, path.clone()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT id, agent, expires_at FROM locks WHERE path = ?1",
            params![path.clone()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    };

    let now = Utc::now();
    let expires_at = (now + Duration::seconds(ttl)).to_rfc3339();
    if let Some((lock_id, holder, holder_expires)) = existing {
        if holder == agent {
            let _ = conn.execute(
                "UPDATE locks SET expires_at = ?1 WHERE path = ?2",
                params![expires_at.clone(), path],
            );
            checkpoint_wal_best_effort(&conn);
            return json_response(
                StatusCode::OK,
                json!({ "locked": true, "lockId": lock_id, "expiresAt": expires_at }),
            );
        }

        let minutes_left = {
            let target = parse_timestamp_ms(&holder_expires);
            let now_ms = Utc::now().timestamp_millis();
            ((target - now_ms) as f64 / 60000.0).ceil().max(0.0) as i64
        };
        return json_response(
            StatusCode::CONFLICT,
            json!({
                "error": "file_already_locked",
                "holder": holder,
                "expiresAt": holder_expires,
                "minutesLeft": minutes_left
            }),
        );
    }

    let lock_id = Uuid::new_v4().to_string();
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO locks (id, path, agent, owner_id, locked_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                lock_id.clone(),
                path.clone(),
                agent.clone(),
                owner_id,
                now_iso(),
                expires_at.clone()
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO locks (id, path, agent, locked_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                lock_id.clone(),
                path.clone(),
                agent.clone(),
                now_iso(),
                expires_at.clone()
            ],
        )
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "lock",
                json!({ "action": "acquired", "path": path, "agent": agent }),
            );
            json_response(
                StatusCode::OK,
                json!({ "locked": true, "lockId": lock_id, "expiresAt": expires_at }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Lock failed: {err}") }),
        ),
    }
}

// ─── POST /unlock ───────────────────────────────────────────────────────────

pub async fn handle_unlock(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<LockRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let path = match body.path {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: path, agent" }),
            )
        }
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: path, agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_expired_locks(&conn, owner_id);
    let holder = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT agent FROM locks WHERE owner_id = ?1 AND path = ?2",
            params![owner_id, path.clone()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT agent FROM locks WHERE path = ?1",
            params![path.clone()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    };

    let holder = match holder {
        Some(v) => v,
        None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "no_lock_found" })),
    };

    if holder != agent {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "not_lock_holder", "holder": holder }),
        );
    }

    if let Some(owner_id) = owner_id {
        let _ = conn.execute(
            "DELETE FROM locks WHERE owner_id = ?1 AND path = ?2",
            params![owner_id, path.clone()],
        );
    } else {
        let _ = conn.execute("DELETE FROM locks WHERE path = ?1", params![path.clone()]);
    }
    checkpoint_wal_best_effort(&conn);
    state.emit(
        "lock",
        json!({ "action": "released", "path": path, "agent": agent }),
    );
    json_response(StatusCode::OK, json!({ "unlocked": true }))
}

// ─── GET /locks ─────────────────────────────────────────────────────────────

pub async fn handle_locks(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_expired_locks(&conn, owner_id);
    match fetch_locks(&conn, owner_id) {
        Ok(locks) => json_response(StatusCode::OK, json!({ "locks": locks })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Get locks failed: {err}") }),
        ),
    }
}

// ─── POST /activity ─────────────────────────────────────────────────────────

pub async fn handle_post_activity(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ActivityRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, description" }),
            )
        }
    };
    let description = match body.description {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: agent, description" }),
            )
        }
    };

    let files = body.files.unwrap_or_default();
    let id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_activities(&conn);
    let owner_id = owner_id_from_state(&state);
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO activities (id, agent, description, files_json, timestamp, owner_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.clone(),
                agent,
                description,
                serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string()),
                now_iso(),
                owner_id
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO activities (id, agent, description, files_json, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.clone(),
                agent,
                description,
                serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string()),
                now_iso()
            ],
        )
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            json_response(
                StatusCode::OK,
                json!({ "recorded": true, "activityId": id }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Post activity failed: {err}") }),
        ),
    }
}

// ─── GET /activity ──────────────────────────────────────────────────────────

pub async fn handle_get_activity(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<SinceQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let since_secs = parse_duration_to_seconds(query.since.as_deref().unwrap_or("1h"));
    let cutoff = (Utc::now() - Duration::seconds(since_secs)).to_rfc3339();
    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;

    let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(owner_id) =
        owner_id
    {
        (
            "SELECT id, agent, description, files_json, timestamp FROM activities WHERE owner_id = ?1 AND timestamp >= ?2 ORDER BY timestamp ASC",
            vec![Box::new(owner_id), Box::new(cutoff.clone())],
        )
    } else {
        (
            "SELECT id, agent, description, files_json, timestamp FROM activities WHERE timestamp >= ?1 ORDER BY timestamp ASC",
            vec![Box::new(cutoff.clone())],
        )
    };

    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("Get activity failed: {err}") }),
            )
        }
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(param_refs), |row| {
        let files: String = row.get(3)?;
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "agent": row.get::<_, String>(1)?,
            "description": row.get::<_, String>(2)?,
            "files": parse_json_array(&files),
            "timestamp": row.get::<_, String>(4)?
        }))
    });

    match rows {
        Ok(iter) => {
            let mut activities = Vec::new();
            for row in iter.flatten() {
                activities.push(row);
            }
            json_response(StatusCode::OK, json!({ "activities": activities }))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Get activity failed: {err}") }),
        ),
    }
}

// ─── POST /message ──────────────────────────────────────────────────────────

pub async fn handle_post_message(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<MessageRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let from = match body.from {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: from, to, message" }),
            )
        }
    };
    let to = match body.to {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: from, to, message" }),
            )
        }
    };
    let message = match body.message {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: from, to, message" }),
            )
        }
    };

    let id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_messages(&conn, &to);
    let owner_id = owner_id_from_state(&state);
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO messages (id, sender, recipient, message, timestamp, owner_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id.clone(), from, to, message, now_iso(), owner_id],
        )
    } else {
        conn.execute(
            "INSERT INTO messages (id, sender, recipient, message, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id.clone(), from, to, message, now_iso()],
        )
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            json_response(StatusCode::OK, json!({ "sent": true, "messageId": id }))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Post message failed: {err}") }),
        ),
    }
}

// ─── GET /messages ──────────────────────────────────────────────────────────

pub async fn handle_get_messages(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<MessagesQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = match query.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing parameter: agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    match fetch_messages_for_agent(&conn, &agent, owner_id) {
        Ok(messages) => json_response(StatusCode::OK, json!({ "messages": messages })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Get messages failed: {err}") }),
        ),
    }
}

// ─── POST /session/start ────────────────────────────────────────────────────

pub async fn handle_session_start(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<SessionStartRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required field: agent" }),
            )
        }
    };

    let ttl = body.ttl.unwrap_or(SESSION_TTL_SECONDS).max(1);
    let owner_id = owner_id_from_state(&state);
    let now = Utc::now();
    let session_id = Uuid::new_v4().to_string();
    let started_at = now.to_rfc3339();
    let expires_at = (now + Duration::seconds(ttl)).to_rfc3339();
    let files_json =
        serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());

    let conn = state.db.lock().await;
    let write = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               session_id = excluded.session_id,
               project = excluded.project,
               files_json = excluded.files_json,
               description = excluded.description,
               started_at = excluded.started_at,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            params![
                agent.clone(),
                owner_id,
                session_id.clone(),
                body.project.clone(),
                files_json,
                body.description.clone(),
                started_at,
                expires_at
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)
             ON CONFLICT(agent) DO UPDATE SET
               session_id = excluded.session_id,
               project = excluded.project,
               files_json = excluded.files_json,
               description = excluded.description,
               started_at = excluded.started_at,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            params![
                agent.clone(),
                session_id.clone(),
                body.project.clone(),
                files_json,
                body.description.clone(),
                started_at,
                expires_at
            ],
        )
    };
    match write {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "session",
                json!({ "action": "started", "agent": agent, "project": body.project }),
            );
            json_response(
                StatusCode::OK,
                json!({ "sessionId": session_id, "heartbeatInterval": 60 }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Session start failed: {err}") }),
        ),
    }
}

// ─── POST /session/heartbeat ────────────────────────────────────────────────

pub async fn handle_session_heartbeat(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<SessionHeartbeatRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = body.agent.unwrap_or_default().trim().to_string();
    if agent.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing or invalid required field: agent" }),
        );
    }
    if agent.len() > 100 {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Invalid agent: name too long (max 100 chars)" }),
        );
    }
    let agent_re = Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
    if !agent_re.is_match(&agent) {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Invalid agent: name contains invalid characters (use alphanumeric, underscore, hyphen only)" }),
        );
    }

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_expired_sessions(&conn, owner_id);
    let exists = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT session_id FROM sessions WHERE owner_id = ?1 AND agent = ?2",
            params![owner_id, agent.clone()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT session_id FROM sessions WHERE agent = ?1",
            params![agent.clone()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    };
    if exists.is_none() {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({ "error": "no_active_session" }),
        );
    }

    let now = Utc::now();
    let expires_at = (now + Duration::seconds(SESSION_TTL_SECONDS)).to_rfc3339();
    let files_json = body
        .files
        .as_ref()
        .map(|f| serde_json::to_string(f).unwrap_or_else(|_| "[]".to_string()));
    let update = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE sessions SET
               last_heartbeat = ?1,
               expires_at = ?2,
               files_json = CASE WHEN ?3 IS NULL THEN files_json ELSE ?3 END,
               description = CASE WHEN ?4 IS NULL THEN description ELSE ?4 END
             WHERE owner_id = ?5 AND agent = ?6",
            params![
                now.to_rfc3339(),
                expires_at.clone(),
                files_json,
                body.description,
                owner_id,
                agent
            ],
        )
    } else {
        conn.execute(
            "UPDATE sessions SET
               last_heartbeat = ?1,
               expires_at = ?2,
               files_json = CASE WHEN ?3 IS NULL THEN files_json ELSE ?3 END,
               description = CASE WHEN ?4 IS NULL THEN description ELSE ?4 END
             WHERE agent = ?5",
            params![
                now.to_rfc3339(),
                expires_at.clone(),
                files_json,
                body.description,
                agent
            ],
        )
    };
    match update {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            json_response(
                StatusCode::OK,
                json!({ "renewed": true, "expiresAt": expires_at }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Session heartbeat failed: {err}") }),
        ),
    }
}

// ─── POST /session/end ──────────────────────────────────────────────────────

pub async fn handle_session_end(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<SessionEndRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required field: agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let deleted = if let Some(owner_id) = owner_id {
        conn.execute(
            "DELETE FROM sessions WHERE owner_id = ?1 AND agent = ?2",
            params![owner_id, agent.clone()],
        )
    } else {
        conn.execute(
            "DELETE FROM sessions WHERE agent = ?1",
            params![agent.clone()],
        )
    };
    match deleted {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit("session", json!({ "action": "ended", "agent": agent }));
            json_response(StatusCode::OK, json!({ "ended": true }))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Session end failed: {err}") }),
        ),
    }
}

// ─── GET /sessions ──────────────────────────────────────────────────────────

pub async fn handle_sessions(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let _ = clean_expired_sessions(&conn, owner_id);
    match fetch_sessions(&conn, owner_id) {
        Ok(sessions) => json_response(StatusCode::OK, json!({ "sessions": sessions })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Get sessions failed: {err}") }),
        ),
    }
}

// ─── POST /tasks ────────────────────────────────────────────────────────────

pub async fn handle_create_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskCreateRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let title = match body.title {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required field: title" }),
            )
        }
    };

    let task_id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_tasks(&conn);
    let files_json =
        serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
    let owner_id = owner_id_from_state(&state);
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO tasks (task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary, owner_id, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, NULL, NULL, NULL, ?9, 'private')",
            params![
                task_id.clone(),
                title.clone(),
                body.description,
                body.project,
                files_json,
                body.priority.unwrap_or_else(|| "medium".to_string()),
                body.required_capability
                    .unwrap_or_else(|| "any".to_string()),
                now_iso(),
                owner_id
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO tasks (task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, NULL, NULL, NULL)",
            params![
                task_id.clone(),
                title.clone(),
                body.description,
                body.project,
                files_json,
                body.priority.unwrap_or_else(|| "medium".to_string()),
                body.required_capability
                    .unwrap_or_else(|| "any".to_string()),
                now_iso()
            ],
        )
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({ "action": "created", "taskId": task_id, "title": title }),
            );
            json_response(
                StatusCode::CREATED,
                json!({ "taskId": task_id, "status": "pending" }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Create task failed: {err}") }),
        ),
    }
}

// ─── GET /tasks ─────────────────────────────────────────────────────────────

pub async fn handle_get_tasks(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<TaskQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let status_filter = query.status.unwrap_or_else(|| "pending".to_string());
    let project_filter = query.project;
    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    match fetch_tasks(&conn, &status_filter, project_filter.as_deref(), owner_id) {
        Ok(tasks) => json_response(StatusCode::OK, json!({ "tasks": tasks })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Get tasks failed: {err}") }),
        ),
    }
}

// ─── POST /tasks/claim ──────────────────────────────────────────────────────

pub async fn handle_claim_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskClaimRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let task_id = match body.task_id {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let row = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT status, claimed_by, title FROM tasks WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT status, claimed_by, title FROM tasks WHERE task_id = ?1",
            params![task_id.clone()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    };
    let (status, claimed_by, title) = match row {
        Some(v) => v,
        None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
    };
    if status == "claimed" {
        return json_response(
            StatusCode::CONFLICT,
            json!({ "error": "task_already_claimed", "claimedBy": claimed_by }),
        );
    }
    if status == "completed" {
        return json_response(
            StatusCode::CONFLICT,
            json!({ "error": "task_already_completed" }),
        );
    }

    let claim = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE tasks SET status = 'claimed', claimed_by = ?1, claimed_at = ?2 WHERE owner_id = ?3 AND task_id = ?4",
            params![agent.clone(), now_iso(), owner_id, task_id.clone()],
        )
    } else {
        conn.execute(
            "UPDATE tasks SET status = 'claimed', claimed_by = ?1, claimed_at = ?2 WHERE task_id = ?3",
            params![agent.clone(), now_iso(), task_id.clone()],
        )
    };
    match claim {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({ "action": "claimed", "taskId": task_id, "title": title, "agent": agent }),
            );
            json_response(
                StatusCode::OK,
                json!({ "claimed": true, "taskId": task_id }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Claim task failed: {err}") }),
        ),
    }
}

// ─── POST /tasks/complete ───────────────────────────────────────────────────

pub async fn handle_complete_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskCompleteRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let task_id = match body.task_id {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let row = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT claimed_by, title, files_json FROM tasks WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT claimed_by, title, files_json FROM tasks WHERE task_id = ?1",
            params![task_id.clone()],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten()
    };
    let (claimed_by, title, files_json) = match row {
        Some(v) => v,
        None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
    };
    if claimed_by.as_deref() != Some(agent.as_str()) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "not_task_holder", "claimedBy": claimed_by }),
        );
    }

    let complete = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE tasks SET status = 'completed', completed_at = ?1, summary = ?2 WHERE owner_id = ?3 AND task_id = ?4",
            params![now_iso(), body.summary.clone(), owner_id, task_id.clone()],
        )
    } else {
        conn.execute(
            "UPDATE tasks SET status = 'completed', completed_at = ?1, summary = ?2 WHERE task_id = ?3",
            params![now_iso(), body.summary.clone(), task_id.clone()],
        )
    };
    match complete {
        Ok(_) => {
            state.emit(
                "task",
                json!({ "action": "completed", "taskId": task_id, "title": title, "agent": agent }),
            );

            // Auto-post feed entry for task completion
            let posted: i64 = if let Some(owner_id) = owner_id {
                conn.query_row(
                    "SELECT COUNT(*) FROM feed WHERE owner_id = ?1 AND task_id = ?2 AND kind = 'task_complete'",
                    params![owner_id, task_id.clone()],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            } else {
                conn.query_row(
                    "SELECT COUNT(*) FROM feed WHERE task_id = ?1 AND kind = 'task_complete'",
                    params![task_id.clone()],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            };
            if posted == 0 {
                let feed_id = Uuid::new_v4().to_string();
                let summary_text = redact_secrets(&format!("Completed: {title}"));
                let content_text = body.summary.as_ref().map(|s| redact_secrets(s));
                let files = parse_json_array(&files_json);
                let tokens = ((title.len() as f64) / 4.0).ceil() as i64;
                let ts = now_iso();
                if let Some(owner_id) = owner_id {
                    let _ = conn.execute(
                        "INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens, owner_id, visibility)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'team')",
                        params![
                            feed_id.clone(),
                            agent.clone(),
                            "task_complete",
                            summary_text.clone(),
                            content_text.clone(),
                            files.to_string(),
                            task_id.clone(),
                            Option::<String>::None,
                            "normal",
                            ts,
                            tokens,
                            owner_id
                        ],
                    );
                } else {
                    let _ = conn.execute(
                        "INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            feed_id.clone(),
                            agent.clone(),
                            "task_complete",
                            summary_text.clone(),
                            content_text.clone(),
                            files.to_string(),
                            task_id.clone(),
                            Option::<String>::None,
                            "normal",
                            ts,
                            tokens
                        ],
                    );
                }
                state.emit(
                    "feed",
                    json!({ "feedId": feed_id, "agent": agent, "kind": "task_complete", "summary": summary_text }),
                );
            }
            checkpoint_wal_best_effort(&conn);
            json_response(
                StatusCode::OK,
                json!({ "completed": true, "taskId": task_id }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Complete task failed: {err}") }),
        ),
    }
}

// ─── POST /tasks/abandon ────────────────────────────────────────────────────

pub async fn handle_abandon_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskAbandonRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let task_id = match body.task_id {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };
    let agent = match body.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing required fields: taskId, agent" }),
            )
        }
    };

    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;
    let row = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT claimed_by, title FROM tasks WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT claimed_by, title FROM tasks WHERE task_id = ?1",
            params![task_id.clone()],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
    };
    let (claimed_by, title) = match row {
        Some(v) => v,
        None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
    };
    if claimed_by.as_deref() != Some(agent.as_str()) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "not_task_holder", "claimedBy": claimed_by }),
        );
    }

    let abandon = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE tasks SET status = 'pending', claimed_by = NULL, claimed_at = NULL WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
        )
    } else {
        conn.execute(
            "UPDATE tasks SET status = 'pending', claimed_by = NULL, claimed_at = NULL WHERE task_id = ?1",
            params![task_id.clone()],
        )
    };
    match abandon {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({ "action": "abandoned", "taskId": task_id, "title": title, "agent": agent }),
            );
            json_response(
                StatusCode::OK,
                json!({ "abandoned": true, "taskId": task_id, "status": "pending" }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Abandon task failed: {err}") }),
        ),
    }
}

// ─── GET /tasks/next ────────────────────────────────────────────────────────

pub async fn handle_next_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<NextTaskQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let _agent = match query.agent {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing parameter: agent" }),
            )
        }
    };
    let capability = query.capability.unwrap_or_else(|| "any".to_string());
    let owner_id = owner_id_from_state(&state);
    let conn = state.db.lock().await;

    let sql = if owner_id.is_some() {
        "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary
         FROM tasks
         WHERE owner_id = ?2
           AND status = 'pending'
           AND (?1 = 'any' OR required_capability = 'any' OR required_capability = ?1)
         ORDER BY
           CASE priority
             WHEN 'critical' THEN 4
             WHEN 'high' THEN 3
             WHEN 'medium' THEN 2
             WHEN 'low' THEN 1
             ELSE 0
           END DESC,
           created_at ASC
         LIMIT 1"
    } else {
        "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary
         FROM tasks
         WHERE status = 'pending'
           AND (?1 = 'any' OR required_capability = 'any' OR required_capability = ?1)
         ORDER BY
           CASE priority
             WHEN 'critical' THEN 4
             WHEN 'high' THEN 3
             WHEN 'medium' THEN 2
             WHEN 'low' THEN 1
             ELSE 0
           END DESC,
           created_at ASC
         LIMIT 1"
    };
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("Get next task failed: {err}") }),
            )
        }
    };

    let task = if let Some(owner_id) = owner_id {
        stmt.query_row(params![capability, owner_id], |row| {
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
        })
        .optional()
        .ok()
        .flatten()
    } else {
        stmt.query_row(params![capability], |row| {
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
        })
        .optional()
        .ok()
        .flatten()
    };

    json_response(StatusCode::OK, json!({ "task": task }))
}

