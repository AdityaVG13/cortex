// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget};
use crate::handlers::{
    estimate_tokens, json_response, now_iso, parse_timestamp_ms, resolve_source_identity,
    truncate_chars,
};

use super::*;
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

pub(crate) fn collect_semantic_candidates(
    conn: &Connection,
    query_vector: &[f32],
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Vec<SemanticCandidate> {
    let selected_model = crate::embeddings::selected_model_key();
    let expected_vector_bytes = std::mem::size_of_val(query_vector) as i64;
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let scale_sim = |sim: f32| -> f64 {
        SEMANTIC_SCALE_BASE
            + (sim as f64 - SEMANTIC_SIM_FLOOR)
                * ((1.0 - SEMANTIC_SCALE_BASE) / (1.0 - SEMANTIC_SIM_FLOOR))
    };
    let keyword_terms = extract_search_keywords(query_text);
    let semantic_floor = if keyword_terms.len() >= 3 {
        SEMANTIC_SIM_FLOOR + 0.12
    } else {
        SEMANTIC_SIM_FLOOR
    };

    let mut candidates: HashMap<String, SemanticCandidate> = HashMap::new();

    let semantic_memory_query_with_acl = "SELECT e.vector, m.text, m.source, m.owner_id, m.visibility, m.score, m.trust_score, m.last_accessed, m.created_at \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let semantic_memory_query_without_acl = "SELECT e.vector, m.text, m.source, NULL AS owner_id, NULL AS visibility, m.score, m.trust_score, m.last_accessed, m.created_at \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let semantic_memory_stmt = match conn.prepare(semantic_memory_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(semantic_memory_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = semantic_memory_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<MemorySemanticRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        ) {
            for (
                blob,
                text,
                source,
                owner_id,
                visibility,
                score,
                trust_score,
                last_accessed,
                created_at,
            ) in rows.flatten()
            {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                let existing_vec = crate::embeddings::blob_to_vector(&blob);
                let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
                if sim <= semantic_floor as f32 {
                    continue;
                }

                let mut scaled = scale_sim(sim);
                if !keyword_terms.is_empty() {
                    let haystack = text.to_lowercase();
                    let overlap = keyword_terms
                        .iter()
                        .filter(|term| haystack.contains(term.as_str()))
                        .count();
                    if overlap == 0 {
                        scaled *= 0.82;
                    } else {
                        let ratio = overlap as f64 / keyword_terms.len().max(1) as f64;
                        scaled *= 1.0 + ratio * 0.08;
                    }
                }
                let excerpt = query_focused_excerpt(&text, query_text, 280);
                let importance = blend_importance(score, trust_score);
                let ts_source = last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or_default();
                let ts = parse_timestamp_ms(ts_source);
                let entry = candidates
                    .entry(source.clone())
                    .or_insert(SemanticCandidate {
                        source,
                        excerpt: excerpt.clone(),
                        relevance: scaled,
                        importance,
                        ts,
                    });
                if scaled > entry.relevance {
                    *entry = SemanticCandidate {
                        source: entry.source.clone(),
                        excerpt,
                        relevance: scaled,
                        importance,
                        ts,
                    };
                }
            }
        }
    }

    let semantic_decision_query_with_acl = "SELECT e.vector, d.decision, d.context, d.owner_id, d.visibility, d.score, d.trust_score, d.last_accessed, d.created_at \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let semantic_decision_query_without_acl = "SELECT e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility, d.score, d.trust_score, d.last_accessed, d.created_at \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let semantic_decision_stmt = match conn.prepare(semantic_decision_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(semantic_decision_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = semantic_decision_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<DecisionSemanticRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        ) {
            for (
                blob,
                decision,
                context,
                owner_id,
                visibility,
                score,
                trust_score,
                last_accessed,
                created_at,
            ) in rows.flatten()
            {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                let existing_vec = crate::embeddings::blob_to_vector(&blob);
                let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
                if sim <= semantic_floor as f32 {
                    continue;
                }

                let source = context.unwrap_or_else(|| {
                    format!(
                        "decision::{}",
                        decision.chars().take(40).collect::<String>()
                    )
                });
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                let mut scaled = scale_sim(sim);
                if !keyword_terms.is_empty() {
                    let haystack = decision.to_lowercase();
                    let overlap = keyword_terms
                        .iter()
                        .filter(|term| haystack.contains(term.as_str()))
                        .count();
                    if overlap == 0 {
                        scaled *= 0.82;
                    } else {
                        let ratio = overlap as f64 / keyword_terms.len().max(1) as f64;
                        scaled *= 1.0 + ratio * 0.08;
                    }
                }
                let excerpt = query_focused_excerpt(&decision, query_text, 280);
                let importance = blend_importance(score, trust_score);
                let ts_source = last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or_default();
                let ts = parse_timestamp_ms(ts_source);
                let entry = candidates
                    .entry(source.clone())
                    .or_insert(SemanticCandidate {
                        source,
                        excerpt: excerpt.clone(),
                        relevance: scaled,
                        importance,
                        ts,
                    });
                if scaled > entry.relevance {
                    *entry = SemanticCandidate {
                        source: entry.source.clone(),
                        excerpt,
                        relevance: scaled,
                        importance,
                        ts,
                    };
                }
            }
        }
    }

    let mut sorted: Vec<SemanticCandidate> = candidates.into_values().collect();
    sorted.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    sorted.truncate(MAX_SEMANTIC_RRF_CANDIDATES);
    sorted
}

pub(crate) fn collect_shadow_semantic_rows(
    conn: &Connection,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    expected_dimension: usize,
) -> Vec<ShadowSemanticRow> {
    let selected_model = crate::embeddings::selected_model_key();
    let expected_vector_bytes = (expected_dimension * std::mem::size_of::<f32>()) as i64;
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let mut rows_by_source: HashMap<String, Vec<f32>> = HashMap::new();

    let memory_query_with_acl = "SELECT e.vector, m.source, m.owner_id, m.visibility \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let memory_query_without_acl = "SELECT e.vector, m.source, NULL AS owner_id, NULL AS visibility \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let memory_stmt = match conn.prepare(memory_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(memory_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = memory_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<ShadowMemoryRow> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            },
        ) {
            for (blob, source, owner_id, visibility) in rows.flatten() {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                rows_by_source
                    .entry(source)
                    .or_insert_with(|| crate::embeddings::blob_to_vector(&blob));
            }
        }
    }

    let decision_query_with_acl = "SELECT e.vector, d.decision, d.context, d.owner_id, d.visibility \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let decision_query_without_acl = "SELECT e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let decision_stmt = match conn.prepare(decision_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(decision_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = decision_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<ShadowDecisionRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        ) {
            for (blob, decision, context, owner_id, visibility) in rows.flatten() {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                let source = context.unwrap_or_else(|| {
                    format!(
                        "decision::{}",
                        decision.chars().take(40).collect::<String>()
                    )
                });
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                rows_by_source
                    .entry(source)
                    .or_insert_with(|| crate::embeddings::blob_to_vector(&blob));
            }
        }
    }

    let mut rows: Vec<ShadowSemanticRow> = rows_by_source
        .into_iter()
        .map(|(source, vector)| ShadowSemanticRow { source, vector })
        .collect();
    rows.sort_by(|a, b| a.source.cmp(&b.source));
    rows
}

