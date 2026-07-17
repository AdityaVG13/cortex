// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{
    ensure_auth_rated, json_response, now_iso, parse_duration_to_seconds, parse_json_array,
    parse_timestamp_ms, redact_secrets, resolve_caller_id,
};
use crate::state::RuntimeState;


use super::*;
// ─── POST /activity ─────────────────────────────────────────────────────────

pub async fn handle_post_activity(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ActivityRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: agent, description"),
    };
    let description = match trimmed_non_empty(body.description) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: agent, description"),
    };

    let files = body.files.unwrap_or_default();
    let id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_activities(&conn);
    let owner_id = owner_id_from_headers(&headers, &state);
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
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let since_secs = parse_duration_to_seconds(query.since.as_deref().unwrap_or("1h"));
    let cutoff = (Utc::now() - Duration::seconds(since_secs)).to_rfc3339();
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;

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
            );
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

