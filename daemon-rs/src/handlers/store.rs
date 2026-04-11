// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ensure_auth_with_caller, json_response, log_event, now_iso};
use crate::conflict::{detect_conflict, jaccard_similarity};
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

const HARD_MERGE_THRESHOLD: f32 = 0.92;
const REVIEW_MERGE_THRESHOLD: f32 = 0.90;
const JACCARD_MERGE_THRESHOLD: f64 = 0.70;
const MERGE_SCORE_BONUS: f64 = 5.0;
const TOO_VAGUE_THRESHOLD: i32 = 20;

#[derive(Deserialize, Default)]
pub struct StoreRequest {
    pub decision: Option<String>,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub source_agent: Option<String>,
    pub confidence: Option<f64>,
    pub ttl_seconds: Option<i64>,
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

    if let Some(ttl_seconds) = body.ttl_seconds {
        if ttl_seconds <= 0 {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "ttl_seconds must be > 0" }),
            );
        }
    }

    let decision_text = decision.trim().to_string();
    let decision_embedding = state
        .embedding_engine
        .as_ref()
        .and_then(|engine| engine.embed(&decision_text));

    let mut conn = state.db.lock().await;
    let result = store_decision_with_input_embedding(
        &mut conn,
        &decision_text,
        body.context,
        body.entry_type,
        source_agent.clone(),
        body.confidence,
        body.ttl_seconds,
        decision_embedding.as_deref(),
        caller_id,
    );

    match result {
        Ok((entry, new_id)) => {
            if let Some(id) = new_id {
                if let Some(vec) = decision_embedding.as_deref() {
                    if let Err(err) = persist_decision_embedding(&conn, id, vec) {
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
                            let _ = persist_decision_embedding(&conn, id, &vec);
                        }
                    });
                }
            }

            crate::focus::focus_append(&conn, &source_agent, &decision_text);
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
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
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
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        confidence,
        ttl_seconds,
        None,
        owner_id,
    )
    .map_err(|err| err.to_string())
}

