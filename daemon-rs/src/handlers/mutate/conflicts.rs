// SPDX-License-Identifier: MIT
use super::*;
use crate::db::{archive_entries_scoped, checkpoint_wal_best_effort};
use crate::handlers::{ensure_admin, ensure_auth_rated, json_response, log_event, now_iso};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub fn list_conflicts_payload(conn: &Connection, options: &ConflictListOptions) -> Result<Value, String> {
    let mut open_conflicts = if options.status.includes_open() { list_open_conflicts(conn, options.limit)? } else { Vec::new() };
    let mut resolved_conflicts = if options.status.includes_resolved() {
        list_resolved_conflicts(conn, options.limit)?
    } else {
        Vec::new()
    };
    open_conflicts.retain(|entry| conflict_matches_filters(entry, options));
    resolved_conflicts.retain(|entry| conflict_matches_filters(entry, options));
    let mut conflicts = Vec::with_capacity(open_conflicts.len() + resolved_conflicts.len());
    if options.status.includes_open() {
        conflicts.extend(open_conflicts.clone());
    }
    if options.status.includes_resolved() {
        conflicts.extend(resolved_conflicts.clone());
    }
    let pairs: Vec<Value> = open_conflicts.iter().map(legacy_pair_from_conflict).collect();
    let conflict = if options.conflict_id.is_some() {
        conflicts.first().cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Ok(json!({
        "statusFilter": options.status.as_str(),
        "classificationFilter": options.classification,
        "conflictIdFilter": options.conflict_id,
        "openCount": open_conflicts.len(),
        "resolvedCount": resolved_conflicts.len(),
        "count": conflicts.len(),
        "pairs": pairs,
        "conflicts": conflicts,
        "conflict": conflict,
    }))
}
#[allow(clippy::result_large_err)]
pub(crate) fn ensure_admin_surface(headers: &HeaderMap, state: &RuntimeState, conn: &Connection) -> Result<Option<i64>, Response> {
    if state.team_mode {
        ensure_admin(headers, state, conn).map(Some)
    } else {
        Ok(None)
    }
}
pub async fn handle_forget(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ForgetRequest>) -> Response {
    let keyword = body.keyword.or(body.source).unwrap_or_default();
    if keyword.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing field: keyword" }));
    }
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let mut conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    match forget_keyword_scoped(&mut conn, keyword.trim(), owner_id) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "affected": affected })),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Forget failed: {err}") })),
    }
}
pub fn forget_keyword_scoped(conn: &mut Connection, keyword: &str, owner_id: Option<i64>) -> Result<usize, String> {
    let pattern = format!("%{}%", keyword.to_lowercase());
    let now = now_iso();
    let (memories, decisions) = if let Some(owner_id) = owner_id {
        let memories = conn
            .execute(
                "UPDATE memories SET score = score * 0.3, updated_at = ?2 \
                 WHERE owner_id = ?3 AND status = 'active' AND (lower(text) LIKE ?1 OR lower(source) LIKE ?1)",
                params![pattern.clone(), now.clone(), owner_id],
            )
            .map_err(|e| e.to_string())?;
        let decisions = conn
            .execute(
                "UPDATE decisions SET score = score * 0.3, updated_at = ?2 \
                 WHERE owner_id = ?3 AND status = 'active' AND (lower(decision) LIKE ?1 OR lower(context) LIKE ?1)",
                params![pattern, now, owner_id],
            )
            .map_err(|e| e.to_string())?;
        (memories, decisions)
    } else {
        let memories = conn
            .execute(
                "UPDATE memories SET score = score * 0.3, updated_at = ?2 \
                 WHERE status = 'active' AND (lower(text) LIKE ?1 OR lower(source) LIKE ?1)",
                params![pattern.clone(), now.clone()],
            )
            .map_err(|e| e.to_string())?;
        let decisions = conn
            .execute(
                "UPDATE decisions SET score = score * 0.3, updated_at = ?2 \
                 WHERE status = 'active' AND (lower(decision) LIKE ?1 OR lower(context) LIKE ?1)",
                params![pattern, now],
            )
            .map_err(|e| e.to_string())?;
        (memories, decisions)
    };
    let affected = memories + decisions;
    if affected > 0 {
        let _ = log_event(conn, "forget", json!({ "keyword": keyword, "affected": affected, "ownerId": owner_id }), "rust-daemon");
        checkpoint_wal_best_effort(conn);
    }
    Ok(affected)
}
pub async fn handle_resolve(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ResolveRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let mut conn = state.db.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }
    let mut keep_id = body.keep_id;
    let mut superseded_id = body.superseded_id;
    if let Some((a, b)) = body.conflict_id.as_deref().and_then(parse_conflict_id) {
        if keep_id.is_none() {
            keep_id = Some(a);
        }
        if superseded_id.is_none() {
            superseded_id = keep_id.map(|winner| {
                if winner == a {
                    b
                } else if winner == b {
                    a
                } else {
                    b
                }
            });
        }
    }
    let keep_id = match keep_id {
        Some(value) => value,
        _ => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing fields: keepId, action" }));
        }
    };
    let action = match body.action.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => value,
        None => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing fields: keepId, action" }));
        }
    };
    let metadata = ResolutionMetadata {
        conflict_id: body.conflict_id.clone(),
        classification: body.classification.clone(),
        notes: body.notes.clone(),
        resolved_by: body.resolved_by.clone(),
        similarity: body.similarity,
    };
    match resolve_decision_with_metadata(&mut conn, keep_id, action, superseded_id, metadata) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Resolve failed: {err}") })),
    }
}
pub fn resolve_decision(conn: &mut Connection, keep_id: i64, action: &str, superseded_id: Option<i64>) -> Result<(), String> {
    resolve_decision_with_metadata(conn, keep_id, action, superseded_id, ResolutionMetadata::default())?;
    Ok(())
}
pub fn resolve_decision_with_metadata(conn: &mut Connection, keep_id: i64, action: &str, superseded_id: Option<i64>, metadata: ResolutionMetadata) -> Result<Value, String> {
    let resolved_at = now_iso();
    match action {
        "keep" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'superseded', supersedes_id = ?1, disputes_id = NULL, updated_at = ?3 WHERE id = ?2",
                    params![keep_id, other, resolved_at],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        "merge" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute("UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1", params![other, resolved_at])
                    .map_err(|e| e.to_string())?;
            }
        }
        "archive" => {
            conn.execute(
                "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                    params![other, resolved_at],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("Invalid action. Expected keep, merge, or archive.".to_string()),
    }
    let classification = metadata
        .classification
        .as_deref()
        .and_then(normalize_conflict_classification)
        .unwrap_or_else(|| default_classification_for_action(action).to_string());
    let conflict_id = metadata.conflict_id.or_else(|| superseded_id.map(|other| conflict_id_from_pair(keep_id, other)));
    let resolved_by = metadata
        .resolved_by
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rust-daemon".to_string());
    let event_payload = json!({
        "conflictId": conflict_id,
        "keepId": keep_id,
        "winnerId": keep_id,
        "action": action,
        "supersededId": superseded_id,
        "classification": classification,
        "similarity": metadata.similarity,
        "resolvedBy": resolved_by,
        "resolvedAt": resolved_at,
        "notes": metadata.notes,
    });
    let _ = log_event(conn, "decision_resolve", event_payload.clone(), &resolved_by);
    checkpoint_wal_best_effort(conn);
    Ok(json!({
        "resolved": true,
        "conflictId": event_payload.get("conflictId").cloned().unwrap_or(Value::Null),
        "winnerId": keep_id,
        "keepId": keep_id,
        "supersededId": superseded_id,
        "action": action,
        "classification": event_payload.get("classification").cloned().unwrap_or(Value::Null),
        "similarity": event_payload.get("similarity").cloned().unwrap_or(Value::Null),
        "resolvedBy": event_payload.get("resolvedBy").cloned().unwrap_or(Value::Null),
        "resolvedAt": event_payload.get("resolvedAt").cloned().unwrap_or(Value::Null),
        "notes": event_payload.get("notes").cloned().unwrap_or(Value::Null),
    }))
}
pub async fn handle_conflicts(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<ConflictListQuery>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }
    let options = match ConflictListOptions::from_query(query) {
        Ok(options) => options,
        Err(err) => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": err }));
        }
    };
    match list_conflicts_payload(&conn, &options) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Conflict query failed: {err}") })),
    }
}
pub async fn handle_permissions_list(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };
    match list_permissions(&conn, owner_id) {
        Ok(grants) => json_response(
            StatusCode::OK,
            json!({
                "ownerId": owner_id,
                "count": grants.len(),
                "grants": grants,
            }),
        ),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Permission list failed: {err}") })),
    }
}
pub async fn handle_permissions_grant(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<PermissionGrantRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };
    let raw_client = match body.client.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => value,
        None => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing field: client" }));
        }
    };
    let client = if raw_client == "*" {
        "*".to_string()
    } else if let Some(normalized) = normalize_permission_client_id(raw_client) {
        normalized
    } else {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Invalid client id. Use letters, numbers, '-', '_'." }));
    };
    let permission = match body.permission.as_deref().and_then(parse_permission).map(str::to_string) {
        Some(value) => value,
        None => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Invalid permission; expected read, write, or admin" }));
        }
    };
    let scope = normalize_permission_scope(body.scope.as_deref());
    let granted_by = body
        .granted_by
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(normalize_permission_client_id)
        .unwrap_or_else(|| "control-center".to_string());
    match grant_permission(&conn, owner_id, &client, &permission, &scope, &granted_by) {
        Ok(()) => json_response(
            StatusCode::OK,
            json!({
                "granted": true,
                "ownerId": owner_id,
                "client": client,
                "permission": permission,
                "scope": scope,
            }),
        ),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Permission grant failed: {err}") })),
    }
}
pub async fn handle_permissions_revoke(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<PermissionRevokeRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };
    let raw_client = match body.client.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(value) => value,
        None => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing field: client" }));
        }
    };
    let client = if raw_client == "*" {
        "*".to_string()
    } else if let Some(normalized) = normalize_permission_client_id(raw_client) {
        normalized
    } else {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Invalid client id. Use letters, numbers, '-', '_'." }));
    };
    let permission = match body.permission.as_deref().and_then(parse_permission).map(str::to_string) {
        Some(value) => value,
        None => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Invalid permission; expected read, write, or admin" }));
        }
    };
    let scope = normalize_permission_scope(body.scope.as_deref());
    match revoke_permission(&conn, owner_id, &client, &permission, &scope) {
        Ok(deleted) => json_response(
            StatusCode::OK,
            json!({
                "revoked": deleted > 0,
                "deleted": deleted,
                "ownerId": owner_id,
                "client": client,
                "permission": permission,
                "scope": scope,
            }),
        ),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Permission revoke failed: {err}") })),
    }
}
pub async fn handle_archive(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ArchiveRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let table = body.table.unwrap_or_default();
    let ids = body.ids.unwrap_or_default();
    if table.is_empty() || ids.is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing fields: table, ids" }));
    }
    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    match archive_entries_scoped(&conn, &table, &ids, owner_id) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "archived": affected })),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Archive failed: {err}") })),
    }
}
pub async fn handle_shutdown(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }
    checkpoint_wal_best_effort(&conn);
    drop(conn);
    let mut tx_guard = state.shutdown_tx.lock().await;
    if let Some(tx) = tx_guard.take() {
        let _ = tx.send(());
    }
    json_response(StatusCode::OK, json!({ "shutdown": true }))
}
