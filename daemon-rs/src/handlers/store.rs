// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ensure_auth_with_caller, json_response, log_event, now_iso};
use crate::conflict::detect_conflict;
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct StoreRequest {
    pub decision: Option<String>,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub source_agent: Option<String>,
    pub confidence: Option<f64>,
}

// ─── POST /store ─────────────────────────────────────────────────────────────

pub async fn handle_store(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<StoreRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    let decision = body.decision.unwrap_or_default();
    if decision.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing field: decision" }),
        );
    }

    let source_agent = headers
        .get("x-source-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or(body.source_agent)
        .unwrap_or_else(|| "http".to_string());

    // Try cosine conflict detection first (if embeddings available).
    let cosine_conflict = if let Some(engine) = &state.embedding_engine {
        let conn = state.db.lock().await;
        crate::conflict::detect_conflict_cosine(decision.trim(), &source_agent, engine, &conn)
    } else {
        None
    };

    let mut conn = state.db.lock().await;
    let result = store_decision(
        &mut conn,
        decision.trim(),
        body.context,
        body.entry_type,
        source_agent.clone(),
        body.confidence,
        cosine_conflict,
        caller_id,
    );

    match result {
        Ok((entry, new_id)) => {
            // Fire-and-forget: generate embedding for the new decision.
            if let (Some(id), Some(engine)) = (new_id, state.embedding_engine.clone()) {
                let db = state.db.clone();
                let text = decision.trim().to_string();
                tokio::spawn(async move {
                    if let Some(vec) = engine.embed(&text) {
                        let blob = crate::embeddings::vector_to_blob(&vec);
                        let conn = db.lock().await;
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                             VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
                            rusqlite::params![id, blob],
                        );
                    }
                });
            }
            // Track store activity in open focus sessions for HTTP path too.
            crate::focus::focus_append(&conn, &source_agent, decision.trim());
            json_response(StatusCode::OK, json!({ "stored": true, "entry": entry }))
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Store failed: {err}") }),
        ),
    }
}

// ─── Core store logic ────────────────────────────────────────────────────────

