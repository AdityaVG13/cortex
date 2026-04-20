// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::{
    ensure_auth_with_caller_rated, json_response, log_event, now_iso, resolve_source_identity,
    truncate_chars,
};
use crate::api_types::StoreRequest;
use crate::conflict::{
    detect_conflict, jaccard_similarity, ConflictClassification, ConflictResult,
};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

const HARD_MERGE_THRESHOLD: f32 = 0.92;
const REVIEW_MERGE_THRESHOLD: f32 = 0.90;
const JACCARD_MERGE_THRESHOLD: f64 = 0.70;
const MERGE_SCORE_BONUS: f64 = 5.0;
const TOO_VAGUE_THRESHOLD: i32 = 20;
const BENCHMARK_ENTRY_TYPE: &str = "benchmark";
const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex::";
const MAX_DECISION_CHARS: usize = 4096;

fn is_benchmark_entry_type(entry_type: &str) -> bool {
    entry_type.eq_ignore_ascii_case(BENCHMARK_ENTRY_TYPE)
}

fn is_benchmark_source_agent(source_agent: &str) -> bool {
    source_agent
        .trim()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_AGENT_PREFIX)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecisionProvenance {
    source_client: String,
    source_model: Option<String>,
    reasoning_depth: String,
}

impl DecisionProvenance {
    pub(crate) fn from_fields(
        source_agent: &str,
        source_model: Option<&str>,
        reasoning_depth: Option<&str>,
    ) -> Self {
        let normalized_model = source_model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Self {
            source_client: normalize_source_client(source_agent),
            source_model: normalized_model,
            reasoning_depth: normalize_reasoning_depth(reasoning_depth),
        }
    }

    fn trust_score(&self, confidence: f64) -> f64 {
        compute_trust_score(confidence, self.source_model.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityFactors {
    length_score: i32,
    specificity_bonus: i32,
    question_penalty: i32,
}

impl QualityFactors {
    fn as_json(&self) -> Value {
        json!({
            "length_score": self.length_score,
            "specificity_bonus": self.specificity_bonus,
            "question_penalty": self.question_penalty,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QualityAssessment {
    score: i32,
    factors: QualityFactors,
}

#[derive(Debug, Clone)]
struct SemanticCandidate {
    id: i64,
    decision: String,
    similarity: f32,
}

#[derive(Debug, Clone, PartialEq)]
enum SemanticDedupAction {
    Insert,
    Merge {
        target_id: i64,
        similarity: f32,
        jaccard: f64,
    },
}

#[derive(Debug)]
pub(crate) enum StoreError {
    Validation {
        message: &'static str,
        quality: i32,
        factors: QualityFactors,
    },
    Internal(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Validation {
                message, quality, ..
            } => write!(f, "{message} (quality {quality})"),
            StoreError::Internal(message) => write!(f, "{message}"),
        }
    }
}

impl From<String> for StoreError {
    fn from(value: String) -> Self {
        StoreError::Internal(value)
    }
}

fn normalize_source_client(raw: &str) -> String {
    let before_model = raw
        .split('(')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    let normalized: String = before_model
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
    }
}

fn normalize_reasoning_depth(raw: Option<&str>) -> String {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .map(|value| {
            value
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' {
                        ch
                    } else if ch == ' ' || ch == '_' {
                        '-'
                    } else {
                        '\0'
                    }
                })
                .filter(|ch| *ch != '\0')
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "single-shot".to_string());

    match normalized.as_str() {
        "chain-of-thought" | "single-shot" | "tool-assisted" | "multi-step" | "user-stated" => {
            normalized
        }
        _ => "single-shot".to_string(),
    }
}

fn model_weight(source_model: Option<&str>) -> f64 {
    let Some(model) = source_model.map(|value| value.to_ascii_lowercase()) else {
        return 0.70;
    };
    if model.contains("opus") {
        1.0
    } else if model.contains("sonnet") {
        0.85
    } else if model.contains("gemini") && model.contains("pro") {
        0.80
    } else if model.contains("gemini") {
        0.60
    } else if model.contains("qwen") {
        0.50
    } else {
        0.70
    }
}

fn compute_trust_score(confidence: f64, source_model: Option<&str>) -> f64 {
    let bounded_confidence = confidence.clamp(0.0, 1.0);
    let raw = bounded_confidence * model_weight(source_model);
    ((raw * 10_000.0).round() / 10_000.0).clamp(0.0, 1.0)
}

pub async fn handle_store(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<StoreRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
        );
    }

    let decision = body.decision.unwrap_or_default();
    if decision.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing field: decision" }),
        );
    }

    let source_identity =
        resolve_source_identity(&headers, body.source_agent.as_deref().unwrap_or("http"));
    let source_agent = source_identity.agent.clone();
    let benchmark_store = body
        .entry_type
        .as_deref()
        .map(is_benchmark_entry_type)
        .unwrap_or(false)
        || is_benchmark_source_agent(&source_agent);
    let provenance = DecisionProvenance::from_fields(
        &source_agent,
        body.source_model
            .as_deref()
            .or(source_identity.model.as_deref()),
        body.reasoning_depth.as_deref(),
    );

    if let Some(ttl_seconds) = body.ttl_seconds {
        if ttl_seconds <= 0 {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "ttl_seconds must be > 0" }),
            );
        }
    }

    let decision_text = decision.trim().to_string();
    let embedding_model_key = state
        .embedding_engine
        .as_ref()
        .map(|engine| engine.model_key())
        .unwrap_or(crate::embeddings::selected_model_key());
    let decision_embedding = state
        .embedding_engine
        .as_ref()
        .and_then(|engine| engine.embed(&decision_text));

    let mut conn = state.db.lock().await;
    let result = store_decision_with_input_embedding_and_provenance(
        &mut conn,
        &decision_text,
        body.context,
        body.entry_type,
        source_agent.clone(),
        provenance,
        body.confidence,
        body.ttl_seconds,
        decision_embedding.as_deref(),
        caller_id,
    );

    match result {
        Ok((entry, new_id)) => {
            if let Some(id) = new_id {
                if let Some(vec) = decision_embedding.as_deref() {
                    if let Err(err) =
                        persist_decision_embedding(&conn, id, vec, embedding_model_key)
                    {
                        eprintln!(
                            "[store] Warning: failed to persist decision embedding for {id}: {err}"
                        );
                    }
                } else if let Some(engine) = state.embedding_engine.clone() {
                    let db = state.db.clone();
                    let text = decision_text.clone();
                    tokio::spawn(async move {
                        if let Some(vec) = engine.embed(&text) {
                            let conn = db.lock().await;
                            let _ = persist_decision_embedding(&conn, id, &vec, engine.model_key());
                        }
                    });
                }
            }

            if !benchmark_store {
                crate::focus::focus_append(&conn, &source_agent, &decision_text);
            }
            json_response(StatusCode::OK, json!({ "stored": true, "entry": entry }))
        }
        Err(StoreError::Validation {
            message,
            quality,
            factors,
        }) => json_response(
            StatusCode::BAD_REQUEST,
            json!({
                "error": message,
                "quality": quality,
                "factors": factors.as_json(),
            }),
        ),
        Err(StoreError::Internal(err)) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Store failed: {err}") }),
        ),
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
pub fn store_decision(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    confidence: Option<f64>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), String> {
    let provenance = DecisionProvenance::from_fields(&source_agent, None, None);
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        None,
        None,
        owner_id,
    )
    .map_err(|err| err.to_string())
}

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
    let provenance = DecisionProvenance::from_fields(&source_agent, None, None);
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        ttl_seconds,
        None,
        owner_id,
    )
    .map_err(|err| err.to_string())
}

