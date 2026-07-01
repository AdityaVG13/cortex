// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Instant;
use crate::handlers::diary::{write_diary_entry, DiaryRequest};
use crate::handlers::feedback::{build_agent_feedback_stats_payload, recommend_recall_k, record_agent_feedback_from_value};
use crate::handlers::health::{build_digest, build_health_payload};
use crate::handlers::mutate::{forget_keyword_scoped, list_conflicts_payload, parse_conflict_id, resolve_decision, resolve_decision_with_metadata, ConflictListOptions, ConflictStatusFilter, ResolutionMetadata};
use crate::handlers::recall::{execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall, parse_recall_policy_mode, resolve_recall_budget_k, unfold_source, RecallContext};
use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding_and_provenance_retention, validate_explicit_ttl_seconds, DecisionProvenance};
use crate::handlers::{estimate_tokens, now_iso, SourceIdentity};
use crate::api_types::RetentionClass;
use crate::state::RuntimeState;
use crate::{aging, db, indexer};

use super::*;
use super::{mcp_session_description, mcp_session_owner_id, normalize_mcp_agent_label};
pub(crate) async fn upsert_mcp_session(
    state: &RuntimeState,
    caller_id: Option<i64>,
    raw_agent: &str,
    model: Option<&str>,
    description_prefix: &str,
) -> Result<(String, String), String> {
    let agent = normalize_mcp_agent_label(raw_agent, model)?;
    let owner_id = mcp_session_owner_id(state, caller_id)?;
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let description = mcp_session_description(description_prefix, model);

    let conn = state.db.lock().await;
    if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, 'mcp', '[]', ?4, ?5, ?5, ?6)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               description = CASE
                   WHEN sessions.description IS NULL OR trim(sessions.description) = '' THEN excluded.description
                   ELSE sessions.description
               END,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![agent, owner_id, session_id, description, now, expires_at],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, 'mcp', '[]', ?3, ?4, ?4, ?5)
             ON CONFLICT(agent) DO UPDATE SET
               description = CASE
                   WHEN sessions.description IS NULL OR trim(sessions.description) = '' THEN excluded.description
                   ELSE sessions.description
               END,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![agent, session_id, description, now, expires_at],
        )
        .map_err(|e| e.to_string())?;
    }

    crate::db::checkpoint_wal_best_effort(&conn);
    Ok((agent, expires_at))
}
