// SPDX-License-Identifier: MIT
use super::types::{UserAddBody, UsernameBody};
use crate::handlers::{ensure_admin, ensure_auth_rated, json_error, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde_json::json;
pub async fn handle_user_add(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<UserAddBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let username = body.username.trim();
    if username.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "username is required");
    }
    let role = body.role.as_deref().unwrap_or("member");
    if !["owner", "admin", "member"].contains(&role) {
        return json_error(StatusCode::BAD_REQUEST, "role must be owner, admin, or member");
    }
    let api_key = crate::auth::generate_ctx_api_key();
    let hash = match crate::auth::hash_api_key_argon2id(&api_key) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let result = conn.execute(
        "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, ?4)",
        params![username, body.display_name, hash, role],
    );
    match result {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                return json_error(StatusCode::CONFLICT, "username already exists");
            }
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg);
        }
    }
    let user_id: i64 = conn.last_insert_rowid();
    {
        let mut hashes = match state.team_api_key_hashes.write() {
            Ok(hashes) => hashes,
            Err(_) => {
                eprintln!("[cortex] team_api_key_hashes write lock poisoned while adding user");
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, "team auth cache unavailable");
            }
        };
        hashes.push((user_id, hash));
    }
    json_response(
        StatusCode::OK,
        json!({
            "username": username,
            "user_id": user_id,
            "api_key": api_key,
            "role": role,
        }),
    )
}
pub async fn handle_user_rotate_key(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<UsernameBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let username = body.username.trim();
    if username.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "username is required");
    }
    let user_id: i64 = match conn.query_row("SELECT id FROM users WHERE username = ?1", params![username], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };
    let api_key = crate::auth::generate_ctx_api_key();
    let hash = match crate::auth::hash_api_key_argon2id(&api_key) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if let Err(e) = conn.execute("UPDATE users SET api_key_hash = ?1 WHERE id = ?2", params![hash, user_id]) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    {
        let mut hashes = match state.team_api_key_hashes.write() {
            Ok(hashes) => hashes,
            Err(_) => {
                eprintln!("[cortex] team_api_key_hashes write lock poisoned while rotating key");
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, "team auth cache unavailable");
            }
        };
        hashes.retain(|(id, _)| *id != user_id);
        hashes.push((user_id, hash));
    }
    json_response(
        StatusCode::OK,
        json!({
            "username": username,
            "api_key": api_key,
        }),
    )
}
pub async fn handle_user_remove(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<UsernameBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let username = body.username.trim();
    if username.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "username is required");
    }
    let user_id: i64 = match conn.query_row("SELECT id FROM users WHERE username = ?1", params![username], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };
    let _ = conn.execute("DELETE FROM team_members WHERE user_id = ?1", params![user_id]);
    if let Err(e) = conn.execute("DELETE FROM users WHERE id = ?1", params![user_id]) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    {
        let mut hashes = match state.team_api_key_hashes.write() {
            Ok(hashes) => hashes,
            Err(_) => {
                eprintln!("[cortex] team_api_key_hashes write lock poisoned while removing user");
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, "team auth cache unavailable");
            }
        };
        hashes.retain(|(id, _)| *id != user_id);
    }
    json_response(StatusCode::OK, json!({ "removed": username }))
}
pub async fn handle_user_list(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let mut stmt = match conn.prepare("SELECT id, username, display_name, role, created_at, last_active_at FROM users") {
        Ok(s) => s,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let users: Vec<serde_json::Value> = match stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "username": row.get::<_, String>(1)?,
            "display_name": row.get::<_, Option<String>>(2)?,
            "role": row.get::<_, String>(3)?,
            "created_at": row.get::<_, Option<String>>(4)?,
            "last_active_at": row.get::<_, Option<String>>(5)?,
        }))
    }) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    };
    json_response(StatusCode::OK, json!({ "users": users }))
}