#[allow(clippy::too_many_arguments, dead_code)]
pub(crate) fn store_decision_with_input_embedding(
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
pub(crate) fn store_decision_with_input_embedding_and_provenance(
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
    store_decision_internal(
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
fn store_decision_internal(
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
    let entry_type = entry_type.unwrap_or_else(|| "decision".to_string());
    let suppress_benchmark_events =
        is_benchmark_entry_type(&entry_type) || is_benchmark_source_agent(&source_agent);
    let mut decision_text = decision.trim().to_string();
    let decision_chars = decision_text.chars().count();
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
    let expires_at = compute_expires_at(conn, ttl_seconds).map_err(StoreError::Internal)?;

    if decision_truncated {
        let _ = log_event(
            conn,
            "decision_truncated",
            json!({
                "source_agent": source_agent,
                "entry_type": entry_type.as_str(),
                "original_chars": decision_chars,
                "stored_chars": MAX_DECISION_CHARS,
                "preview": truncate_chars(decision, 180),
            }),
            "rust-daemon",
        );
    }

    // Benchmark ingestion must preserve corpus fidelity (no dedup/conflict collapse).
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

    if let Some(query_vector) = query_embedding {
        let candidates = fetch_top_semantic_candidates(conn, query_vector, owner_id)?;
        let dedup_action = choose_semantic_dedup_action(&candidates, decision);
        let best_similarity = candidates
            .first()
            .map(|candidate| candidate.similarity as f64)
            .unwrap_or(0.0);

        if let SemanticDedupAction::Merge {
            target_id,
            similarity,
            jaccard,
        } = dedup_action
        {
            return merge_into_existing_decision(
                conn,
                target_id,
                decision,
                context.as_deref(),
                &source_agent,
                quality.score,
                similarity,
                jaccard,
                &ts,
                owner_id,
            );
        }

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
            expires_at,
            &ts,
            owner_id,
            (1.0 - best_similarity).clamp(0.0, 1.0),
            !suppress_benchmark_events,
        );
    }

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
        expires_at,
        &ts,
        owner_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn store_decision_legacy(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    expires_at: Option<String>,
    ts: &str,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    let relation =
        detect_conflict(conn, decision, source_agent, owner_id).map_err(StoreError::Internal)?;

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
                expires_at.as_deref(),
                ts,
                owner_id,
                &relation,
            );
        }
        ConflictClassification::Unrelated => {}
    }

    let existing: Vec<String> = if let Some(owner_id) = owner_id {
        let mut stmt = conn
            .prepare(
                "SELECT decision FROM decisions \
                 WHERE owner_id = ?1 \
                 AND status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY created_at DESC LIMIT 50",
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        let rows = stmt
            .query_map(params![owner_id], |row| row.get(0))
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        rows.filter_map(|row| row.ok()).collect()
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT decision FROM decisions \
                 WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY created_at DESC LIMIT 50",
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        rows.filter_map(|row| row.ok()).collect()
    };

    let max_sim = existing
        .iter()
        .map(|text| jaccard_similarity(decision, text))
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
                "quality": quality,
            }),
            "rust-daemon",
        );
        checkpoint_wal_best_effort(conn);
        let mut entry = json!({
            "stored": false,
            "reason": "duplicate",
            "surprise": surprise,
            "quality": quality,
        });
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
        expires_at,
        ts,
        owner_id,
        surprise,
        !(is_benchmark_entry_type(entry_type) || is_benchmark_source_agent(source_agent)),
    )?;
    decorate_entry_with_relation(&mut entry, &relation, None);
    Ok((entry, new_id))
}

