// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, json_response, log_event, now_iso, resolve_source_identity, truncate_chars};
use crate::api_types::{RetentionClass, StoreRequest};
use crate::budgets::BudgetEndpoint;
use crate::conflict::{detect_conflict, jaccard_similarity, ConflictClassification, ConflictResult};
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;


use super::*;
pub fn persist_decision_embedding(
    conn: &Connection,
    decision_id: i64,
    vector: &[f32],
    model_key: &str,
) -> Result<(), String> {
    let blob = crate::embeddings::vector_to_blob(vector);
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
         VALUES ('decision', ?1, ?2, ?3)",
        params![decision_id, blob, model_key],
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to persist decision embedding: {e}"))
}

