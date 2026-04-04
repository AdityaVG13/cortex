use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use crate::state::RuntimeState;

use super::{ensure_admin, json_error, json_response};

// ─── Allowlisted tables for dynamic SQL (prevent injection) ─────────────────

const OWNER_TABLES: &[&str] = &[
    "memories",
    "decisions",
    "memory_clusters",
    "recall_feedback",
    "sessions",
    "locks",
    "tasks",
    "messages",
    "feed",
    "feed_acks",
    "activities",
    "focus_sessions",
];

const VISIBILITY_TABLES: &[&str] = &[
    "memories",
    "decisions",
    "memory_clusters",
    "feed",
];

fn is_allowed_table(table: &str, allowlist: &[&str]) -> bool {
    allowlist.contains(&table)
}

// ─── Request bodies ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UserAddBody {
    pub username: String,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct UsernameBody {
    pub username: String,
}

#[derive(Deserialize)]
pub struct TeamCreateBody {
    pub name: String,
}

#[derive(Deserialize)]
pub struct TeamMemberBody {
    pub team_name: String,
    pub username: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct TeamRemoveMemberBody {
    pub team_name: String,
    pub username: String,
}

#[derive(Deserialize)]
pub struct AssignOwnerBody {
    pub from_user: Option<String>,
    pub to_user: String,
    pub table: Option<String>,
}

#[derive(Deserialize)]
pub struct SetVisibilityBody {
    pub table: String,
    pub ids: Vec<i64>,
    pub visibility: String,
}

#[derive(Deserialize)]
pub struct ArchiveBody {
    pub table: String,
    pub ids: Vec<i64>,
}

// ─── User Management ────────────────────────────────────────────────────────

pub async fn handle_user_add(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<UserAddBody>,
) -> Response {
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

    // Update in-memory key cache
    {
        let mut hashes = state.team_api_key_hashes.write().unwrap();
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

pub async fn handle_user_rotate_key(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<UsernameBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let username = body.username.trim();
    if username.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "username is required");
    }

    let user_id: i64 = match conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![username],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };

    let api_key = crate::auth::generate_ctx_api_key();
    let hash = match crate::auth::hash_api_key_argon2id(&api_key) {
        Ok(h) => h,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    if let Err(e) = conn.execute(
        "UPDATE users SET api_key_hash = ?1 WHERE id = ?2",
        params![hash, user_id],
    ) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }

    // Swap in-memory key cache entry
    {
        let mut hashes = state.team_api_key_hashes.write().unwrap();
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

pub async fn handle_user_remove(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<UsernameBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let username = body.username.trim();
    if username.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "username is required");
    }

    let user_id: i64 = match conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![username],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };

    let _ = conn.execute(
        "DELETE FROM team_members WHERE user_id = ?1",
        params![user_id],
    );
    if let Err(e) = conn.execute("DELETE FROM users WHERE id = ?1", params![user_id]) {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }

    // Remove from in-memory key cache
    {
        let mut hashes = state.team_api_key_hashes.write().unwrap();
        hashes.retain(|(id, _)| *id != user_id);
    }

    json_response(StatusCode::OK, json!({ "removed": username }))
}

pub async fn handle_user_list(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let mut stmt = match conn.prepare(
        "SELECT id, username, display_name, role, created_at, last_active_at FROM users",
    ) {
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

// ─── Team Management ────────────────────────────────────────────────────────

pub async fn handle_team_create(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TeamCreateBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let name = body.name.trim();
    if name.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "name is required");
    }

    let result = conn.execute(
        "INSERT INTO teams (name) VALUES (?1)",
        params![name],
    );

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

    json_response(
        StatusCode::OK,
        json!({ "team_id": team_id, "name": name }),
    )
}

pub async fn handle_team_add_member(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TeamMemberBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let team_id: i64 = match conn.query_row(
        "SELECT id FROM teams WHERE name = ?1",
        params![body.team_name],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "team not found"),
    };

    let user_id: i64 = match conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![body.username],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };

    let role = body.role.as_deref().unwrap_or("member");
    if !["admin", "member"].contains(&role) {
        return json_error(StatusCode::BAD_REQUEST, "team role must be admin or member");
    }

    let result = conn.execute(
        "INSERT INTO team_members (team_id, user_id, role) VALUES (?1, ?2, ?3)",
        params![team_id, user_id, role],
    );

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

    json_response(
        StatusCode::OK,
        json!({
            "team": body.team_name,
            "username": body.username,
            "role": role,
        }),
    )
}

pub async fn handle_team_remove_member(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TeamRemoveMemberBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let team_id: i64 = match conn.query_row(
        "SELECT id FROM teams WHERE name = ?1",
        params![body.team_name],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "team not found"),
    };

    let user_id: i64 = match conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![body.username],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "user not found"),
    };

    let deleted = conn
        .execute(
            "DELETE FROM team_members WHERE team_id = ?1 AND user_id = ?2",
            params![team_id, user_id],
        )
        .unwrap_or(0);

    if deleted == 0 {
        return json_error(StatusCode::NOT_FOUND, "membership not found");
    }

    json_response(
        StatusCode::OK,
        json!({
            "removed": {
                "team": body.team_name,
                "username": body.username,
            }
        }),
    )
}

pub async fn handle_team_list(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    let conn = state.db.lock().await;
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
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "name": row.get::<_, String>(1)?,
            "member_count": row.get::<_, i64>(2)?,
            "created_at": row.get::<_, Option<String>>(3)?,
        }))
    }) {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    };

    json_response(StatusCode::OK, json!({ "teams": teams }))
}

// ─── Data Management ────────────────────────────────────────────────────────

