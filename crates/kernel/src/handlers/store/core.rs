use super::*;
use crate::api_types::RetentionClass;
use crate::conflict::{
    fetch_recent_decision_candidates, jaccard_token_set, scan_recent_decision_candidates,
    ConflictClassification,
};
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{log_event, now_iso, truncate_chars};
use rusqlite::Connection;
use serde_json::{json, Value};
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
#[allow(clippy::too_many_arguments)]
pub fn store_decision_internal(
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
    let entry_type = entry_type.unwrap_or_else(|| "decision".to_string());
    let suppress_benchmark_events =
        is_benchmark_entry_type(&entry_type) || is_benchmark_source_agent(&source_agent);
    let mut decision_text = decision.trim().to_string();
    decision_text = crate::handlers::redact_secrets(&decision_text);
    let context = context.map(|c| {
        let redacted = crate::handlers::redact_secrets(&c);
        if !is_benchmark_entry_type(&entry_type) && redacted.chars().count() > MAX_CONTEXT_CHARS {
            truncate_chars(&redacted, MAX_CONTEXT_CHARS)
        } else {
            redacted
        }
    });
    let decision_chars = if decision_text.is_ascii() {
        decision_text.len()
    } else {
        decision_text.chars().count()
    };
    let decision_truncated =
        !is_benchmark_entry_type(&entry_type) && decision_chars > MAX_DECISION_CHARS;
    if decision_truncated {
        decision_text = truncate_chars(&decision_text, MAX_DECISION_CHARS);
    }
    let decision = decision_text.as_str();
    let quality = assess_quality(decision);
    let confidence = confidence.unwrap_or(0.8);
    let trust_score = provenance.trust_score(confidence);
    let ts = now_iso();
    let retention_class =
        RetentionClass::classify(retention_class, &entry_type, decision, context.as_deref());
    let ttl_seconds = validate_explicit_ttl_seconds(ttl_seconds)?;
    let effective_ttl_seconds = ttl_seconds.or_else(|| retention_class.default_ttl_seconds());
    let expires_at =
        compute_expires_at(conn, effective_ttl_seconds).map_err(StoreError::Internal)?;
    if decision_truncated {
        let _ = log_event(
            conn,
            "decision_truncated",
            json!({
"source_agent":source_agent,"entry_type":entry_type.as_str(),"original_chars":decision_chars,"stored_chars":MAX_DECISION_CHARS,
"preview":truncate_chars(decision,180),}),
            "rust-daemon",
        );
    }
    if is_benchmark_entry_type(&entry_type) {
        return insert_decision(
            conn,
            decision,
            context,
            &entry_type,
            &source_agent,
            &provenance,
            confidence,
            trust_score,
            quality.score,
            retention_class,
            expires_at,
            &ts,
            owner_id,
            1.0,
            !suppress_benchmark_events,
        );
    }
    if quality.score < TOO_VAGUE_THRESHOLD {
        return Err(StoreError::Validation {
            message: "Memory too vague",
            quality: quality.score,
            factors: quality.factors,
        });
    }
    let _ = query_embedding;
    store_decision_legacy(
        conn,
        decision,
        context,
        &entry_type,
        &source_agent,
        &provenance,
        confidence,
        trust_score,
        quality.score,
        retention_class,
        expires_at,
        &ts,
        owner_id,
        paths,
    )
}
pub fn is_typed_evidence_kind(entry_type: &str) -> bool {
    crate::conflict::is_typed_evidence_kind(entry_type)
}

#[allow(clippy::too_many_arguments)]
pub fn store_decision_legacy(
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
    paths: &[String],
) -> Result<(Value, Option<i64>), StoreError> {
    let decision_tokens = jaccard_token_set(decision);
    let mut recent_candidates =
        fetch_recent_decision_candidates(conn, owner_id).map_err(StoreError::Internal)?;
    let candidate_ids: Vec<i64> = recent_candidates.iter().map(|candidate| candidate.id).collect();
    match crate::handlers::recall::explicit_paths_by_target(conn, "decision", &candidate_ids) {
        Ok(path_map) => {
            let incoming = crate::handlers::recall::normalize_query_paths(paths);
            recent_candidates.retain(|candidate| {
                let candidate_paths = path_map
                    .get(&candidate.id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                crate::handlers::recall::jaccard_path_sets(&incoming, candidate_paths)
            });
        }
        Err(err) => {
            return Err(StoreError::Internal(err));
        }
    }
    let mut recent_scan = scan_recent_decision_candidates(
        &recent_candidates,
        decision,
        source_agent,
        &decision_tokens,
    );
    // Typed evidence records are separate observations: a case, a
    // counterexample, an attempt or an exception is never merged into or
    // superseded by a lexically similar row. Constraint-like kinds
    // (policy/rule/convention/contract) are the same family as constraint
    // and preference: Jaccard must not collapse two distinct rules.
    // The recent-window query already omits those kinds as *targets*, so a
    // later plain decision cannot merge into or retire a policy. Incoming
    // typed rows still skip the policy engine here. Only plain
    // decisions/notes take part in agreement/refinement/contradiction
    // *or* the surprise duplicate gate below.
    if is_typed_evidence_kind(entry_type) {
        recent_scan.relation.classification = ConflictClassification::Unrelated;
    }
    let relation = recent_scan.relation;
    match relation.classification {
        ConflictClassification::Contradicts => {
            return handle_contradiction_policy(
                conn,
                decision,
                context.as_deref(),
                entry_type,
                source_agent,
                provenance,
                confidence,
                trust_score,
                quality,
                retention_class,
                expires_at.as_deref(),
                ts,
                owner_id,
                &relation,
            );
        }
        ConflictClassification::Agrees => {
            return handle_agreement_policy(
                conn,
                decision,
                context.as_deref(),
                source_agent,
                quality,
                ts,
                owner_id,
                &relation,
            );
        }
        ConflictClassification::Refines => {
            return handle_refinement_policy(
                conn,
                decision,
                context.as_deref(),
                entry_type,
                source_agent,
                provenance,
                confidence,
                trust_score,
                quality,
                retention_class,
                expires_at.as_deref(),
                ts,
                owner_id,
                &relation,
            );
        }
        ConflictClassification::Unrelated => {}
    }
    let surprise = 1.0 - recent_scan.max_jaccard;
    // Typed kinds already skipped merge/refine. Dropping them here as
    // "duplicate" still collapsed a second attempt, exception, or policy
    // whose text overlapped the first (surprise is 1 - Jaccard).
    if !is_typed_evidence_kind(entry_type) && surprise < 0.25 {
        let _ = log_event(
            conn,
            "decision_rejected_duplicate",
            json!({
"decision":truncate_chars(decision, 100),"surprise":surprise,"source_agent":source_agent,"quality":quality,}),
            "rust-daemon",
        );
        checkpoint_wal_best_effort(conn);
        let mut entry = json!({"stored":false,"reason":"duplicate","surprise":surprise,"quality":quality
,});
        decorate_entry_with_relation(&mut entry, &relation, None);
        return Ok((entry, None));
    }
    let (mut entry, new_id) = insert_decision(
        conn,
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
        surprise,
        !(is_benchmark_entry_type(entry_type) || is_benchmark_source_agent(source_agent)),
    )?;
    decorate_entry_with_relation(&mut entry, &relation, None);
    Ok((entry, new_id))
}