#[allow(clippy::too_many_arguments)]
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
    store_decision_internal(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
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
    confidence: Option<f64>,
    ttl_seconds: Option<i64>,
    query_embedding: Option<&[f32]>,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    let decision = decision.trim();
    let quality = assess_quality(decision);
    if quality.score < TOO_VAGUE_THRESHOLD {
        return Err(StoreError::Validation {
            message: "Memory too vague",
            quality: quality.score,
            factors: quality.factors,
        });
    }

    let entry_type = entry_type.unwrap_or_else(|| "decision".to_string());
    let confidence = confidence.unwrap_or(0.8);
    let ts = now_iso();
    let expires_at = compute_expires_at(conn, ttl_seconds).map_err(StoreError::Internal)?;

    if let Some(query_vector) = query_embedding {
        let candidates = fetch_top_semantic_candidates(conn, query_vector)?;
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
            );
        }

        return insert_decision(
            conn,
            decision,
            context,
            &entry_type,
            &source_agent,
            confidence,
            quality.score,
            expires_at,
            &ts,
            owner_id,
            (1.0 - best_similarity).clamp(0.0, 1.0),
        );
    }

    store_decision_legacy(
        conn,
        decision,
        context,
        &entry_type,
        &source_agent,
        confidence,
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
    confidence: f64,
    quality: i32,
    expires_at: Option<String>,
    ts: &str,
    owner_id: Option<i64>,
) -> Result<(Value, Option<i64>), StoreError> {
    let cr = detect_conflict(conn, decision, source_agent).map_err(StoreError::Internal)?;

    if cr.is_conflict {
        let existing_id = cr
            .matched_id
            .ok_or_else(|| StoreError::Internal("Missing conflict target id".to_string()))?;
        let tx = conn.transaction().map_err(|e| StoreError::Internal(e.to_string()))?;
        if let Some(oid) = owner_id {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, status, disputes_id, owner_id, quality, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'disputed', ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    decision,
                    context,
                    entry_type,
                    source_agent,
                    confidence,
                    existing_id,
                    oid,
                    quality,
                    expires_at,
                    ts
                ],
            )
        } else {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, status, disputes_id, quality, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'disputed', ?6, ?7, ?8, ?9, ?9)",
                params![
                    decision,
                    context,
                    entry_type,
                    source_agent,
                    confidence,
                    existing_id,
                    quality,
                    expires_at,
                    ts
                ],
            )
        }
        .map_err(|e| StoreError::Internal(e.to_string()))?;
        let new_id = tx.last_insert_rowid();

        tx.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_id, ts, existing_id],
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;

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
        tx.commit()
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        checkpoint_wal_best_effort(conn);

        return Ok((
            json!({
                "action": "inserted",
                "id": new_id,
                "status": "disputed",
                "conflictWith": existing_id,
                "quality": quality,
            }),
            Some(new_id),
        ));
    }

    if cr.is_update {
        let old_id = cr
            .matched_id
            .ok_or_else(|| StoreError::Internal("Missing supersede target id".to_string()))?;
        let tx = conn.transaction().map_err(|e| StoreError::Internal(e.to_string()))?;
        tx.execute(
            "UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2",
            params![ts, old_id],
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        if let Some(oid) = owner_id {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, supersedes_id, owner_id, quality, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    decision,
                    context,
                    entry_type,
                    source_agent,
                    confidence,
                    old_id,
                    oid,
                    quality,
                    expires_at,
                    ts
                ],
            )
        } else {
            tx.execute(
                "INSERT INTO decisions \
                 (decision, context, type, source_agent, confidence, supersedes_id, quality, expires_at, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                params![
                    decision,
                    context,
                    entry_type,
                    source_agent,
                    confidence,
                    old_id,
                    quality,
                    expires_at,
                    ts
                ],
            )
        }
        .map_err(|e| StoreError::Internal(e.to_string()))?;
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
        tx.commit()
            .map_err(|e| StoreError::Internal(e.to_string()))?;
        checkpoint_wal_best_effort(conn);

        return Ok((
            json!({
                "action": "inserted",
                "id": new_id,
                "status": "superseded_old",
                "supersedes": old_id,
                "quality": quality,
            }),
            Some(new_id),
        ));
    }

    let existing: Vec<String> = {
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
            .map_err(|e| StoreError::Internal(e.to_string()))?
            .filter_map(|row| row.ok())
            .collect();
        rows
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
        return Ok((
            json!({
                "stored": false,
                "reason": "duplicate",
                "surprise": surprise,
                "quality": quality,
            }),
            None,
        ));
    }

    insert_decision(
        conn,
        decision,
        context,
        entry_type,
        source_agent,
        confidence,
        quality,
        expires_at,
        ts,
        owner_id,
        surprise,
    )
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

    let specificity_bonus = if has_specificity_markers(trimmed) { 20 } else { 0 };
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
        "fn ",
        "func ",
        "def ",
        "class ",
        "struct ",
        "impl ",
        "select ",
        "insert ",
        "update ",
        "delete ",
    ];

    let has_path = text.contains('/') || text.contains('\\');
    let has_extension = file_extensions.iter().any(|ext| lower.contains(ext));
    let has_function = text.contains("::")
        || text.contains("()")
        || text.contains("->")
        || code_prefixes.iter().any(|needle| lower.contains(needle));
    let has_identifier = text.split_whitespace().any(|token| {
        token.contains('_') && token.chars().any(|ch| ch.is_ascii_alphabetic())
    });

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
) -> Result<Vec<SemanticCandidate>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT d.id, d.decision, d.context, e.vector \
             FROM decisions d \
             JOIN embeddings e ON e.target_type = 'decision' AND e.target_id = d.id \
             WHERE d.status = 'active' \
             AND (d.expires_at IS NULL OR d.expires_at > datetime('now'))",
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;

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

    let mut candidates = Vec::new();
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
) -> Result<(Value, Option<i64>), StoreError> {
    let tx = conn
        .transaction()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    let (existing_decision, existing_context, previous_merged_count): (String, Option<String>, i64) = tx
        .query_row(
            "SELECT decision, context, COALESCE(merged_count, 0) FROM decisions WHERE id = ?1",
            params![target_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| StoreError::Internal(e.to_string()))?;

    let merged_context = merge_context(
        existing_context,
        &existing_decision,
        incoming_context,
        incoming_text,
    );
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
    confidence: f64,
    quality: i32,
    expires_at: Option<String>,
    ts: &str,
    owner_id: Option<i64>,
    surprise: f64,
) -> Result<(Value, Option<i64>), StoreError> {
    let surprise = (surprise * 10_000.0).round() / 10_000.0;
    if let Some(oid) = owner_id {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, owner_id, quality, expires_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?10, ?10)",
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
                ts
            ],
        )
    } else {
        conn.execute(
            "INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, quality, expires_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?9)",
            params![
                decision,
                context,
                entry_type,
                source_agent,
                confidence,
                surprise,
                quality,
                expires_at,
                ts
            ],
        )
    }
    .map_err(|e| StoreError::Internal(e.to_string()))?;

    let id = conn.last_insert_rowid();
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
) -> Result<(), String> {
    let blob = crate::embeddings::vector_to_blob(vector);
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
         VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
        params![decision_id, blob],
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
            write_buffer_path: PathBuf::from("write_buffer.jsonl"),
        }
    }

    fn unit_vector_for_similarity(similarity: f32) -> Vec<f32> {
        vec![similarity, (1.0 - similarity * similarity).sqrt()]
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
        persist_decision_embedding(conn, id, vector).unwrap();
        id
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
