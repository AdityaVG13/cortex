use super::*;
use crate::api_types::RetentionClass;
use crate::conflict::{
    ConflictClassification, fetch_recent_decision_candidates, is_typed_evidence_kind,
    jaccard_token_set, scan_recent_decision_candidates,
};
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{log_event, now_iso, truncate_chars};
use rusqlite::Connection;
use serde_json::{Value, json};
#[allow(clippy::too_many_arguments, dead_code)]
pub fn store_decision_with_ttl(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    confidence: Option<f64>,
    ttl_seconds: Option<i64>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), String> {
    // Redact BEFORE persist and BEFORE graph/clock projection. The handler path
    // (handlers/store/handler.rs) redacts upstream, but direct lib callers pass
    // raw text; projecting the raw parameter here leaked secrets into
    // entities/entity_aliases/clock_anchors even though the stored row is
    // redacted inside store_decision_internal.
    let decision = crate::handlers::redact_secrets(decision.trim());
    let provenance = DecisionProvenance::from_fields(&source_agent, None, None);
    let (entry, id) = store_decision_internal(
        conn,
        &decision,
        context.clone(),
        entry_type,
        source_agent,
        provenance,
        confidence,
        ttl_seconds,
        None,
        None,
        owner_id,
        &[],
    )
    .map_err(|err| err.to_string())?;
    let target_id = id
        .or_else(|| entry.get("id").and_then(|v| v.as_i64()))
        .or_else(|| entry.get("target_id").and_then(|v| v.as_i64()));
    if let Some(target_id) = target_id {
        crate::graph::ingest_for_target(
            conn,
            &decision,
            "decision",
            Some(target_id),
            None,
            owner_id,
        );
        let extra = Vec::new();
        crate::clockwork::project_target(
            conn,
            &decision,
            &extra,
            "decision",
            target_id,
            crate::clockwork::ClockOrigin::DeterministicExtract,
            None,
        )
        .map_err(|err| format!("clock projection failed for {target_id}: {err}"))?;
    }
    Ok((entry, id))
}
#[allow(clippy::too_many_arguments, dead_code)]
pub fn store_decision_with_input_embedding(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    confidence: Option<f64>,
    ttl_seconds: Option<i64>,
    query_embedding: Option<&[f32]>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    let provenance = DecisionProvenance::from_fields(&source_agent, None, None);
    store_decision_with_input_embedding_and_provenance(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        ttl_seconds,
        query_embedding,
        owner_id,
    )
}
#[allow(clippy::too_many_arguments)]
pub fn store_decision_with_input_embedding_and_provenance(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    provenance: DecisionProvenance,
    confidence: Option<f64>,
    ttl_seconds: Option<i64>,
    query_embedding: Option<&[f32]>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    store_decision_with_input_embedding_and_provenance_retention(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        ttl_seconds,
        None,
        query_embedding,
        owner_id,
        &[],
    )
}
#[allow(clippy::too_many_arguments)]
pub fn store_decision_with_input_embedding_and_provenance_retention(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    provenance: DecisionProvenance,
    confidence: Option<f64>,
    ttl_seconds: Option<i64>,
    retention_class: Option<RetentionClass>,
    query_embedding: Option<&[f32]>,
    owner_id: Option<i64>,
    paths: &[String],
) -> Result<(Value, Option<i64>), StoreError> {
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        ttl_seconds,
        retention_class,
        query_embedding,
        owner_id,
        paths,
    )
}
mod persist;
use persist::store_decision_internal;