pub async fn handle_unowned(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let mut unowned = serde_json::Map::new();
    for table in OWNER_TABLES {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE owner_id IS NULL");
        let count: i64 = conn
            .query_row(&sql, [], |row| row.get(0))
            .unwrap_or(0);
        unowned.insert(table.to_string(), json!(count));
    }

    json_response(StatusCode::OK, json!({ "unowned": unowned }))
}

pub async fn handle_assign_owner(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<AssignOwnerBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let to_id: i64 = match conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![body.to_user],
        |row| row.get(0),
    ) {
        Ok(id) => id,
        Err(_) => return json_error(StatusCode::NOT_FOUND, "to_user not found"),
    };

    let from_id: Option<i64> = if let Some(ref from_user) = body.from_user {
        match conn.query_row(
            "SELECT id FROM users WHERE username = ?1",
            params![from_user],
            |row| row.get(0),
        ) {
            Ok(id) => Some(id),
            Err(_) => return json_error(StatusCode::NOT_FOUND, "from_user not found"),
        }
    } else {
        None
    };

    let tables: Vec<&str> = if let Some(ref t) = body.table {
        if !is_allowed_table(t, OWNER_TABLES) {
            return json_error(StatusCode::BAD_REQUEST, "table not in allowlist");
        }
        vec![t.as_str()]
    } else {
        OWNER_TABLES.to_vec()
    };

    let mut assigned = serde_json::Map::new();
    for table in tables {
        let count = if let Some(fid) = from_id {
            conn.execute(
                &format!("UPDATE {table} SET owner_id = ?1 WHERE owner_id = ?2"),
                params![to_id, fid],
            )
            .unwrap_or(0)
        } else {
            conn.execute(
                &format!("UPDATE {table} SET owner_id = ?1 WHERE owner_id IS NULL"),
                params![to_id],
            )
            .unwrap_or(0)
        };
        assigned.insert(table.to_string(), json!(count));
    }

    json_response(StatusCode::OK, json!({ "assigned": assigned }))
}

pub async fn handle_set_visibility(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<SetVisibilityBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    if !["private", "team", "shared"].contains(&body.visibility.as_str()) {
        return json_error(StatusCode::BAD_REQUEST, "visibility must be private, team, or shared");
    }

    if !is_allowed_table(&body.table, VISIBILITY_TABLES) {
        return json_error(StatusCode::BAD_REQUEST, "table not in visibility allowlist");
    }

    if body.ids.is_empty() {
        return json_response(StatusCode::OK, json!({ "updated": 0 }));
    }

    let placeholders: Vec<String> = body.ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect();
    let sql = format!(
        "UPDATE {} SET visibility = ?1 WHERE id IN ({})",
        body.table,
        placeholders.join(", ")
    );

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(body.visibility.clone()));
    for id in &body.ids {
        param_values.push(Box::new(*id));
    }
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let updated = conn.execute(&sql, params_ref.as_slice()).unwrap_or(0);

    json_response(StatusCode::OK, json!({ "updated": updated }))
}

pub async fn handle_archive(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ArchiveBody>,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    // Only tables with a status column make sense for archiving
    const ARCHIVABLE: &[&str] = &["memories", "decisions"];

    if !is_allowed_table(&body.table, ARCHIVABLE) {
        return json_error(StatusCode::BAD_REQUEST, "table not archivable");
    }

    if body.ids.is_empty() {
        return json_response(StatusCode::OK, json!({ "archived": 0 }));
    }

    let placeholders: Vec<String> = body.ids.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect();
    let sql = format!(
        "UPDATE {} SET status = 'archived' WHERE id IN ({})",
        body.table,
        placeholders.join(", ")
    );

    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = body.ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

    let archived = conn.execute(&sql, params_ref.as_slice()).unwrap_or(0);

    json_response(StatusCode::OK, json!({ "archived": archived }))
}

pub async fn handle_stats(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let user_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap_or(0);

    let team_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM teams", [], |row| row.get(0))
        .unwrap_or(0);

    // Per-table row counts
    let table_names = [
        "memories", "decisions", "memory_clusters", "recall_feedback",
        "sessions", "locks", "tasks", "messages", "feed", "feed_acks",
        "activities", "focus_sessions", "events",
    ];
    let mut tables = serde_json::Map::new();
    for table in &table_names {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let count: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap_or(0);
        tables.insert(table.to_string(), json!(count));
    }

    // Per-user counts for core tables
    let mut per_user = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT u.id, u.username,
                    (SELECT COUNT(*) FROM memories WHERE owner_id = u.id),
                    (SELECT COUNT(*) FROM decisions WHERE owner_id = u.id),
                    (SELECT COUNT(*) FROM memory_clusters WHERE owner_id = u.id)
                 FROM users u",
            )
            .ok();
        if let Some(ref mut s) = stmt {
            if let Ok(rows) = s.query_map([], |row| {
                Ok(json!({
                    "user_id": row.get::<_, i64>(0)?,
                    "username": row.get::<_, String>(1)?,
                    "memories": row.get::<_, i64>(2)?,
                    "decisions": row.get::<_, i64>(3)?,
                    "crystals": row.get::<_, i64>(4)?,
                }))
            }) {
                for row in rows.flatten() {
                    per_user.push(row);
                }
            }
        }
    }

    // DB file size
    let db_size = std::fs::metadata(&state.db_path)
        .map(|m| m.len())
        .unwrap_or(0);

    json_response(
        StatusCode::OK,
        json!({
            "user_count": user_count,
            "team_count": team_count,
            "tables": tables,
            "per_user": per_user,
            "db_size_bytes": db_size,
            "db_size_mb": format!("{:.1}", db_size as f64 / 1_048_576.0),
        }),
    )
}