pub(crate) fn vector_to_vec0_literal(vector: &[f32]) -> String {
    let mut literal = String::with_capacity(vector.len().saturating_mul(12).saturating_add(2));
    literal.push('[');
    for (idx, value) in vector.iter().enumerate() {
        if idx > 0 {
            literal.push_str(", ");
        }
        let stable = if value.is_finite() { *value } else { 0.0 };
        let _ = write!(&mut literal, "{stable}");
    }
    literal.push(']');
    literal
}

pub(crate) fn run_sqlite_vec_shadow_knn_sources(
    conn: &Connection,
    query_vector: &[f32],
    candidates: &[ShadowSemanticRow],
    top_k: usize,
) -> Result<Vec<String>, String> {
    if query_vector.is_empty() || candidates.is_empty() {
        return Ok(Vec::new());
    }

    const SHADOW_TABLE: &str = "cortex_shadow_semantic_knn";
    let k = top_k.max(1).min(candidates.len());
    let query_literal = vector_to_vec0_literal(query_vector);
    let result = (|| -> Result<Vec<String>, String> {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {SHADOW_TABLE};"))
            .map_err(|err| format!("sqlite-vec shadow drop failed: {err}"))?;
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE {SHADOW_TABLE} USING vec0(\
                candidate_id INTEGER PRIMARY KEY,\
                embedding FLOAT[{}]\
            );",
            query_vector.len()
        ))
        .map_err(|err| format!("sqlite-vec shadow create failed: {err}"))?;

        let insert_sql =
            format!("INSERT INTO {SHADOW_TABLE}(candidate_id, embedding) VALUES (?1, ?2)");
        let mut insert_stmt = conn
            .prepare(&insert_sql)
            .map_err(|err| format!("sqlite-vec shadow insert prepare failed: {err}"))?;
        for (candidate_idx, candidate) in candidates.iter().enumerate() {
            let candidate_id = i64::try_from(candidate_idx + 1)
                .map_err(|_| "sqlite-vec shadow candidate id overflow".to_string())?;
            let embedding_literal = vector_to_vec0_literal(&candidate.vector);
            insert_stmt
                .execute(params![candidate_id, embedding_literal])
                .map_err(|err| format!("sqlite-vec shadow insert failed: {err}"))?;
        }

        let k_i64 = i64::try_from(k).map_err(|_| "sqlite-vec shadow k overflow".to_string())?;
        let query_sql = format!(
            "SELECT candidate_id, distance \
             FROM {SHADOW_TABLE} \
             WHERE embedding MATCH ?1 AND k = ?2"
        );
        let mut query_stmt = conn
            .prepare(&query_sql)
            .map_err(|err| format!("sqlite-vec shadow query prepare failed: {err}"))?;
        let rows = query_stmt
            .query_map(params![query_literal, k_i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|err| format!("sqlite-vec shadow query failed: {err}"))?;

        let mut sources = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let (candidate_id, _distance) =
                row.map_err(|err| format!("sqlite-vec shadow row decode failed: {err}"))?;
            if candidate_id <= 0 {
                continue;
            }
            let Some(candidate) = candidates.get((candidate_id - 1) as usize) else {
                continue;
            };
            if seen.insert(candidate.source.clone()) {
                sources.push(candidate.source.clone());
            }
        }

        Ok(sources)
    })();

    let _ = conn.execute_batch(&format!("DROP TABLE IF EXISTS {SHADOW_TABLE};"));
    result
}

