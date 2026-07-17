use super::*;
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{
    ensure_auth_rated, json_response, now_iso, parse_json_array, redact_secrets,
};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use uuid::Uuid;
pub async fn handle_create_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskCreateRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let title = match trimmed_non_empty(body.title) {
        Some(v) => v,
        None => return missing_field_response("Missing required field: title"),
    };
    let task_id = Uuid::new_v4().to_string();
    let conn = state.db.lock().await;
    let _ = clean_old_tasks(&conn);
    let files_json =
        serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
    let owner_id = owner_id_from_headers(&headers, &state);
    let insert = if let Some(owner_id) = owner_id {
        conn.execute(
"INSERT INTO tasks (task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary, owner_id, visibility)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, NULL, NULL, NULL, ?9, 'private')"
,params![task_id.clone(),title.clone(),body.description,body.project,files_json,body.priority.unwrap_or_else(||"medium".to_string(
)),body.required_capability.unwrap_or_else(||"any".to_string()),now_iso(),owner_id],)
    } else {
        conn.execute(
"INSERT INTO tasks (task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, NULL, NULL, NULL)"
,params![task_id.clone(),title.clone(),body.description,body.project,files_json,body.priority.unwrap_or_else(||"medium".to_string(
)),body.required_capability.unwrap_or_else(||"any".to_string()),now_iso()],)
    };
    match insert {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({"action":"created","taskId":task_id,"title":title}),
            );
            json_response(
                StatusCode::CREATED,
                json!({
"taskId":task_id,"status":"pending"}),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!(
"Create task failed: {err}")}),
        ),
    }
}
pub async fn handle_get_tasks(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<TaskQuery>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let status_filter = query.status.unwrap_or_else(|| "pending".to_string());
    let project_filter = query.project;
    let requested_limit = query.limit.unwrap_or(DEFAULT_TASK_QUERY_LIMIT);
    let limit = requested_limit.clamp(1, MAX_TASK_QUERY_LIMIT);
    let offset = query.offset.unwrap_or(0);
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;
    match fetch_tasks(
        &conn,
        &status_filter,
        project_filter.as_deref(),
        owner_id,
        limit,
        offset,
    ) {
        Ok(tasks) => json_response(StatusCode::OK, json!({"tasks":tasks})),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Get tasks failed: {err}")}),
        ),
    }
}
pub async fn handle_claim_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskClaimRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let task_id = match trimmed_non_empty(body.task_id) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
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
        None => return json_response(StatusCode::NOT_FOUND, json!({"error":"task_not_found"})),
    };
    if status == "claimed" {
        return json_response(
            StatusCode::CONFLICT,
            json!({"error":"task_already_claimed","claimedBy":claimed_by}),
        );
    }
    if status == "completed" {
        return json_response(
            StatusCode::CONFLICT,
            json!({"error":"task_already_completed"}),
        );
    }
    let claim = if let Some(owner_id) = owner_id {
        conn.execute(
"UPDATE tasks SET status = 'claimed', claimed_by = ?1, claimed_at = ?2 WHERE owner_id = ?3 AND task_id = ?4",params![agent.clone()
,now_iso(),owner_id,task_id.clone()],)
    } else {
        conn.execute(
"UPDATE tasks SET status = 'claimed', claimed_by = ?1, claimed_at = ?2 WHERE task_id = ?3",params![agent.clone(),now_iso(),task_id
.clone()],)
    };
    match claim {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({"action":"claimed","taskId":task_id,
"title":title,"agent":agent}),
            );
            json_response(StatusCode::OK, json!({"claimed":true,"taskId":task_id}))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Claim task failed: {err}")}),
        ),
    }
}
pub async fn handle_complete_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskCompleteRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let task_id = match trimmed_non_empty(body.task_id) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
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
        None => return json_response(StatusCode::NOT_FOUND, json!({"error":"task_not_found"})),
    };
    if claimed_by.as_deref() != Some(agent.as_str()) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"error":"not_task_holder","claimedBy":claimed_by}),
        );
    }
    let complete = if let Some(owner_id) = owner_id {
        conn.execute(
"UPDATE tasks SET status = 'completed', completed_at = ?1, summary = ?2 WHERE owner_id = ?3 AND task_id = ?4",params![now_iso(),
body.summary.clone(),owner_id,task_id.clone()],)
    } else {
        conn.execute(
"UPDATE tasks SET status = 'completed', completed_at = ?1, summary = ?2 WHERE task_id = ?3",params![now_iso(),body.summary.clone()
,task_id.clone()],)
    };
    match complete {
        Ok(_) => {
            state.emit(
                "task",
                json!({"action":"completed","taskId":task_id,"title":title,"agent":
agent}),
            );
            let posted: i64 = if let Some(owner_id) = owner_id {
                conn.query_row(
"SELECT COUNT(*) FROM feed WHERE owner_id = ?1 AND task_id = ?2 AND kind = 'task_complete'",params![owner_id,task_id.clone()],|r|r
.get(0),).unwrap_or(0)
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
                    let _=conn.execute(
"INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens, owner_id, visibility)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'team')"
,params![feed_id.clone(),agent.clone(),"task_complete",summary_text.clone(),content_text.clone(),files.to_string(),task_id.clone()
,Option::<String>::None,"normal",ts,tokens,owner_id],);
                } else {
                    let _=conn.execute(
"INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
,params![feed_id.clone(),agent.clone(),"task_complete",summary_text.clone(),content_text.clone(),files.to_string(),task_id.clone()
,Option::<String>::None,"normal",ts,tokens],);
                }
                state.emit(
                    "feed",
                    json!({"feedId":feed_id,"agent":agent,"kind":"task_complete",
"summary":summary_text}),
                );
            }
            checkpoint_wal_best_effort(&conn);
            json_response(
                StatusCode::OK,
                json!({"completed":true,"taskId":task_id
                }),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Complete task failed: {err}")}),
        ),
    }
}
pub async fn handle_delete_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskDeleteRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let task_id = match trimmed_non_empty(body.task_id) {
        Some(v) => v,
        None => return missing_field_response("Missing required field: taskId"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db.lock().await;
    let title = if let Some(owner_id) = owner_id {
        conn.query_row(
            "SELECT title FROM tasks WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    } else {
        conn.query_row(
            "SELECT title FROM tasks WHERE task_id = ?1",
            params![task_id.clone()],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
    };
    let title = match title {
        Some(v) => v,
        None => {
            return json_response(
                StatusCode::NOT_FOUND,
                json!({
"error":"task_not_found"}),
            )
        }
    };
    let delete = if let Some(owner_id) = owner_id {
        conn.execute(
            "DELETE FROM tasks WHERE owner_id = ?1 AND task_id = ?2",
            params![owner_id, task_id.clone()],
        )
    } else {
        conn.execute(
            "DELETE FROM tasks WHERE task_id = ?1",
            params![task_id.clone()],
        )
    };
    match delete {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({"action":"deleted","taskId":task_id,"title":title}),
            );
            json_response(
                StatusCode::OK,
                json!({"deleted":true,
"taskId":task_id}),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Delete task failed: {err}")}
            ),
        ),
    }
}
pub async fn handle_abandon_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<TaskAbandonRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let task_id = match trimmed_non_empty(body.task_id) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let agent = match trimmed_non_empty(body.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing required fields: taskId, agent"),
    };
    let owner_id = owner_id_from_headers(&headers, &state);
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
        None => return json_response(StatusCode::NOT_FOUND, json!({"error":"task_not_found"})),
    };
    if claimed_by.as_deref() != Some(agent.as_str()) {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"error":"not_task_holder","claimedBy":claimed_by}),
        );
    }
    let abandon = if let Some(owner_id) = owner_id {
        conn.execute(
"UPDATE tasks SET status = 'pending', claimed_by = NULL, claimed_at = NULL WHERE owner_id = ?1 AND task_id = ?2",params![owner_id,
task_id.clone()],)
    } else {
        conn.execute(
"UPDATE tasks SET status = 'pending', claimed_by = NULL, claimed_at = NULL WHERE task_id = ?1",params![task_id.clone()])
    };
    match abandon {
        Ok(_) => {
            checkpoint_wal_best_effort(&conn);
            state.emit(
                "task",
                json!({"action":"abandoned","taskId":task_id,"title":title,
"agent":agent}),
            );
            json_response(
                StatusCode::OK,
                json!({"abandoned":true,"taskId":task_id,"status":"pending"}),
            )
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":format!("Abandon task failed: {err}")}),
        ),
    }
}
pub async fn handle_next_task(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<NextTaskQuery>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let _agent = match trimmed_non_empty(query.agent) {
        Some(v) => v,
        None => return missing_field_response("Missing parameter: agent"),
    };
    let capability = query.capability.unwrap_or_else(|| "any".to_string());
    let owner_id = owner_id_from_headers(&headers, &state);
    let conn = state.db_read.lock().await;
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
                json!({
"error":format!("Get next task failed: {err}")}),
            );
        }
    };
    let task = if let Some(owner_id) = owner_id {
        stmt.query_row(params![capability, owner_id], task_row_to_json)
            .optional()
            .ok()
            .flatten()
    } else {
        stmt.query_row(params![capability], task_row_to_json)
            .optional()
            .ok()
            .flatten()
    };
    json_response(StatusCode::OK, json!({"task":task}))
}
