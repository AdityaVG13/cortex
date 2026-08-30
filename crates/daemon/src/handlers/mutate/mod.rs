mod types;
pub(crate) use types::*;

use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub fn parse_conflict_id(raw: &str) -> Option<(i64, i64)> {
    let payload = raw.trim().strip_prefix("decision:").or_else(|| raw.trim().strip_prefix("decision_pair:")).unwrap_or(raw.trim());
    let mut parts = payload.split(':');
    let a = parts.next()?.trim().parse::<i64>().ok()?;
    let b = parts.next()?.trim().parse::<i64>().ok()?;
    parts.next().is_none().then_some((a.min(b), a.max(b)))
}

pub(crate) fn normalize_conflict_classification(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_uppercase();
    matches!(normalized.as_str(), "AGREES" | "CONTRADICTS" | "REFINES" | "UNRELATED").then_some(normalized)
}

pub fn list_permissions(conn: &Connection, owner_id: i64) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT client_id, permission, scope, granted_by, granted_at FROM client_permissions WHERE owner_id = ?1 ORDER BY client_id, permission, scope",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![owner_id], |row| {
            Ok(json!({"client":row.get::<_,String>(0)?,"permission":row.get::<_,String>(1)?,"scope":row.get::<_,String>(2)?,
                "grantedBy":row.get::<_,String>(3)?,"grantedAt":row.get::<_,String>(4)?}))
        })
        .map_err(|err| err.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn grant_permission(conn: &Connection, owner_id: i64, client: &str, permission: &str, scope: &str, granted_by: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by, granted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(owner_id, client_id, permission, scope) DO UPDATE SET granted_by = excluded.granted_by, granted_at = excluded.granted_at",
        params![owner_id, client, permission, scope, granted_by],
    )
    .map(|_| ())
    .map_err(|err| err.to_string())
}

pub fn revoke_permission(conn: &Connection, owner_id: i64, client: &str, permission: &str, scope: &str) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM client_permissions WHERE owner_id = ?1 AND client_id = ?2 AND permission = ?3 AND scope = ?4",
        params![owner_id, client, permission, scope],
    )
    .map_err(|err| err.to_string())
}

pub fn list_conflicts_payload(_conn: &Connection, options: &ConflictListOptions) -> Result<Value, String> {
    Ok(json!({"statusFilter":options.status.as_str(),"classificationFilter":options.classification,"conflictIdFilter":options.conflict_id,
        "openCount":0,"resolvedCount":0,"count":0,"pairs":[],"conflicts":[],"conflict":Value::Null}))
}

pub fn forget_keyword_scoped(conn: &mut Connection, keyword: &str, owner_id: Option<i64>) -> Result<usize, String> {
    let pattern = format!("%{}%", keyword.to_lowercase());
    let updated = if let Some(owner_id) = owner_id {
        conn.execute("UPDATE memories SET score = score * 0.3 WHERE owner_id = ?2 AND lower(text) LIKE ?1", params![pattern, owner_id])
    } else {
        conn.execute("UPDATE memories SET score = score * 0.3 WHERE lower(text) LIKE ?1", params![pattern])
    };
    updated.map_err(|err| err.to_string())
}

pub fn resolve_decision_with_metadata(
    conn: &mut Connection, keep_id: i64, action: &str, superseded_id: Option<i64>, _metadata: ResolutionMetadata,
) -> Result<Value, String> {
    if !matches!(action, "keep" | "merge" | "archive") {
        return Err("Invalid action. Expected keep, merge, or archive.".to_string());
    }
    let status = if action == "archive" { "archived" } else { "active" };
    conn.execute("UPDATE decisions SET status = ?2, disputes_id = NULL, updated_at = datetime('now') WHERE id = ?1", params![keep_id, status])
        .map_err(|err| err.to_string())?;
    if let Some(other) = superseded_id {
        let other_status = if action == "keep" { "superseded" } else { status };
        let _ = conn.execute("UPDATE decisions SET status = ?2, disputes_id = NULL, updated_at = datetime('now') WHERE id = ?1", params![other, other_status]);
    }
    Ok(json!({"resolved":true,"keepId":keep_id,"winnerId":keep_id,"supersededId":superseded_id,"action":action}))
}

async fn auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    ensure_auth_rated(headers, state).await.map(|_| ())
}

pub async fn handle_forget(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ForgetRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let keyword = body.keyword.or(body.source).unwrap_or_default();
    if keyword.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Missing field: keyword"}));
    }
    let mut conn = state.db.lock().await;
    match forget_keyword_scoped(&mut conn, keyword.trim(), None) {
        Ok(affected) => json_response(StatusCode::OK, json!({"affected":affected})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}

pub async fn handle_resolve(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ResolveRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let keep_id = body.keep_id.or_else(|| body.conflict_id.as_deref().and_then(parse_conflict_id).map(|pair| pair.0));
    let Some(keep_id) = keep_id else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Missing fields: keepId, action"}));
    };
    let action = body.action.as_deref().unwrap_or("").trim();
    if action.is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Missing fields: keepId, action"}));
    }
    let mut conn = state.db.lock().await;
    match resolve_decision_with_metadata(&mut conn, keep_id, action, body.superseded_id, ResolutionMetadata::default()) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}

pub async fn handle_conflicts(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<ConflictListQuery>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let options = match ConflictListOptions::from_query(query) {
        Ok(options) => options,
        Err(err) => return json_response(StatusCode::BAD_REQUEST, json!({"error":err})),
    };
    let conn = state.db_read.lock().await;
    match list_conflicts_payload(&conn, &options) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}

pub async fn handle_archive(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ArchiveRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    json_response(StatusCode::OK, json!({"archived":body.ids.unwrap_or_default().len(),"table":body.table.unwrap_or_default()}))
}

pub async fn handle_shutdown(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if let Some(tx) = state.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }
    json_response(StatusCode::OK, json!({"shuttingDown":true}))
}

pub async fn handle_permissions_list(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    match list_permissions(&conn, 0) {
        Ok(permissions) => json_response(StatusCode::OK, json!({"permissions":permissions})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}

pub async fn handle_permissions_grant(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<PermissionGrantRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let client = body.client.unwrap_or_default();
    let permission = body.permission.unwrap_or_else(|| "read".to_string());
    let scope = body.scope.unwrap_or_else(|| "*".to_string());
    match grant_permission(&conn, 0, &client, &permission, &scope, body.granted_by.as_deref().unwrap_or("http")) {
        Ok(()) => json_response(StatusCode::OK, json!({"granted":true})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}

pub async fn handle_permissions_revoke(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<PermissionRevokeRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    match revoke_permission(&conn, 0, body.client.as_deref().unwrap_or(""), body.permission.as_deref().unwrap_or("read"), body.scope.as_deref().unwrap_or("*"))
    {
        Ok(revoked) => json_response(StatusCode::OK, json!({"revoked":revoked})),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    }
}