pub(crate) fn shadow_error_to_unavailable_reason(error: &str) -> Option<&'static str> {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("no such module: vec0") {
        return Some("sqlite_vec_not_available");
    }
    None
}

pub(crate) fn build_shadow_semantic_explain(
    conn: &Connection,
    query_vector: Option<&[f32]>,
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    top_k: usize,
    baseline_override: Option<&ShadowSemanticBaseline>,
) -> Value {
    let top_k = top_k.clamp(1, MAX_SEMANTIC_RRF_CANDIDATES);
    let Some(query_vector) = query_vector else {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "query_embedding_unavailable",
            "topK": top_k
        });
    };
    if query_vector.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "query_embedding_empty",
            "topK": top_k
        });
    }

    let (baseline_candidate_count, baseline_top_sources) = if let Some(baseline) = baseline_override
    {
        (baseline.candidate_count, baseline.top_sources(top_k))
    } else {
        let baseline =
            collect_semantic_candidates(conn, query_vector, query_text, ctx, source_prefix);
        let top_sources = baseline
            .iter()
            .take(top_k)
            .map(|candidate| candidate.source.clone())
            .collect();
        (baseline.len(), top_sources)
    };

    let rows = collect_shadow_semantic_rows(conn, ctx, source_prefix, query_vector.len());
    if rows.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "no_shadow_candidates",
            "topK": top_k,
            "baselineCandidateCount": baseline_candidate_count,
            "baselineTopSources": baseline_top_sources,
        });
    }

    let vector_dim = query_vector.len();
    let compatible_rows: Vec<ShadowSemanticRow> = rows
        .into_iter()
        .filter(|row| row.vector.len() == vector_dim)
        .collect();
    if compatible_rows.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "no_dimension_compatible_candidates",
            "topK": top_k,
            "vectorDimension": vector_dim,
            "baselineCandidateCount": baseline_candidate_count,
            "baselineTopSources": baseline_top_sources,
        });
    }

    let compatible_count = compatible_rows.len();
    let shadow_top_sources =
        match run_sqlite_vec_shadow_knn_sources(conn, query_vector, &compatible_rows, top_k) {
            Ok(sources) => sources,
            Err(error) => {
                if let Some(reason) = shadow_error_to_unavailable_reason(&error) {
                    return json!({
                        "enabled": true,
                        "status": "unavailable",
                        "reason": reason,
                        "detail": error,
                        "topK": top_k,
                        "vectorDimension": vector_dim,
                        "baselineCandidateCount": baseline_candidate_count,
                        "shadowCandidateCount": compatible_count,
                        "baselineTopSources": baseline_top_sources,
                    });
                }
                return json!({
                    "enabled": true,
                    "status": "error",
                    "reason": error,
                    "topK": top_k,
                    "vectorDimension": vector_dim,
                    "baselineCandidateCount": baseline_candidate_count,
                    "shadowCandidateCount": compatible_count,
                    "baselineTopSources": baseline_top_sources,
                });
            }
        };

    let baseline_set: HashSet<&str> = baseline_top_sources.iter().map(String::as_str).collect();
    let shadow_set: HashSet<&str> = shadow_top_sources.iter().map(String::as_str).collect();
    let overlap_count = baseline_set.intersection(&shadow_set).count();
    let union_count = baseline_set.union(&shadow_set).count();
    let overlap_ratio = if top_k == 0 {
        0.0
    } else {
        round4(overlap_count as f64 / top_k as f64)
    };
    let jaccard = if union_count == 0 {
        1.0
    } else {
        round4(overlap_count as f64 / union_count as f64)
    };
    let baseline_index: HashMap<&str, usize> = baseline_top_sources
        .iter()
        .enumerate()
        .map(|(idx, source)| (source.as_str(), idx))
        .collect();
    let shadow_index: HashMap<&str, usize> = shadow_top_sources
        .iter()
        .enumerate()
        .map(|(idx, source)| (source.as_str(), idx))
        .collect();
    let mut matched_rank_pairs: usize = 0;
    let mut rank_delta_sum: usize = 0;
    for (source, baseline_rank) in &baseline_index {
        if let Some(shadow_rank) = shadow_index.get(source) {
            matched_rank_pairs += 1;
            rank_delta_sum += baseline_rank.abs_diff(*shadow_rank);
        }
    }
    let mean_abs_rank_delta = if matched_rank_pairs > 0 {
        Some(round4(rank_delta_sum as f64 / matched_rank_pairs as f64))
    } else {
        None
    };
    let top1_match = match (
        baseline_top_sources.first().map(String::as_str),
        shadow_top_sources.first().map(String::as_str),
    ) {
        (Some(left), Some(right)) => Some(left == right),
        _ => None,
    };

    json!({
        "enabled": true,
        "status": "ok",
        "topK": top_k,
        "vectorDimension": vector_dim,
        "baselineCandidateCount": baseline_candidate_count,
        "shadowCandidateCount": compatible_count,
        "baselineTopSources": baseline_top_sources,
        "shadowTopSources": shadow_top_sources,
        "overlapCount": overlap_count,
        "overlapRatio": overlap_ratio,
        "jaccard": jaccard,
        "matchedRankPairs": matched_rank_pairs,
        "meanAbsRankDelta": mean_abs_rank_delta,
        "top1Match": top1_match,
    })
}

