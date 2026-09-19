use super::*;
use crate::api_types::RetentionClass;
use crate::handlers::log_event;
use crate::protocol::nonempty_opt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub fn load_merge_target(
    tx: &Connection,
    target_id: i64,
    owner_id: Option<i64>,
) -> Result<(String, Option<String>, i64), StoreError> {
    tx.query_row(
        &format!(
            "SELECT decision, context, COALESCE(merged_count, 0) FROM decisions WHERE id = ?1{}",
            sql_owner_and(owner_id)
        ),
        params![target_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .map_err(|e| StoreError::Internal(e.to_string()))
}

pub fn apply_merge_update(
    tx: &Connection,
    target_id: i64,
    owner_id: Option<i64>,
    merged_context: Option<&str>,
    merged_count: i64,
    quality: i32,
    ts: &str,
) -> Result<(), StoreError> {
    let updated = tx.execute(&format!("UPDATE decisions SET context = ?1, score = COALESCE(score, 0) + ?2, merged_count = ?3, quality = MAX(COALESCE(quality, 50), ?4), updated_at = ?5 WHERE id = ?6{}", sql_owner_and(owner_id)), params![merged_context, MERGE_SCORE_BONUS, merged_count, quality, ts, target_id]).map_err(|e| StoreError::Internal(e.to_string()))?;
    require_row_updated(updated, target_id)
}

pub fn require_row_updated(updated: usize, target_id: i64) -> Result<(), StoreError> {
    if updated == 0 {
        return Err(StoreError::Internal(format!(
            "decision `{target_id}` was not updated"
        )));
    }
    Ok(())
}

pub fn merge_into_existing_decision(
    conn: &mut Connection,
    target_id: i64,
    incoming_text: &str,
    incoming_context: Option<&str>,
    source_agent: &str,
    quality: i32,
    similarity: f32,
    jaccard: f64,
    ts: &str,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    with_store_savepoint(conn, |tx| {
        let (existing_decision, existing_context, previous_merged_count) =
            load_merge_target(tx, target_id, owner_id)?;
        let merged_context = merge_context(
            existing_context,
            &existing_decision,
            incoming_context,
            incoming_text,
        );
        let merged_count = previous_merged_count + 1;
        apply_merge_update(
            tx,
            target_id,
            owner_id,
            merged_context.as_deref(),
            merged_count,
            quality,
            ts,
        )?;
        let _ = log_event(
            tx,
            "merge",
            json!({"source_id":Value::Null,"target_id":target_id,"target_type":"decision","incoming_text":incoming_text,"similarity":similarity,"jaccard":jaccard,"source_agent":source_agent,}),
            "rust-daemon",
        );
        Ok((
            json!({"action":"merged","target_id":target_id,"merged_count":merged_count,"quality":quality,"similarity":similarity,"jaccard":jaccard,}),
            Some(target_id),
        ))
    })
}
pub fn merge_context(
    existing_context: Option<String>,
    existing_decision: &str,
    incoming_context: Option<&str>,
    incoming_text: &str,
) -> Option<String> {
    let incoming_note = nonempty_opt(incoming_context)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| incoming_text.trim().to_string());
    if incoming_note.is_empty() || incoming_note.eq_ignore_ascii_case(existing_decision.trim()) {
        return existing_context;
    }
    match existing_context {
        Some(existing) if !existing.trim().is_empty() => {
            let already_present = existing
                .split("\n\n")
                .any(|part| part.trim().eq_ignore_ascii_case(&incoming_note));
            if already_present {
                Some(existing)
            } else {
                Some(format!("{existing}\n\n{incoming_note}"))
            }
        }
        _ => Some(incoming_note),
    }
}
#[allow(clippy::too_many_arguments)]
pub fn insert_decision(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    retention_class: RetentionClass,
    expires_at: Option<String>,
    ts: &str,
    owner_id: Option<i64>,
    surprise: f64,
    emit_decision_stored_event: bool,
) -> Result<(Value, Option<i64>), StoreError> {
    let surprise = (surprise * 10_000.0).round() / 10_000.0;
    // Savepoint (not BEGIN) so the store nests inside the runtime's atomic
    // deposit batch; outside any transaction a savepoint starts one.
    with_store_savepoint(conn, |tx| {
        if let Some(oid) = owner_id {
            let mut stmt = tx.prepare_cached("INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise, status, owner_id, quality, retention_class, expires_at, observed_at, valid_from, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?11, ?11, ?11, ?11, ?12, ?13, ?14, ?15)").map_err(|e| StoreError::Internal(e.to_string()))?;
            stmt.execute(params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                oid,
                quality,
                retention_class.as_str(),
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score
            ])
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        } else {
            let mut stmt = tx.prepare_cached("INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise, status, quality, retention_class, expires_at, observed_at, valid_from, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?10, ?10, ?10, ?11, ?12, ?13, ?14)").map_err(|e| StoreError::Internal(e.to_string()))?;
            stmt.execute(params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                quality,
                retention_class.as_str(),
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score
            ])
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        }
        let id = tx.last_insert_rowid();
        if emit_decision_stored_event {
            let _ = log_event(
                tx,
                "decision_stored",
                json!({"id":id,"source_agent":source_agent,"surprise":surprise,"quality":quality,}),
                "rust-daemon",
            );
        }
        Ok((
            json!({"action":"inserted","id":id,"status":"active","retention_class":retention_class.as_str(),"surprise":surprise,"quality":quality,"observedAt":ts,"validFrom":ts}),
            Some(id),
        ))
    })
}
pub fn compute_expires_at(
    conn: &Connection,
    ttl_seconds: Option<i64>,
) -> Result<Option<String>, String> {
    let Some(ttl_seconds) = ttl_seconds else {
        return Ok(None);
    };
    let modifier = format!("+{ttl_seconds} seconds");
    conn.prepare_cached("SELECT datetime('now', ?1)")
        .and_then(|mut stmt| stmt.query_row(params![modifier], |row| row.get(0)))
        .map(Some)
        .map_err(|e| format!("Failed to compute expires_at: {e}"))
}
