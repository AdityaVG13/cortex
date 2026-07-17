use super::types::{TeamCreateBody, TeamMemberBody, TeamRemoveMemberBody};
use crate::handlers::{ensure_admin, ensure_auth_rated, json_error, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde_json::json;
pub async fn handle_team_create(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TeamCreateBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let name = body.name.trim();
    if name.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "name is required");
    }
    let result = conn.execute("INSERT INTO teams (name) VALUES (?1)", params![name]);
    match result {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") {
                return json_error(StatusCode::CONFLICT, "team name already exists");
            }
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg);
        }
    }
    let team_id = conn.last_insert_rowid();
    json_response(StatusCode::OK, json!({"team_id":team_id,"name":name}))
}
pub async fn handle_team_add_member(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TeamMemberBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let team_id: i64 = match conn.query_row("SELECT id FROM teams WHERE name = ?1", params![body.team_name], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "team not found"),
    };
    let user_id: i64 = match conn.query_row("SELECT id FROM users WHERE username = ?1", params![body.username], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };
    let role = body.role.as_deref().unwrap_or("member");
    if !["admin", "member"].contains(&role) {
        return json_error(StatusCode::BAD_REQUEST, "team role must be admin or member");
    }
    let result = conn.execute("INSERT INTO team_members (team_id, user_id, role) VALUES (?1, ?2, ?3)", params![team_id, user_id, role]);
    match result {
        Ok(_) => {}
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("PRIMARY KEY") {
                return json_error(StatusCode::CONFLICT, "user is already a member of this team");
            }
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &msg);
        }
    }
    json_response(StatusCode::OK, json!({"team":body.team_name,"username":body.username,"role":role,}))
}
pub async fn handle_team_remove_member(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TeamRemoveMemberBody>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let team_id: i64 = match conn.query_row("SELECT id FROM teams WHERE name = ?1", params![body.team_name], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "team not found"),
    };
    let user_id: i64 = match conn.query_row("SELECT id FROM users WHERE username = ?1", params![body.username], |row| row.get(0)) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };
    let deleted = conn.execute("DELETE FROM team_members WHERE team_id = ?1 AND user_id = ?2", params![team_id, user_id]).unwrap_or(0);
    if deleted == 0 {
        return json_error(StatusCode::NOT_FOUND, "membership not found");
    }
    json_response(StatusCode::OK, json!({"removed":{"team":body.team_name,"username":body.username,}}))
}
pub async fn handle_team_list(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }
    let mut stmt = match conn.prepare(
        "SELECT t.id, t.name, COUNT(tm.user_id) as member_count, t.created_at
         FROM teams t
         LEFT JOIN team_members tm ON tm.team_id = t.id
         GROUP BY t.id",
    ) {
        Ok(s) => s,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let teams: Vec<serde_json::Value> = match stmt.query_map([], |row| {
        Ok(json!({"id":row.get::<_,i64>(0)?,"name":row.get::<_,String>(1)?,"member_count":row.get::<_,i64>(2)?,
"created_at":row.get::<_,Option<String>>(3)?,}))
    }) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    };
    json_response(StatusCode::OK, json!({"teams":teams}))
}
