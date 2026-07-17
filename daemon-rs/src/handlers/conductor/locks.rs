use super::*;
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{ensure_auth_rated, json_response, now_iso, parse_timestamp_ms};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use uuid::Uuid;
pub async fn handle_lock(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<LockRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let path = match trimmed_non_empty(body.path) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: path, agent"),
    };
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: path, agent"),
    };
    let ttl = bounded_ttl_seconds(body.ttl, 300);
    let owner_id = owner_id_from_headers(&headers, &state);
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
            let _ = if let Some(owner_id) = owner_id {
                conn.execute(
                    "UPDATE locks SET expires_at = ?1 WHERE owner_id = ?2 AND path = ?3",
                    params![expires_at.clone(), owner_id, path.clone()],
                )
            } else {
                conn.execute(
                    "UPDATE locks SET expires_at = ?1 WHERE path = ?2",
                    params![expires_at.clone(), path.clone()],
                )
            };
            checkpoint_wal_best_effort(&conn);
            return json_response(
                StatusCode::OK,
                json!({"locked":true,
"lockId":lock_id,"expiresAt":expires_at}),
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
"error":"file_already_locked","holder":holder,"expiresAt":holder_expires,"minutesLeft":minutes_left}),
        );
    }
    let lock_id = Uuid::new_v4().to_string();
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
"INSERT INTO locks (id, path, agent, owner_id, locked_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",params![lock_id.clone(),
path.clone(),agent.clone(),owner_id,now_iso(),expires_at.clone()],)
    } else {
        conn.execute(
"INSERT INTO locks (id, path, agent, locked_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",params![lock_id.clone(),path.clone(),
agent.clone(),now_iso(),expires_at.clone()],)
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "lock",
                json!({
"action":"acquired","path":path,"agent":agent}),
            );
            json_response(
                StatusCode::OK,
                json!({"locked":true,"lockId":lock_id,"expiresAt":
expires_at}),
            )
        }
        Err(err) => {
            if is_unique_constraint(&err) {
                json_response(
                    StatusCode::CONFLICT,
                    json!({"error":"file_already_locked",
"message":"Another lock was acquired for this path while your request was in flight"}),
                )
            } else {
                json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error":format!("Lock failed: {err}")}),
                )
            }
        }
    }
}
pub async fn handle_unlock(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<LockRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let path = match trimmed_non_empty(body.path) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: path, agent"),
    };
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: path, agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
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
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                json!({"error":
"no_lock_found"}),
            )
        }
    };
    if holder != agent {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"error":"not_lock_holder","holder":holder}
            ),
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
        json!({"action":"released","path":path,"agent":agent}),
    );
    json_response(StatusCode::OK, json!({"unlocked":true}))
}
pub async fn handle_locks(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;
    match fetch_locks(&conn, owner_id) {
        Ok(locks) => json_response(StatusCode::OK, json!({"locks":locks})),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Get locks failed: {err}")}),
        ),
    }
}