#[allow(clippy::too_many_arguments)]
fn handle_contradiction_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
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
    let strategy = if incoming_wins {
        "trust_score_source_wins"
    } else {
        "trust_score_target_wins"
    };

    let tx = conn
        .transaction()
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    if incoming_wins {
        if let Some(owner_id) = owner_id {
            tx.execute(
                "UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2 AND owner_id = ?3",
                params![ts, existing_id, owner_id],
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        } else {
            tx.execute(
                "UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2",
                params![ts, existing_id],
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        }
    }

    let new_id = insert_decision_with_state(
        &tx,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        trust_score,
        quality,
        expires_at,
        ts,
        owner_id,
        if incoming_wins { "active" } else { "disputed" },
        if incoming_wins {
            None
        } else {
            Some(existing_id)
        },
        if incoming_wins {
            Some(existing_id)
        } else {
            None
        },
        Some((1.0 - relation.similarity_jaccard).clamp(0.0, 1.0)),
    )?;

    let conflict_record_id = insert_conflict_record(
        &tx,
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
        &tx,
        "decision_conflict",
        json!({
            "newId": new_id,
            "existingId": existing_id,
            "source_agent": source_agent,
            "matchedAgent": relation.matched_agent,
            "strategy": strategy,
            "source_trust_score": trust_score,
            "target_trust_score": existing_trust,
            "conflict_record_id": conflict_record_id,
        }),
        "rust-daemon",
    );

    tx.commit()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    checkpoint_wal_best_effort(conn);

    let mut entry = json!({
        "action": "inserted",
        "id": new_id,
        "status": if incoming_wins { "active" } else { "disputed" },
        "quality": quality,
        "conflictWith": existing_id,
        "resolution_strategy": strategy,
    });
    if incoming_wins {
        entry["supersedes"] = json!(existing_id);
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
}

#[allow(clippy::too_many_arguments)]
fn handle_agreement_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    source_agent: &str,
    quality: i32,
    ts: &str,
    relation: &ConflictResult,
) -> Result<(Value, Option<i64>), StoreError> {
    let target_id = relation
        .matched_id
        .ok_or_else(|| StoreError::Internal("Missing agreement target id".to_string()))?;
    let tx = conn
        .transaction()
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    let (existing_decision, existing_context, previous_merged_count): (
        String,
        Option<String>,
        i64,
    ) = tx
        .query_row(
            "SELECT decision, context, COALESCE(merged_count, 0) FROM decisions WHERE id = ?1",
            params![target_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    let merged_context = merge_context(existing_context, &existing_decision, context, decision);
    let merged_count = previous_merged_count + 1;
    tx.execute(
        "UPDATE decisions \
         SET context = ?1, \
             score = COALESCE(score, 0) + ?2, \
             merged_count = ?3, \
             quality = MAX(COALESCE(quality, 50), ?4), \
             updated_at = ?5 \
         WHERE id = ?6",
        params![
            merged_context,
            MERGE_SCORE_BONUS,
            merged_count,
            quality,
            ts,
            target_id
        ],
    )
    .map_err(|e| StoreError::Internal(e.to_string()))?;

    let conflict_record_id = insert_conflict_record(
        &tx,
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
        &tx,
        "decision_agreement_merge",
        json!({
            "targetId": target_id,
            "source_agent": source_agent,
            "similarity_jaccard": relation.similarity_jaccard,
            "conflict_record_id": conflict_record_id,
        }),
        "rust-daemon",
    );

    tx.commit()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    checkpoint_wal_best_effort(conn);

    let mut entry = json!({
        "action": "merged",
        "target_id": target_id,
        "merged_count": merged_count,
        "quality": quality,
    });
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
    Ok((entry, None))
}

#[allow(clippy::too_many_arguments)]
fn handle_refinement_policy(
    conn: &mut Connection,
    decision: &str,
    context: Option<&str>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    expires_at: Option<&str>,
    ts: &str,
    owner_id: Option<i64>,
    relation: &ConflictResult,
) -> Result<(Value, Option<i64>), StoreError> {
    let target_id = relation
        .matched_id
        .ok_or_else(|| StoreError::Internal("Missing refinement target id".to_string()))?;
    let target_trust = relation.matched_trust_score.unwrap_or(0.8);
    let should_supersede =
        relation.matched_agent.as_deref() == Some(source_agent) || trust_score >= target_trust;

    let tx = conn
        .transaction()
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    if should_supersede {
        if let Some(owner_id) = owner_id {
            tx.execute(
                "UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2 AND owner_id = ?3",
                params![ts, target_id, owner_id],
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        } else {
            tx.execute(
                "UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2",
                params![ts, target_id],
            )
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        }
    }

    let new_id = insert_decision_with_state(
        &tx,
        decision,
        context,
        entry_type,
        source_agent,
        provenance,
        confidence,
        trust_score,
        quality,
        expires_at,
        ts,
        owner_id,
        if should_supersede {
            "active"
        } else {
            "disputed"
        },
        if should_supersede {
            None
        } else {
            Some(target_id)
        },
        if should_supersede {
            Some(target_id)
        } else {
            None
        },
        Some((1.0 - relation.similarity_jaccard).clamp(0.0, 1.0)),
    )?;

    let conflict_status = if should_supersede {
        "auto_resolved"
    } else {
        "open"
    };
    let strategy = if should_supersede {
        Some("refine_supersede")
    } else {
        Some("requires_user_review")
    };

    let conflict_record_id = insert_conflict_record(
        &tx,
        Some(new_id),
        target_id,
        relation.classification,
        relation.similarity_jaccard,
        relation.similarity_cosine,
        conflict_status,
        strategy,
        if should_supersede {
            Some("policy_engine")
        } else {
            None
        },
        ts,
    )?;

    let event_name = if should_supersede {
        "decision_supersede"
    } else {
        "decision_refine_pending"
    };
    let _ = log_event(
        &tx,
        event_name,
        json!({
            "newId": new_id,
            "targetId": target_id,
            "source_agent": source_agent,
            "strategy": strategy,
            "conflict_record_id": conflict_record_id,
        }),
        "rust-daemon",
    );

    tx.commit()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    checkpoint_wal_best_effort(conn);

    let mut entry = json!({
        "action": "inserted",
        "id": new_id,
        "status": if should_supersede { "superseded_old" } else { "disputed" },
        "quality": quality,
    });
    if should_supersede {
        entry["supersedes"] = json!(target_id);
    } else {
        entry["conflictWith"] = json!(target_id);
    }
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
}

#[allow(clippy::too_many_arguments)]
fn insert_decision_with_state(
    tx: &rusqlite::Transaction<'_>,
    decision: &str,
    context: Option<&str>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    expires_at: Option<&str>,
    ts: &str,
    owner_id: Option<i64>,
    status: &str,
    disputes_id: Option<i64>,
    supersedes_id: Option<i64>,
    surprise: Option<f64>,
) -> Result<i64, StoreError> {
    let surprise = surprise.map(round4);
    if let Some(oid) = owner_id {
        tx.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, disputes_id, supersedes_id, owner_id, quality, expires_at, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14, ?15, ?16, ?17)",
            params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                status,
                disputes_id,
                supersedes_id,
                oid,
                quality,
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score,
            ],
        )
    } else {
        tx.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, disputes_id, supersedes_id, quality, expires_at, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14, ?15, ?16)",
            params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                status,
                disputes_id,
                supersedes_id,
                quality,
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score,
            ],
        )
    }
    .map_err(|e| StoreError::Internal(e.to_string()))?;

    Ok(tx.last_insert_rowid())
}

