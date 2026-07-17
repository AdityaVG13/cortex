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

pub(crate) fn rerank_candidate_text(item: &RecallItem) -> String {
    let text = if item.excerpt.trim().is_empty() {
        item.source.clone()
    } else {
        format!("{} {}", item.source, item.excerpt)
    };
    truncate_chars(&text, 1800)
}

pub(crate) fn build_rerank_candidates(results: &[RecallItem], top_n: usize) -> Vec<RerankCandidate> {
    results
        .iter()
        .take(top_n.max(1))
        .map(|item| RerankCandidate {
            id: item.source.clone(),
            text: rerank_candidate_text(item),
            base_score: item.relevance,
        })
        .collect()
}

pub(crate) fn remap_fused_score_to_relevance(fused_score: f64, window: &[RecallItem]) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for relevance in window.iter().map(|item| item.relevance) {
        if relevance.is_finite() {
            min = min.min(relevance);
            max = max.max(relevance);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return round4(fused_score.clamp(0.0, 1.0));
    }
    let span = (max - min).max(0.01);
    round4(min + (span * fused_score.clamp(0.0, 1.0)))
}

pub(crate) fn apply_primary_rerank(results: Vec<RecallItem>, reranked: &[RerankedScore]) -> Vec<RecallItem> {
    if reranked.is_empty() {
        return results;
    }
    let window_len = reranked.len().min(results.len());
    let window = &results[..window_len];
    let mut by_source: HashMap<String, RecallItem> = results
        .iter()
        .take(window_len)
        .cloned()
        .map(|item| (item.source.clone(), item))
        .collect();
    let mut output = Vec::with_capacity(results.len());
    for score in reranked {
        if let Some(mut item) = by_source.remove(&score.id) {
            item.relevance = remap_fused_score_to_relevance(score.fused_score, window);
            if !item.method.contains("rerank") {
                item.method = format!("{}+rerank", item.method);
            }
            output.push(item);
        }
    }
    for item in results.iter().take(window_len) {
        if let Some(item) = by_source.remove(&item.source) {
            output.push(item);
        }
    }
    output.extend(results.into_iter().skip(window_len));
    output
}

pub(crate) fn rerank_scores_json(reranked: &[RerankedScore]) -> Vec<Value> {
    reranked
        .iter()
        .take(12)
        .enumerate()
        .map(|(idx, score)| {
            json!({
                "rank": idx + 1,
                "source": score.id,
                "baseScore": round4(score.base_score),
                "rerankScore": round4(score.rerank_score),
                "fusedScore": round4(score.fused_score),
            })
        })
        .collect()
}

pub(crate) fn maybe_apply_rerank(
    state: &RuntimeState,
    query_text: &str,
    results: Vec<RecallItem>,
    budget: usize,
) -> (Vec<RecallItem>, Value) {
    let config = &state.rerank_config;
    if budget == 0 {
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "headlines_mode",
                "mode": config.mode.as_str(),
            }),
        );
    }
    if !config.is_active() {
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "mode_off",
                "mode": config.mode.as_str(),
            }),
        );
    }
    if results.len() < 2 {
        let candidate_count = results.len();
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "not_enough_candidates",
                "mode": config.mode.as_str(),
                "candidateCount": candidate_count,
            }),
        );
    }
    let Some(reranker) = state.reranker.as_ref() else {
        return (
            results,
            json!({
                "status": "unavailable",
                "reason": "model_not_loaded",
                "mode": config.mode.as_str(),
                "configuredModel": crate::rerank::selected_reranker_selection().key,
            }),
        );
    };

    let top_n = config.top_n.min(results.len());
    let candidates = build_rerank_candidates(&results, top_n);
    let baseline_top_sources = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    match reranker.rerank(query_text, &candidates, config.fusion_alpha) {
        Ok(reranked) => {
            let reranked_top_sources = reranked
                .iter()
                .map(|score| score.id.clone())
                .collect::<Vec<_>>();
            let telemetry = json!({
                "status": "ok",
                "mode": config.mode.as_str(),
                "applied": config.is_primary(),
                "model": reranker.name(),
                "modelSizeMb": reranker.model_size_mb(),
                "topN": top_n,
                "fusionAlpha": round4(config.fusion_alpha),
                "baselineTopSources": baseline_top_sources,
                "rerankedTopSources": reranked_top_sources,
                "scores": rerank_scores_json(&reranked),
            });
            let results = if config.is_primary() {
                apply_primary_rerank(results, &reranked)
            } else {
                results
            };
            (results, telemetry)
        }
        Err(error) => (
            results,
            json!({
                "status": "error",
                "mode": config.mode.as_str(),
                "applied": false,
                "model": reranker.name(),
                "reason": truncate_chars(&error, 240),
            }),
        ),
    }
}

