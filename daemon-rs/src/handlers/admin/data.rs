// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde_json::{json, Value};

use crate::handlers::{ensure_admin, ensure_auth_rated, json_error, json_response};
use crate::state::RuntimeState;

use super::types::{
    is_allowed_table, ArchiveBody, AssignOwnerBody, SetVisibilityBody, OWNER_TABLES,
    VISIBILITY_TABLES,
};

pub async fn handle_unowned(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    let mut unowned = serde_json::Map::new();
    for table in OWNER_TABLES {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE owner_id IS NULL");
        let count: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap_or(0);
        unowned.insert(table.to_string(), json!(count));
    }

    json_response(StatusCode::OK, json!({ "unowned": unowned }))
}

pub async fn handle_assign_owner(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<AssignOwnerBody>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
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
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin(&headers, &state, &conn) {
        return resp;
    }

    if !["private", "team", "shared"].contains(&body.visibility.as_str()) {
        return json_error(
            StatusCode::BAD_REQUEST,
            "visibility must be private, team, or shared",
        );
    }

    if !is_allowed_table(&body.table, VISIBILITY_TABLES) {
        return json_error(StatusCode::BAD_REQUEST, "table not in visibility allowlist");
    }

    if body.ids.is_empty() {
        return json_response(StatusCode::OK, json!({ "updated": 0 }));
    }

    let placeholders: Vec<String> = body
        .ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect();
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
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let updated = conn.execute(&sql, params_ref.as_slice()).unwrap_or(0);

    json_response(StatusCode::OK, json!({ "updated": updated }))
}

pub async fn handle_archive(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ArchiveBody>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
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

    let placeholders: Vec<String> = body
        .ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect();
    let sql = format!(
        "UPDATE {} SET status = 'archived' WHERE id IN ({})",
        body.table,
        placeholders.join(", ")
    );

    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = body
        .ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let params_ref: Vec<&dyn rusqlite::types::ToSql> =
        param_values.iter().map(|p| p.as_ref()).collect();

    let archived = conn.execute(&sql, params_ref.as_slice()).unwrap_or(0);

    json_response(StatusCode::OK, json!({ "archived": archived }))
}

pub async fn handle_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
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
        "events",
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