#[allow(clippy::too_many_arguments)]
fn insert_conflict_record(
    tx: &rusqlite::Transaction<'_>,
    source_decision_id: Option<i64>,
    target_decision_id: i64,
    classification: ConflictClassification,
    similarity_jaccard: f64,
    similarity_cosine: Option<f64>,
    status: &str,
    resolution_strategy: Option<&str>,
    resolved_by: Option<&str>,
    ts: &str,
) -> Result<i64, StoreError> {
    let resolved_at = if status == "open" { None } else { Some(ts) };
    tx.execute(
        "INSERT INTO decision_conflicts \
         (source_decision_id, target_decision_id, classification, similarity_jaccard, similarity_cosine, status, resolution_strategy, resolved_by, resolved_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            source_decision_id,
            target_decision_id,
            classification.as_str(),
            round4(similarity_jaccard),
            similarity_cosine.map(round4),
            status,
            resolution_strategy,
            resolved_by,
            resolved_at,
            ts,
        ],
    )
    .map_err(|e| StoreError::Internal(e.to_string()))?;
    Ok(tx.last_insert_rowid())
}

fn decorate_entry_with_relation(
    entry: &mut Value,
    relation: &ConflictResult,
    conflict_record: Option<Value>,
) {
    if let Some(object) = entry.as_object_mut() {
        object.insert(
            "classification".to_string(),
            json!(relation.classification.as_str()),
        );
        object.insert("relation".to_string(), relation_to_json(relation));
        if let Some(conflict_record) = conflict_record {
            object.insert("conflict".to_string(), conflict_record);
        }
    }
}

fn relation_to_json(relation: &ConflictResult) -> Value {
    json!({
        "matched_id": relation.matched_id,
        "matched_agent": relation.matched_agent,
        "matched_trust_score": relation.matched_trust_score.map(round4),
        "similarity": {
            "jaccard": round4(relation.similarity_jaccard),
            "cosine": relation.similarity_cosine.map(round4),
        },
    })
}