pub(crate) fn sqlite_vec_trial_sampled(
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    trial_percent: u8,
) -> bool {
    if trial_percent == 0 {
        return false;
    }
    if trial_percent >= 100 {
        return true;
    }

    let mut hasher = DefaultHasher::new();
    query_text.hash(&mut hasher);
    ctx.team_mode.hash(&mut hasher);
    ctx.caller_id.hash(&mut hasher);
    source_prefix.unwrap_or_default().hash(&mut hasher);
    let bucket = (hasher.finish() % 100) as u8;
    bucket < trial_percent
}

pub(crate) fn parse_shadow_sources(shadow_semantic: &Value, field: &str) -> Vec<String> {
    shadow_semantic
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn shadow_guard_failure_reason(shadow_semantic: &Value) -> Option<&'static str> {
    if shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error")
        != "ok"
    {
        return Some("shadow_not_ok");
    }

    let overlap_ratio = shadow_semantic
        .get("overlapRatio")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if overlap_ratio < SQLITE_VEC_TRIAL_MIN_OVERLAP_RATIO {
        return Some("overlap_ratio_below_gate");
    }

    let jaccard = shadow_semantic
        .get("jaccard")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if jaccard < SQLITE_VEC_TRIAL_MIN_JACCARD {
        return Some("jaccard_below_gate");
    }

    let mean_abs_rank_delta = shadow_semantic
        .get("meanAbsRankDelta")
        .and_then(Value::as_f64)
        .unwrap_or(f64::INFINITY);
    if mean_abs_rank_delta > SQLITE_VEC_TRIAL_MAX_MEAN_ABS_RANK_DELTA {
        return Some("rank_delta_above_gate");
    }

    if SQLITE_VEC_TRIAL_TOP1_MATCH_REQUIRED {
        let top1_match = shadow_semantic
            .get("top1Match")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !top1_match {
            return Some("top1_match_required");
        }
    }

    None
}

