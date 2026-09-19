use super::*;
use crate::api_types::RetentionClass;
use crate::conflict::ConflictResult;
use crate::handlers::log_event;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

fn relation_surprise(relation: &ConflictResult) -> Option<f64> {
    Some((1.0 - relation.similarity_jaccard).clamp(0.0, 1.0))
}

fn require_supersede(
    tx: &Connection,
    ts: &str,
    target_id: i64,
    owner_id: Option<i64>,
) -> Result<(), StoreError> {
    let updated = tx.execute(&format!("UPDATE decisions SET status = 'superseded', valid_until = ?1, updated_at = ?1 WHERE id = ?2{}", sql_owner_and(owner_id)), params![ts, target_id]).map_err(|e| StoreError::Internal(e.to_string()))?;
    require_row_updated(updated, target_id)
}

pub fn handle_contradiction_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    retention_class: RetentionClass,
    expires_at: Option<&str>,
    ts: &str,
    owner_id: Option<i64>,
    relation: &ConflictResult,
) -> Result<(Value, Option<i64>), StoreError> {
    let existing_id = relation
        .matched_id
        .ok_or_else(|| StoreError::Internal("Missing conflict target id".to_string()))?;
    let existing_trust = relation.matched_trust_score.unwrap_or(0.8);
    let incoming_wins = trust_score > existing_trust;
    let (strategy, status, disputes, supersedes) = if incoming_wins {
        ("trust_score_source_wins", "active", None, Some(existing_id))
    } else {
        (
            "trust_score_target_wins",
            "disputed",
            Some(existing_id),
            None,
        )
    };
    with_store_savepoint(conn, |tx| {
        if incoming_wins {
            require_supersede(tx, ts, existing_id, owner_id)?;
        }
        let new_id = insert_decision_with_state(
            tx,
            decision,
            context,
            entry_type,
            source_agent,
            provenance,
            confidence,
            trust_score,
            quality,
            retention_class,
            expires_at,
            ts,
            owner_id,
            status,
            disputes,
            supersedes,
            relation_surprise(relation),
        )?;
        if !incoming_wins {
            let stamped = tx
                .execute(
                    "UPDATE decisions SET valid_until = ?1 WHERE id = ?2",
                    params![ts, new_id],
                )
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            require_row_updated(stamped, new_id)?;
        }
        let conflict_record_id = insert_conflict_record(
            tx,
            Some(new_id),
            existing_id,
            relation.classification,
            relation.similarity_jaccard,
            relation.similarity_cosine,
            "auto_resolved",
            Some(strategy),
            Some("policy_engine"),
            ts,
        )?;
        let _ = log_event(
            tx,
            "decision_conflict",
            json!({"newId":new_id,"existingId":existing_id,"source_agent":source_agent,"matchedAgent":relation.matched_agent,"strategy":strategy,"source_trust_score":trust_score,"target_trust_score":existing_trust,"conflict_record_id":conflict_record_id,}),
            "rust-daemon",
        );
        let mut entry = json!({"action":"inserted","id":new_id,"status":status,"retention_class":retention_class.as_str(),"quality":quality,"conflictWith":existing_id,"resolution_strategy":strategy,"observedAt":ts,"validFrom":ts,});
        if let Some(id) = supersedes {
            entry["supersedes"] = json!(id);
        } else {
            entry["validUntil"] = json!(ts);
        }
        decorate_entry_with_relation(
            &mut entry,
            relation,
            Some(conflict_record_json(
                conflict_record_id,
                Some(new_id),
                existing_id,
                relation.classification,
                "auto_resolved",
                Some(strategy),
            )),
        );
        Ok((entry, Some(new_id)))
    })
}
#[allow(clippy::too_many_arguments)]
pub fn handle_agreement_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    source_agent: &str,
    quality: i32,
    ts: &str,
    owner_id: Option<i64>,
    relation: &ConflictResult,
) -> Result<(Value, Option<i64>), StoreError> {
    let target_id = relation
        .matched_id
        .ok_or_else(|| StoreError::Internal("Missing agreement target id".to_string()))?;
    with_store_savepoint(conn, |tx| {
        let (existing_decision, existing_context, previous_merged_count) =
            load_merge_target(tx, target_id, owner_id)?;
        let merged_context = merge_context(existing_context, &existing_decision, context, decision);
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
        let conflict_record_id = insert_conflict_record(
            tx,
            None,
            target_id,
            relation.classification,
            relation.similarity_jaccard,
            relation.similarity_cosine,
            "auto_resolved",
            Some("deduplicated_merge"),
            Some("policy_engine"),
            ts,
        )?;
        let _ = log_event(
            tx,
            "decision_agreement_merge",
            json!({"targetId":target_id,"source_agent":source_agent,"similarity_jaccard":relation.similarity_jaccard,"conflict_record_id":conflict_record_id,}),
            "rust-daemon",
        );
        let mut entry = json!({"action":"merged","target_id":target_id,"merged_count":merged_count,"quality":quality,});
        decorate_entry_with_relation(
            &mut entry,
            relation,
            Some(conflict_record_json(
                conflict_record_id,
                None,
                target_id,
                relation.classification,
                "auto_resolved",
                Some("deduplicated_merge"),
            )),
        );
        // The existing row is the store target: deposit must project clock,
        // graph, and a successor revision onto it. Returning None dropped
        // those side effects even though the merge wrote context.
        Ok((entry, Some(target_id)))
    })
}
#[allow(clippy::too_many_arguments)]
pub fn handle_refinement_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    retention_class: RetentionClass,
    expires_at: Option<&str>,
    ts: &str,
    owner_id: Option<i64>,
    relation: &ConflictResult,
) -> Result<(Value, Option<i64>), StoreError> {
    let target_id = relation
        .matched_id
        .ok_or_else(|| StoreError::Internal("Missing refinement target id".to_string()))?;
    let target_trust = relation.matched_trust_score.unwrap_or(0.8);
    // Same agent may refine in place. A different agent supersedes only with
    // strictly higher trust -- equal default 0.8 must not silently replace
    // another writer's related (not agreeing) decision. Contradiction
    // policy uses the same `>` rule. Identity ignores a trailing
    // ` (model)` suffix: hook rows are `claude-code (opus)` while MCP
    // often sends `claude-code`.
    let should_supersede = relation
        .matched_agent
        .as_deref()
        .is_some_and(|matched| same_agent(matched, source_agent))
        || trust_score > target_trust;
    with_store_savepoint(conn, |tx| {
        let (
            status,
            disputes,
            supersedes,
            conflict_status,
            strategy,
            resolved_by,
            event_name,
            entry_status,
            entry_link,
        ) = if should_supersede {
            require_supersede(tx, ts, target_id, owner_id)?;
            (
                "active",
                None,
                Some(target_id),
                "auto_resolved",
                Some("refine_supersede"),
                Some("policy_engine"),
                "decision_supersede",
                "superseded_old",
                "supersedes",
            )
        } else {
            (
                "disputed",
                Some(target_id),
                None,
                "open",
                Some("requires_user_review"),
                None,
                "decision_refine_pending",
                "disputed",
                "conflictWith",
            )
        };
        let new_id = insert_decision_with_state(
            tx,
            decision,
            context,
            entry_type,
            source_agent,
            provenance,
            confidence,
            trust_score,
            quality,
            retention_class,
            expires_at,
            ts,
            owner_id,
            status,
            disputes,
            supersedes,
            relation_surprise(relation),
        )?;
        let conflict_record_id = insert_conflict_record(
            tx,
            Some(new_id),
            target_id,
            relation.classification,
            relation.similarity_jaccard,
            relation.similarity_cosine,
            conflict_status,
            strategy,
            resolved_by,
            ts,
        )?;
        let _ = log_event(
            tx,
            event_name,
            json!({"newId":new_id,"targetId":target_id,"source_agent":source_agent,"strategy":strategy,"conflict_record_id":conflict_record_id,}),
            "rust-daemon",
        );
        let mut entry = json!({"action":"inserted","id":new_id,"status":entry_status,"retention_class":retention_class.as_str(),"quality":quality,"observedAt":ts,"validFrom":ts,});
        entry[entry_link] = json!(target_id);
        decorate_entry_with_relation(
            &mut entry,
            relation,
            Some(conflict_record_json(
                conflict_record_id,
                Some(new_id),
                target_id,
                relation.classification,
                conflict_status,
                strategy,
            )),
        );
        Ok((entry, Some(new_id)))
    })
}