fn conflict_record_json(
    record_id: i64,
    source_decision_id: Option<i64>,
    target_decision_id: i64,
    classification: ConflictClassification,
    status: &str,
    strategy: Option<&str>,
) -> Value {
    json!({
        "id": record_id,
        "source_decision_id": source_decision_id,
        "target_decision_id": target_decision_id,
        "classification": classification.as_str(),
        "status": status,
        "resolution_strategy": strategy,
    })
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn assess_quality(text: &str) -> QualityAssessment {
    let trimmed = text.trim();
    let len = trimmed.chars().count();
    let length_score = if len < 10 {
        0
    } else if len < 50 {
        30
    } else if len < 200 {
        70
    } else {
        100
    };

    let specificity_bonus = if has_specificity_markers(trimmed) {
        20
    } else {
        0
    };
    let question_penalty = if trimmed.ends_with('?') { -30 } else { 0 };
    let score = (length_score + specificity_bonus + question_penalty).clamp(0, 100);

    QualityAssessment {
        score,
        factors: QualityFactors {
            length_score,
            specificity_bonus,
            question_penalty,
        },
    }
}

fn has_specificity_markers(text: &str) -> bool {
    let lower = text.to_lowercase();
    let file_extensions = [
        ".rs", ".go", ".py", ".ts", ".tsx", ".js", ".jsx", ".json", ".toml", ".yaml", ".yml",
        ".sql", ".md",
    ];
    let code_prefixes = [
        "fn ", "func ", "def ", "class ", "struct ", "impl ", "select ", "insert ", "update ",
        "delete ",
    ];

    let has_path = text.contains('/') || text.contains('\\');
    let has_extension = file_extensions.iter().any(|ext| lower.contains(ext));
    let has_function = text.contains("::")
        || text.contains("()")
        || text.contains("->")
        || code_prefixes.iter().any(|needle| lower.contains(needle));
    let has_identifier = text
        .split_whitespace()
        .any(|token| token.contains('_') && token.chars().any(|ch| ch.is_ascii_alphabetic()));

    has_path || has_extension || has_function || has_identifier
}

fn choose_semantic_dedup_action(
    candidates: &[SemanticCandidate],
    incoming_text: &str,
) -> SemanticDedupAction {
    for candidate in candidates {
        let jaccard = jaccard_similarity(incoming_text, &candidate.decision);
        if should_merge_candidate(candidate.similarity, jaccard) {
            return SemanticDedupAction::Merge {
                target_id: candidate.id,
                similarity: candidate.similarity,
                jaccard,
            };
        }
    }
    SemanticDedupAction::Insert
}

fn should_merge_candidate(similarity: f32, jaccard: f64) -> bool {
    if similarity > HARD_MERGE_THRESHOLD {
        return true;
    }
    (REVIEW_MERGE_THRESHOLD..=HARD_MERGE_THRESHOLD).contains(&similarity)
        && jaccard > JACCARD_MERGE_THRESHOLD
}

fn fetch_top_semantic_candidates(
    conn: &Connection,
    query_vector: &[f32],
    owner_id: Option<i64>,
) -> Result<Vec<SemanticCandidate>, StoreError> {
    let (sql, has_owner_scope) = if owner_id.is_some() {
        (
            "SELECT d.id, d.decision, d.context, e.vector \
             FROM decisions d \
             JOIN embeddings e ON e.target_type = 'decision' AND e.target_id = d.id \
             WHERE d.owner_id = ?1 \
             AND d.status = 'active' \
             AND (d.expires_at IS NULL OR d.expires_at > datetime('now'))",
            true,
        )
    } else {
        (
            "SELECT d.id, d.decision, d.context, e.vector \
             FROM decisions d \
             JOIN embeddings e ON e.target_type = 'decision' AND e.target_id = d.id \
             WHERE d.status = 'active' \
             AND (d.expires_at IS NULL OR d.expires_at > datetime('now'))",
            false,
        )
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    let mut candidates = Vec::new();
    if has_owner_scope {
        let rows = stmt
            .query_map([owner_id.unwrap_or_default()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        for row in rows.flatten() {
            let (id, decision, _context, blob) = row;
            let existing_vec = crate::embeddings::blob_to_vector(&blob);
            if existing_vec.len() != query_vector.len() {
                continue;
            }
            let similarity = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
            candidates.push(SemanticCandidate {
                id,
                decision,
                similarity,
            });
        }
    } else {
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        for row in rows.flatten() {
            let (id, decision, _context, blob) = row;
            let existing_vec = crate::embeddings::blob_to_vector(&blob);
            if existing_vec.len() != query_vector.len() {
                continue;
            }
            let similarity = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
            candidates.push(SemanticCandidate {
                id,
                decision,
                similarity,
            });
        }
    }

    candidates.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(3);
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn merge_into_existing_decision(
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
    let tx = conn
        .transaction()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    let (existing_decision, existing_context, previous_merged_count): (
        String,
        Option<String>,
        i64,
    ) = if let Some(owner_id) = owner_id {
        tx.query_row(
            "SELECT decision, context, COALESCE(merged_count, 0) \
                 FROM decisions WHERE id = ?1 AND owner_id = ?2",
            params![target_id, owner_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?
    } else {
        tx.query_row(
            "SELECT decision, context, COALESCE(merged_count, 0) FROM decisions WHERE id = ?1",
            params![target_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?
    };

    let merged_context = merge_context(
        existing_context,
        &existing_decision,
        incoming_context,
        incoming_text,
    );
    let merged_count = previous_merged_count + 1;
    if let Some(owner_id) = owner_id {
        tx.execute(
            "UPDATE decisions \
             SET context = ?1, \
                 score = COALESCE(score, 0) + ?2, \
                 merged_count = ?3, \
                 quality = MAX(COALESCE(quality, 50), ?4), \
                 updated_at = ?5 \
             WHERE id = ?6 AND owner_id = ?7",
            params![
                merged_context,
                MERGE_SCORE_BONUS,
                merged_count,
                quality,
                ts,
                target_id,
                owner_id
            ],
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    } else {
        tx.execute(
            "UPDATE decisions \
             SET context = ?1, \
                 score = COALESCE(score, 0) + ?2, \
                 merged_count = ?3, \
                 quality = MAX(COALESCE(quality, 50), ?4), \
                 updated_at = ?5 \
             WHERE id = ?6",
            params![
                merged_context,
                MERGE_SCORE_BONUS,
                merged_count,
                quality,
                ts,
                target_id
            ],
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    }

    let _ = log_event(
        &tx,
        "merge",
        json!({
            "source_id": Value::Null,
            "target_id": target_id,
            "target_type": "decision",
            "incoming_text": incoming_text,
            "similarity": similarity,
            "jaccard": jaccard,
            "source_agent": source_agent,
        }),
        "rust-daemon",
    );
    tx.commit()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    checkpoint_wal_best_effort(conn);

    Ok((
        json!({
            "action": "merged",
            "target_id": target_id,
            "merged_count": merged_count,
            "quality": quality,
            "similarity": similarity,
            "jaccard": jaccard,
        }),
        None,
    ))
}

fn merge_context(
    existing_context: Option<String>,
    existing_decision: &str,
    incoming_context: Option<&str>,
    incoming_text: &str,
) -> Option<String> {
    let incoming_note = incoming_context
        .map(str::trim)
        .filter(|text| !text.is_empty())
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
fn insert_decision(
    conn: &Connection,
    decision: &str,
    context: Option<String>,
    entry_type: &str,
    source_agent: &str,
    provenance: &DecisionProvenance,
    confidence: f64,
    trust_score: f64,
    quality: i32,
    expires_at: Option<String>,
    ts: &str,
    owner_id: Option<i64>,
    surprise: f64,
    emit_decision_stored_event: bool,
) -> Result<(Value, Option<i64>), StoreError> {
    let surprise = (surprise * 10_000.0).round() / 10_000.0;
    if let Some(oid) = owner_id {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, owner_id, quality, expires_at, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?10, ?11, ?12, ?13, ?14)",
            params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                oid,
                quality,
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score,
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, quality, expires_at, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?9, ?10, ?11, ?12, ?13)",
            params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                quality,
                expires_at,
                ts,
                provenance.source_client.as_str(),
                provenance.source_model.as_deref(),
                provenance.reasoning_depth.as_str(),
                trust_score,
            ],
        )
    }
    .map_err(|e| StoreError::Internal(e.to_string()))?;

    let id = conn.last_insert_rowid();
    if emit_decision_stored_event {
        let _ = log_event(
            conn,
            "decision_stored",
            json!({
                "id": id,
                "source_agent": source_agent,
                "surprise": surprise,
                "quality": quality,
            }),
            "rust-daemon",
        );
    }
    checkpoint_wal_best_effort(conn);

    Ok((
        json!({
            "action": "inserted",
            "id": id,
            "status": "active",
            "surprise": surprise,
            "quality": quality,
        }),
        Some(id),
    ))
}

fn compute_expires_at(
    conn: &Connection,
    ttl_seconds: Option<i64>,
) -> Result<Option<String>, String> {
    let Some(ttl_seconds) = ttl_seconds else {
        return Ok(None);
    };
    let modifier = format!("+{ttl_seconds} seconds");
    conn.query_row("SELECT datetime('now', ?1)", params![modifier], |row| {
        row.get(0)
    })
    .map(Some)
    .map_err(|e| format!("Failed to compute expires_at: {e}"))
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn test_state() -> RuntimeState {
        let write_conn = test_conn();
        let read_conn = test_conn();
        let (events, _) = broadcast::channel(8);
        RuntimeState {
            db: Arc::new(Mutex::new(write_conn)),
            db_read: Arc::new(Mutex::new(read_conn)),
            token: Arc::new("test-token".to_string()),
            events,
            mcp_calls: Arc::new(AtomicU64::new(0)),
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            recall_history: Arc::new(Mutex::new(HashMap::new())),
            pre_cache: Arc::new(Mutex::new(HashMap::new())),
            served_content: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            home: PathBuf::from("."),
            db_path: PathBuf::from(":memory:"),
            token_path: PathBuf::from("cortex.token"),
            pid_path: PathBuf::from("cortex.pid"),
            port: 7437,
            embedding_engine: None,
            rate_limiter: crate::rate_limit::RateLimiter::new(),
            team_mode: false,
            default_owner_id: None,
            team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
            degraded_mode: Arc::new(AtomicBool::new(false)),
            db_corrupted: Arc::new(AtomicBool::new(false)),
            readiness: Arc::new(AtomicBool::new(true)),
            last_activity_unix_secs: Arc::new(AtomicU64::new(0)),
            write_buffer_path: PathBuf::from("write_buffer.jsonl"),
            sqlite_vec_canary: crate::state::SqliteVecCanaryConfig {
                trial_percent: 0,
                force_off: false,
                route_mode: crate::state::SqliteVecRouteMode::Trial,
            },
        }
    }

    fn unit_vector_for_similarity(similarity: f32) -> Vec<f32> {
        vec![similarity, (1.0 - similarity * similarity).sqrt()]
    }

    #[test]
    fn provenance_normalizes_client_model_and_depth() {
        let provenance = DecisionProvenance::from_fields(
            "Codex (GPT-5.4)",
            Some("claude-opus-4.1"),
            Some("multi_step"),
        );
        assert_eq!(provenance.source_client, "codex");
        assert_eq!(provenance.source_model.as_deref(), Some("claude-opus-4.1"));
        assert_eq!(provenance.reasoning_depth, "multi-step");
    }

    #[test]
    fn trust_score_prefers_stronger_models() {
        let weak = compute_trust_score(0.9, Some("qwen-30b"));
        let strong = compute_trust_score(0.9, Some("claude-opus-4.1"));
        assert!(strong > weak);
        assert_eq!(strong, 0.9);
    }

    fn insert_existing_decision(
        conn: &Connection,
        decision: &str,
        context: Option<&str>,
        vector: &[f32],
    ) -> i64 {
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 50, datetime('now'), datetime('now'))",
            params![decision, context],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key())
            .unwrap();
        id
    }

    fn insert_legacy_decision(
        conn: &Connection,
        decision: &str,
        source_agent: &str,
        trust_score: f64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, confidence, trust_score, status, quality, created_at, updated_at) \
             VALUES (?1, ?2, 'decision', ?3, ?4, ?4, 'active', 70, datetime('now'), datetime('now'))",
            params![decision, "seed", source_agent, trust_score],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn persist_decision_embedding_uses_explicit_model_key() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('model-tag check', 'ctx', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();

        persist_decision_embedding(&conn, id, &[0.3, 0.4, 0.5], "unit-test-model").unwrap();

        let stored_model: String = conn
            .query_row(
                "SELECT model FROM embeddings WHERE target_type = 'decision' AND target_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_model, "unit-test-model");
    }

    #[test]
    fn store_with_provenance_persists_trust_fields() {
        let mut conn = test_conn();
        let provenance = DecisionProvenance::from_fields(
            "codex",
            Some("claude-opus-4.1"),
            Some("tool-assisted"),
        );

        let (_, new_id) = store_decision_with_input_embedding_and_provenance(
            &mut conn,
            "persist provenance for memory trust",
            Some("unit test".to_string()),
            Some("decision".to_string()),
            "codex".to_string(),
            provenance,
            Some(0.9),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let (source_client, source_model, reasoning_depth, trust_score): (
            String,
            Option<String>,
            String,
            f64,
        ) = conn
            .query_row(
                "SELECT source_client, source_model, reasoning_depth, trust_score FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(source_client, "codex");
        assert_eq!(source_model.as_deref(), Some("claude-opus-4.1"));
        assert_eq!(reasoning_depth, "tool-assisted");
        assert_eq!(trust_score, 0.9);
    }

    #[test]
    fn semantic_dedup_threshold_boundaries() {
        assert!(!should_merge_candidate(0.89, 0.95));
        assert!(!should_merge_candidate(0.90, 0.70));
        assert!(should_merge_candidate(0.90, 0.71));
        assert!(should_merge_candidate(0.91, 0.71));
        assert!(should_merge_candidate(0.92, 0.71));
        assert!(should_merge_candidate(0.93, 0.00));
    }

    #[test]
    fn benchmark_entries_bypass_semantic_merge() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "store benchmark messages without dedup collapsing",
            Some("seed"),
            &[1.0, 0.0],
        );

        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "store benchmark messages without dedup collapsing",
            Some("bench-doc".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.99)),
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn benchmark_entries_allow_vague_payloads() {
        let mut conn = test_conn();
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "?",
            Some("bench-doc".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
    }

    #[test]
    fn non_benchmark_decisions_are_length_capped() {
        let mut conn = test_conn();
        let long_text = "x".repeat(MAX_DECISION_CHARS + 1800);
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            &long_text,
            Some("long decision body".to_string()),
            Some("decision".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let id = new_id.expect("decision id");
        let stored_chars: i64 = conn
            .query_row(
                "SELECT LENGTH(decision) FROM decisions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_chars as usize, MAX_DECISION_CHARS);

        let truncation_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_truncated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(truncation_events, 1);
    }

    #[test]
    fn benchmark_decisions_keep_full_length() {
        let mut conn = test_conn();
        let long_text = "x".repeat(MAX_DECISION_CHARS + 1800);
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            &long_text,
            Some("benchmark payload".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let id = new_id.expect("decision id");
        let stored_chars: i64 = conn
            .query_row(
                "SELECT LENGTH(decision) FROM decisions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_chars as usize, long_text.len());
    }

    #[test]
    fn merge_behavior_increments_count_and_appends_context() {
        let mut conn = test_conn();
        insert_existing_decision(&conn, "use early returns in Go code", None, &[1.0, 0.0]);

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "always use early returns",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.93)),
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        assert_eq!(entry["action"], "merged");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (merged_count, score, context): (i64, f64, Option<String>) = conn
            .query_row(
                "SELECT merged_count, score, context FROM decisions LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(merged_count, 1);
        assert_eq!(score, 6.0);
        assert!(context.unwrap().contains("always use early returns"));

        let merge_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'merge'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(merge_events, 1);
    }

    #[test]
    fn jaccard_review_band_merges_when_tokens_match() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "use early returns in go code",
            Some("initial"),
            &[1.0, 0.0],
        );

        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "use early returns in go code today",
            Some("follow-up".to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.91)),
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        let merged_count: i64 = conn
            .query_row("SELECT merged_count FROM decisions LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(merged_count, 1);
    }

    #[test]
    fn jaccard_review_band_inserts_when_tokens_do_not_match() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "database migrations need backups",
            Some("initial"),
            &[1.0, 0.0],
        );

        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "always use early returns",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.91)),
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn contradiction_policy_keeps_higher_trust_decision_active() {
        let mut conn = test_conn();
        let existing_id =
            insert_legacy_decision(&conn, "always run migrations before deploy", "claude", 0.95);

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "never run migrations before deploy",
            Some("contradiction".to_string()),
            None,
            "codex".to_string(),
            Some(0.6),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        assert_eq!(entry["classification"], "CONTRADICTS");
        assert_eq!(entry["status"], "disputed");
        assert_eq!(entry["conflict"]["status"], "auto_resolved");

        let existing_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(existing_status, "active");

        let inserted_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(inserted_status, "disputed");

        let conflict_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_conflicts WHERE source_decision_id = ?1 AND target_decision_id = ?2 AND classification = 'CONTRADICTS'",
                params![new_id, existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_rows, 1);
    }

    #[test]
    fn refinement_policy_supersedes_same_agent_decision() {
        let mut conn = test_conn();
        let existing_id = insert_legacy_decision(
            &conn,
            "use structured logging for daemon requests",
            "codex",
            0.6,
        );

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "use structured logging with request ids for daemon requests",
            Some("refinement".to_string()),
            None,
            "codex".to_string(),
            Some(0.7),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        assert_eq!(entry["classification"], "REFINES");
        assert_eq!(entry["status"], "superseded_old");
        assert_eq!(entry["conflict"]["status"], "auto_resolved");
        assert_eq!(entry["supersedes"], existing_id);

        let old_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "superseded");

        let new_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_status, "active");
    }

    #[test]
    fn agreement_policy_merges_duplicate_without_inserting_new_decision() {
        let mut conn = test_conn();
        let target_id = insert_legacy_decision(
            &conn,
            "enable recall cache warming at startup",
            "claude",
            0.8,
        );

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "enable recall cache warming at startup",
            Some("same intent".to_string()),
            None,
            "codex".to_string(),
            Some(0.9),
            None,
            None,
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        assert_eq!(entry["action"], "merged");
        assert_eq!(entry["classification"], "AGREES");
        assert_eq!(entry["target_id"], target_id);
        assert_eq!(entry["conflict"]["status"], "auto_resolved");

        let decision_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(decision_count, 1);

        let conflict_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_conflicts WHERE source_decision_id IS NULL AND target_decision_id = ?1 AND classification = 'AGREES'",
                params![target_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_rows, 1);
    }

    #[test]
    fn quality_scoring_edge_cases() {
        let empty = assess_quality("");
        assert_eq!(empty.score, 0);

        let question = assess_quality("?");
        assert_eq!(question.score, 0);

        let long_specific = assess_quality(
            "Update daemon-rs/src/handlers/store.rs so handle_store() appends merge context and keeps the score bump when semantic dedup hits the review band threshold for near-duplicate decision text.",
        );
        assert_eq!(long_specific.score, 90);

        let code_snippet = assess_quality("fn handle_store() { return Ok(()); }");
        assert_eq!(code_snippet.score, 50);
    }

    #[test]
    fn detailed_store_persists_quality_score() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "Always use rtk prefix for shell commands in Cortex repo",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let quality: i64 = conn
            .query_row(
                "SELECT quality FROM decisions WHERE id = ?1",
                [new_id.unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(quality >= 70);
    }

    #[test]
    fn rejection_at_quality_below_twenty() {
        let mut conn = test_conn();
        let err = store_decision_with_input_embedding(
            &mut conn,
            "?",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap_err();

        match err {
            StoreError::Validation {
                message, quality, ..
            } => {
                assert_eq!(message, "Memory too vague");
                assert_eq!(quality, 0);
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_store_returns_http_400_for_vague_input() {
        let state = test_state();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-token".parse().unwrap());
        headers.insert("x-cortex-request", "true".parse().unwrap());

        let response = handle_store(
            State(state),
            headers,
            Json(StoreRequest {
                decision: Some("?".to_string()),
                ..StoreRequest::default()
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn store_decision_with_ttl_sets_expires_at() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "temporary decision with enough detail to persist",
            Some("ttl-test".to_string()),
            None,
            "tester".to_string(),
            None,
            Some(3600),
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let expires_at: Option<String> = conn
            .query_row(
                "SELECT expires_at FROM decisions WHERE id = ?1",
                [new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(expires_at.is_some());

        let expires_in_future: i64 = conn
            .query_row(
                "SELECT expires_at > datetime('now') FROM decisions WHERE id = ?1",
                [new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expires_in_future, 1);
    }

    #[test]
    fn store_decision_without_ttl_leaves_expires_at_null() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "persistent decision with enough detail to persist",
            Some("ttl-test".to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let expires_at: Option<String> = conn
            .query_row(
                "SELECT expires_at FROM decisions WHERE id = ?1",
                [new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(expires_at.is_none());
    }
}
