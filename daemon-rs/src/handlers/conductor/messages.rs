// SPDX-License-Identifier: MIT
use super::*;
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{ensure_auth_rated, json_response, now_iso};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde_json::json;
use uuid::Uuid;
pub async fn handle_post_message(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<MessageRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let from = match trimmed_non_empty(body.from) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: from, to, message"),
    };
    let to = match trimmed_non_empty(body.to) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: from, to, message"),
    };
    let message = match body.message {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: from, to, message" }));
        }
    };
    let id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_messages(&conn, &to);
    let owner_id = owner_id_from_headers(&headers, &state);
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
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Post message failed: {err}") })),
    }
}
pub async fn handle_get_messages(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<MessagesQuery>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let agent = match trimmed_non_empty(query.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing parameter: agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;
    match fetch_messages_for_agent(&conn, &agent, owner_id) {
        Ok(messages) => json_response(StatusCode::OK, json!({ "messages": messages })),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Get messages failed: {err}") })),
    }
}