pub(crate) fn sqlite_vec_source_fallback_candidate(
    conn: &Connection,
    source: &str,
    query_text: &str,
    fallback_relevance: f64,
) -> Option<SemanticCandidate> {
    let build_candidate = |excerpt_text: String,
                           score: Option<f64>,
                           trust_score: Option<f64>,
                           last_accessed: Option<String>,
                           created_at: Option<String>| {
        let ts_source = last_accessed
            .as_deref()
            .or(created_at.as_deref())
            .unwrap_or_default();
        SemanticCandidate {
            source: source.to_string(),
            excerpt: query_focused_excerpt(&excerpt_text, query_text, 280),
            relevance: fallback_relevance,
            importance: blend_importance(score, trust_score),
            ts: parse_timestamp_ms(ts_source),
        }
    };

    let memory_by_id = source
        .strip_prefix("memory::")
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|id| {
            conn.query_row(
                "SELECT text, score, trust_score, last_accessed, created_at
                 FROM memories
                 WHERE id = ?1 AND status = 'active'
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
                 LIMIT 1",
                params![id],
                |row| {
                    Ok(build_candidate(
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
        });
    if let Some(candidate) = memory_by_id {
        return Some(candidate);
    }

    let memory_by_source = conn
        .query_row(
            "SELECT text, score, trust_score, last_accessed, created_at
             FROM memories
             WHERE source = ?1 AND status = 'active'
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok(build_candidate(
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten();
    if let Some(candidate) = memory_by_source {
        return Some(candidate);
    }

    let decision_by_id = source
        .strip_prefix("decision::")
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|id| {
            conn.query_row(
                "SELECT decision, score, trust_score, last_accessed, created_at
                 FROM decisions
                 WHERE id = ?1 AND status = 'active'
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
                 LIMIT 1",
                params![id],
                |row| {
                    Ok(build_candidate(
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
        });
    if let Some(candidate) = decision_by_id {
        return Some(candidate);
    }

    conn.query_row(
        "SELECT decision, score, trust_score, last_accessed, created_at
         FROM decisions
         WHERE context = ?1 AND status = 'active'
         AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
         ORDER BY COALESCE(last_accessed, created_at) DESC
         LIMIT 1",
        params![source],
        |row| {
            Ok(build_candidate(
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_apply_sqlite_vec_trial(
    conn: &Connection,
    query_text: &str,
    query_vector: Option<&[f32]>,
    semantic_candidates: Vec<SemanticCandidate>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    top_k: usize,
    canary: Option<&SqliteVecCanaryConfig>,
) -> (Vec<SemanticCandidate>, Value) {
    let Some(canary) = canary else {
        return (
            semantic_candidates,
            json!({
                "mode": "baseline",
                "reason": "trial_not_configured",
                "sampled": false,
                "trialPercent": 0,
                "routeMode": "baseline"
            }),
        );
    };
    let effective_route_mode = canary.effective_route_mode();
    let route_mode = effective_route_mode.as_str();
    let active_trial_percent = if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
        100
    } else {
        canary.trial_percent
    };
    let baseline_route = |reason: &str, sampled: bool, trial_percent: u8| {
        json!({
            "mode": "baseline",
            "reason": reason,
            "sampled": sampled,
            "trialPercent": trial_percent,
            "routeMode": route_mode
        })
    };

    if matches!(effective_route_mode, SqliteVecRouteMode::Baseline) {
        let reason = if canary.force_off {
            "trial_force_off"
        } else {
            "route_mode_baseline"
        };
        return (
            semantic_candidates,
            baseline_route(reason, false, active_trial_percent),
        );
    }
    let Some(query_vector) = query_vector else {
        return (
            semantic_candidates,
            baseline_route("query_embedding_unavailable", false, active_trial_percent),
        );
    };
    if semantic_candidates.is_empty() {
        return (
            semantic_candidates,
            baseline_route("no_semantic_candidates", false, active_trial_percent),
        );
    }

    let sampled = if matches!(effective_route_mode, SqliteVecRouteMode::Trial) {
        if canary.trial_percent == 0 {
            return (
                semantic_candidates,
                baseline_route("trial_percent_zero", false, active_trial_percent),
            );
        }
        let sampled =
            sqlite_vec_trial_sampled(query_text, ctx, source_prefix, canary.trial_percent);
        if !sampled {
            return (
                semantic_candidates,
                baseline_route("not_sampled", false, active_trial_percent),
            );
        }
        true
    } else {
        true
    };

    let baseline = ShadowSemanticBaseline {
        candidate_count: semantic_candidates.len(),
        ranked_sources: semantic_candidates
            .iter()
            .take(MAX_SEMANTIC_RRF_CANDIDATES)
            .map(|candidate| candidate.source.clone())
            .collect(),
    };
    let shadow_semantic = build_shadow_semantic_explain(
        conn,
        Some(query_vector),
        query_text,
        ctx,
        source_prefix,
        top_k,
        Some(&baseline),
    );
    if let Some(reason) = shadow_guard_failure_reason(&shadow_semantic) {
        return (
            semantic_candidates,
            baseline_route(reason, sampled, active_trial_percent),
        );
    }

    let shadow_sources = parse_shadow_sources(&shadow_semantic, "shadowTopSources");
    if shadow_sources.is_empty() {
        return (
            semantic_candidates,
            baseline_route("shadow_top_sources_empty", sampled, active_trial_percent),
        );
    }

    let mut by_source: HashMap<String, SemanticCandidate> = semantic_candidates
        .iter()
        .cloned()
        .map(|candidate| (candidate.source.clone(), candidate))
        .collect();
    let mut reordered: Vec<SemanticCandidate> = Vec::new();
    let baseline_max = semantic_candidates
        .first()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SCALE_BASE);
    let baseline_min = semantic_candidates
        .last()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SIM_FLOOR);
    let relevance_span = (baseline_max - baseline_min).abs().max(0.02);
    let rank_denominator = shadow_sources.len().saturating_sub(1).max(1) as f64;
    let fallback_relevance_for_rank = |rank_idx: usize| {
        let rank_weight = 1.0 - (rank_idx as f64 / rank_denominator);
        round4(
            (baseline_min + (relevance_span * rank_weight))
                .clamp(SEMANTIC_SIM_FLOOR, baseline_max.max(SEMANTIC_SIM_FLOOR)),
        )
    };
    for (rank_idx, source) in shadow_sources.iter().enumerate() {
        if let Some(candidate) = by_source.remove(source) {
            reordered.push(candidate);
            continue;
        }
        let fallback_relevance = fallback_relevance_for_rank(rank_idx);
        if let Some(candidate) =
            sqlite_vec_source_fallback_candidate(conn, source, query_text, fallback_relevance)
        {
            reordered.push(candidate);
            continue;
        }
        reordered.push(SemanticCandidate {
            source: source.clone(),
            excerpt: query_focused_excerpt(source, query_text, 160),
            relevance: fallback_relevance,
            importance: 0.5,
            ts: 0,
        });
    }
    for candidate in &semantic_candidates {
        if let Some(remaining) = by_source.remove(&candidate.source) {
            reordered.push(remaining);
        }
    }
    reordered.truncate(semantic_candidates.len());

    (
        reordered,
        json!({
            "mode": if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
                "vec0_primary"
            } else {
                "vec0_trial"
            },
            "reason": if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
                "route_mode_primary"
            } else {
                "guard_passed"
            },
            "sampled": sampled,
            "trialPercent": active_trial_percent,
            "routeMode": route_mode
        }),
    )
}