/// Insert a new decision with Jaccard conflict detection and surprise scoring.
///
/// Logic mirrors the Node.js brain.js store() function:
///   1. Detect conflict via Jaccard similarity (last 50 active decisions).
///   2. Same-agent + sim > 0.7  => mark old as 'superseded', insert new with
///      supersedes_id pointing to old.
///   3. Different-agent + sim > 0.7  => insert new as 'disputed' with
///      disputes_id, then mark existing entry as 'disputed' too.
///   4. No conflict: compute surprise = 1 - max_sim; reject if surprise < 0.25
///      (duplicate suppression). Otherwise insert as 'active'.
///
/// Returns `(json_entry, Option<new_id>)`.
#[allow(clippy::too_many_arguments)]
pub fn store_decision(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    confidence: Option<f64>,
    cosine_conflict: Option<crate::conflict::ConflictResult>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), String> {
    let entry_type = entry_type.unwrap_or_else(|| "decision".to_string());
    let confidence = confidence.unwrap_or(0.8);
    let ts = now_iso();

    // ── 1. Conflict detection (cosine first, then Jaccard fallback) ──────────
    let cr = match cosine_conflict {
        Some(c) => c,
        None => detect_conflict(conn, decision, &source_agent)?,
    };

    if cr.is_conflict {
        // Different-agent conflict: insert new entry as 'disputed', then mark
        // the existing entry as 'disputed' too (they reference each other).
        let existing_id = cr.matched_id.unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        if let Some(oid) = owner_id {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, status, disputes_id, owner_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'disputed', ?6, ?7, ?8, ?8)",
                params![decision, context, entry_type, source_agent.clone(), confidence, existing_id, oid, ts],
            )
        } else {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, status, disputes_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'disputed', ?6, ?7, ?7)",
                params![decision, context, entry_type, source_agent.clone(), confidence, existing_id, ts],
            )
        }
        .map_err(|e| e.to_string())?;
        let new_id = tx.last_insert_rowid();

        // Mark existing entry as disputed, pointing back to the new one.
        tx.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?, updated_at = ? WHERE id = ?",
            params![new_id, ts, existing_id],
        )
        .map_err(|e| e.to_string())?;

        let _ = log_event(
            &tx,
            "decision_conflict",
            json!({
                "newId": new_id,
                "existingId": existing_id,
                "source_agent": source_agent,
                "matchedAgent": cr.matched_agent,
            }),
            "rust-daemon",
        );
        tx.commit().map_err(|e| e.to_string())?;
        checkpoint_wal_best_effort(conn);

        return Ok((
            json!({
                "stored": true,
                "id": new_id,
                "status": "disputed",
                "conflictWith": existing_id,
            }),
            Some(new_id),
        ));
    }

    if cr.is_update {
        // Same-agent update: supersede the old entry and insert the new one.
        let old_id = cr.matched_id.unwrap();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE decisions SET status = 'superseded', updated_at = ? WHERE id = ?",
            params![ts, old_id],
        )
        .map_err(|e| e.to_string())?;

        if let Some(oid) = owner_id {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, supersedes_id, owner_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![decision, context, entry_type, source_agent.clone(), confidence, old_id, oid, ts],
            )
        } else {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, supersedes_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![decision, context, entry_type, source_agent.clone(), confidence, old_id, ts],
            )
        }
        .map_err(|e| e.to_string())?;
        let new_id = tx.last_insert_rowid();

        let _ = log_event(
            &tx,
            "decision_supersede",
            json!({
                "newId": new_id,
                "supersededId": old_id,
                "source_agent": source_agent,
            }),
            "rust-daemon",
        );
        tx.commit().map_err(|e| e.to_string())?;
        checkpoint_wal_best_effort(conn);

        return Ok((
            json!({
                "stored": true,
                "id": new_id,
                "status": "superseded_old",
                "supersedes": old_id,
            }),
            Some(new_id),
        ));
    }

    // ── 2. Duplicate suppression via Jaccard surprise ────────────────────────
    // Recompute max similarity against active decisions for the surprise score.
    let existing: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT decision FROM decisions \
                 WHERE status = 'active' \
                 ORDER BY created_at DESC LIMIT 50",
            )
            .map_err(|e| e.to_string())?;
        let result: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        result
    };

    let max_sim: f64 = existing
        .iter()
        .map(|t| crate::conflict::jaccard_similarity(decision, t))
        .fold(0.0_f64, f64::max);

    let surprise = 1.0 - max_sim;
    if surprise < 0.25 {
        let _ = log_event(
            conn,
            "decision_rejected_duplicate",
            json!({
                "decision": &decision[..decision.len().min(100)],
                "surprise": surprise,
                "source_agent": source_agent,
            }),
            "rust-daemon",
        );
        checkpoint_wal_best_effort(conn);
        return Ok((
            json!({ "stored": false, "reason": "duplicate", "surprise": surprise }),
            None,
        ));
    }

    // ── 3. Normal insert ─────────────────────────────────────────────────────
    let surprise_rounded = (surprise * 10_000.0).round() / 10_000.0;
    if let Some(oid) = owner_id {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, owner_id, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?8)",
            params![decision, context, entry_type, source_agent.clone(), confidence, surprise_rounded, oid, ts],
        )
    } else {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7)",
            params![decision, context, entry_type, source_agent.clone(), confidence, surprise_rounded, ts],
        )
    }
    .map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();
    let _ = log_event(
        conn,
        "decision_stored",
        json!({ "id": id, "source_agent": source_agent, "surprise": surprise_rounded }),
        "rust-daemon",
    );
    checkpoint_wal_best_effort(conn);

    Ok((
        json!({ "stored": true, "id": id, "status": "active", "surprise": surprise_rounded }),
        Some(id),
    ))
}

