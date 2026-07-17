use super::*;
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use uuid::Uuid;
pub async fn handle_session_start(
    State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionStartRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required field: agent"),
    };
    if !is_valid_agent_label(&agent) {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Invalid agent label"}));
    }
    let ttl = bounded_ttl_seconds(body.ttl, SESSION_TTL_SECONDS);
    let owner_id = owner_id_from_headers(&headers, &state);
    let now = Utc::now();
    let session_id = Uuid::new_v4().to_string();
    let started_at = now.to_rfc3339();
    let expires_at = (now + Duration::seconds(ttl)).to_rfc3339();
    let files_json = serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
    let conn = state.db.lock().await;
    let _ = clean_expired_sessions(&conn, owner_id);
    let should_freshen = should_run_session_freshen(&conn, owner_id, now);
    let write =
        if let Some(owner_id) = owner_id {
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
               expires_at = excluded.expires_at"
,params![agent.clone(),owner_id,session_id.clone(),body.project.clone(),files_json,body.description.clone(),started_at,expires_at]
,)
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
            if should_freshen {
                run_session_freshen(&conn, &state, owner_id);
            }
            checkpoint_wal_best_effort(&conn);
            state.emit("session", json!({"action":"started","agent":agent,"project":body.project}));
            json_response(
                StatusCode::OK,
                json!({"sessionId":session_id,
"heartbeatInterval":60,"freshened":should_freshen}),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":
format!("Session start failed: {err}")}),
        ),
    }
}
pub async fn handle_session_heartbeat(
    State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionHeartbeatRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let agent = body.agent.unwrap_or_default().trim().to_string();
    if !is_valid_agent_label(&agent) {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Missing or invalid required field: agent"}));
    }
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db.lock().await;
    let _ = clean_expired_sessions(&conn, owner_id);
    let exists = if let Some(owner_id) = owner_id {
        conn.query_row("SELECT session_id FROM sessions WHERE owner_id = ?1 AND agent = ?2", params![owner_id, agent.clone()], |row| {
            row.get::<_, String>(0)
        })
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row("SELECT session_id FROM sessions WHERE agent = ?1", params![agent.clone()], |row| row.get::<_, String>(0))
            .optional()
            .ok()
            .flatten()
    };
    if exists.is_none() {
        return json_response(
            StatusCode::NOT_FOUND,
            json!({
"error":"no_active_session"}),
        );
    }
    let now = Utc::now();
    let expires_at = (now + Duration::seconds(SESSION_TTL_SECONDS)).to_rfc3339();
    let files_json = body.files.as_ref().map(|f| serde_json::to_string(f).unwrap_or_else(|_| "[]".to_string()));
    let update = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE sessions SET
               last_heartbeat = ?1,
               expires_at = ?2,
               files_json = CASE WHEN ?3 IS NULL THEN files_json ELSE ?3 END,
               description = CASE WHEN ?4 IS NULL THEN description ELSE ?4 END
             WHERE owner_id = ?5 AND agent = ?6",
            params![now.to_rfc3339(), expires_at.clone(), files_json, body.description, owner_id, agent],
        )
    } else {
        conn.execute(
            "UPDATE sessions SET
               last_heartbeat = ?1,
               expires_at = ?2,
               files_json = CASE WHEN ?3 IS NULL THEN files_json ELSE ?3 END,
               description = CASE WHEN ?4 IS NULL THEN description ELSE ?4 END
             WHERE agent = ?5",
            params![now.to_rfc3339(), expires_at.clone(), files_json, body.description, agent],
        )
    };
    match update {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            json_response(StatusCode::OK, json!({"renewed":true,"expiresAt":expires_at}))
        }
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":format!("Session heartbeat failed: {err}")})),
    }
}
pub async fn handle_session_end(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionEndRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required field: agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db.lock().await;
    let deleted = if let Some(owner_id) = owner_id {
        conn.execute("DELETE FROM sessions WHERE owner_id = ?1 AND agent = ?2", params![owner_id, agent.clone()])
    } else {
        conn.execute("DELETE FROM sessions WHERE agent = ?1", params![agent.clone()])
    };
    match deleted {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit("session", json!({"action":"ended","agent":agent}));
            json_response(
                StatusCode::OK,
                json!
({"ended":true}),
            )
        }
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":format!("Session end failed: {err}")})),
    }
}
pub async fn handle_sessions(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;
    match fetch_sessions(&conn, owner_id) {
        Ok(sessions) => json_response(StatusCode::OK, json!({"sessions":sessions})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":format!("Get sessions failed: {err}")})),
    }
}
