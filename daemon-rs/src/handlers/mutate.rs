// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::json;

use super::{ensure_auth, json_response, log_event, now_iso};
use crate::db::{archive_entries, checkpoint_wal_best_effort};
use crate::state::RuntimeState;

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ForgetRequest {
    pub keyword: Option<String>,
    pub source: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ResolveRequest {
    #[serde(rename = "keepId")]
    pub keep_id: Option<i64>,
    pub action: Option<String>,
    #[serde(rename = "supersededId")]
    pub superseded_id: Option<i64>,
}

#[derive(Deserialize, Default)]
pub struct ArchiveRequest {
    pub table: Option<String>,
    pub ids: Option<Vec<i64>>,
}

// ─── POST /forget ────────────────────────────────────────────────────────────

pub async fn handle_forget(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ForgetRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let keyword = body.keyword.or(body.source).unwrap_or_default();
    if keyword.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing field: keyword" }),
        );
    }

    let mut conn = state.db.lock().await;
    match forget_keyword(&mut conn, keyword.trim()) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "affected": affected })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Forget failed: {err}") }),
        ),
    }
}

pub fn forget_keyword(conn: &mut Connection, keyword: &str) -> Result<usize, String> {
    let pattern = format!("%{}%", keyword.to_lowercase());
    let now = now_iso();
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
    let affected = memories + decisions;
    if affected > 0 {
        let _ = log_event(
            conn,
            "forget",
            json!({ "keyword": keyword, "affected": affected }),
            "rust-daemon",
        );
        checkpoint_wal_best_effort(conn);
    }
    Ok(affected)
}

// ─── POST /resolve ───────────────────────────────────────────────────────────

pub async fn handle_resolve(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ResolveRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let keep_id = match body.keep_id {
        Some(v) => v,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing fields: keepId, action" }),
            )
        }
    };
    let action = match body.action {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing fields: keepId, action" }),
            )
        }
    };

    let mut conn = state.db.lock().await;
    match resolve_decision(&mut conn, keep_id, &action, body.superseded_id) {
        Ok(()) => json_response(StatusCode::OK, json!({ "resolved": true })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Resolve failed: {err}") }),
        ),
    }
}

pub fn resolve_decision(
    conn: &mut Connection,
    keep_id: i64,
    action: &str,
    superseded_id: Option<i64>,
) -> Result<(), String> {
    match action {
        "keep" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, now_iso()],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'superseded', supersedes_id = ?1, disputes_id = NULL, updated_at = ?3 WHERE id = ?2",
                    params![keep_id, other, now_iso()],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        "merge" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, now_iso()],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                    params![other, now_iso()],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        "archive" => {
            let ts = now_iso();
            conn.execute(
                "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, ts],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                    params![other, ts],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("Invalid action. Expected keep, merge, or archive.".to_string()),
    }
    let _ = log_event(
        conn,
        "decision_resolve",
        json!({ "keepId": keep_id, "action": action, "supersededId": superseded_id }),
        "rust-daemon",
    );
    checkpoint_wal_best_effort(conn);
    Ok(())
}

// ─── GET /conflicts ──────────────────────────────────────────────────────────

pub async fn handle_conflicts(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let conn = state.db.lock().await;
    let mut stmt = match conn.prepare(
        "SELECT d1.id, d1.decision, d1.context, d1.source_agent, d1.confidence, d1.created_at,
                d2.id, d2.decision, d2.context, d2.source_agent, d2.confidence, d2.created_at
         FROM decisions d1
         JOIN decisions d2 ON d1.disputes_id = d2.id
         WHERE d1.status = 'disputed' AND d1.id > d2.id
         ORDER BY d1.created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": format!("Query failed: {e}") }),
            )
        }
    };

    let pairs: Vec<serde_json::Value> = match stmt.query_map([], |row| {
        Ok(json!({
            "left": {
                "id": row.get::<_, i64>(0)?,
                "decision": row.get::<_, String>(1)?,
                "context": row.get::<_, Option<String>>(2)?,
                "source_agent": row.get::<_, Option<String>>(3)?,
                "confidence": row.get::<_, Option<f64>>(4)?,
                "created_at": row.get::<_, Option<String>>(5)?,
            },
            "right": {
                "id": row.get::<_, i64>(6)?,
                "decision": row.get::<_, String>(7)?,
                "context": row.get::<_, Option<String>>(8)?,
                "source_agent": row.get::<_, Option<String>>(9)?,
                "confidence": row.get::<_, Option<f64>>(10)?,
                "created_at": row.get::<_, Option<String>>(11)?,
            },
        }))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };

    let count = pairs.len();
    json_response(StatusCode::OK, json!({ "pairs": pairs, "count": count }))
}

// ─── POST /archive ───────────────────────────────────────────────────────────

pub async fn handle_archive(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ArchiveRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let table = body.table.unwrap_or_default();
    let ids = body.ids.unwrap_or_default();

    if table.is_empty() || ids.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing fields: table, ids" }),
        );
    }

    let conn = state.db.lock().await;
    match archive_entries(&conn, &table, &ids) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "archived": affected })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Archive failed: {err}") }),
        ),
    }
}

// ─── POST /shutdown ──────────────────────────────────────────────────────────

pub async fn handle_shutdown(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    // WAL checkpoint before exiting
    {
        let conn = state.db.lock().await;
        checkpoint_wal_best_effort(&conn);
    }

    // Fire the oneshot shutdown signal
    let mut tx_guard = state.shutdown_tx.lock().await;
    if let Some(tx) = tx_guard.take() {
        let _ = tx.send(());
    }

    json_response(StatusCode::OK, json!({ "shutdown": true }))
}
