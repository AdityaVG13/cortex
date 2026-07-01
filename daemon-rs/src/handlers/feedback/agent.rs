// SPDX-License-Identifier: MIT
//! Relevance Feedback Loop — learns which recalled results are actually useful.
//!
//! Signal sources:
//!   - Explicit: cortex_unfold() calls → positive signal for unfolded sources
//!   - Explicit: POST /feedback → caller reports useful/not-useful
//!
//! Feedback is stored in `recall_feedback` with the query embedding so future
//! recalls for similar queries can rerank based on what worked before.
//!
//! Reranking: boost = sum(signal * decay) for matching result_source,
//! where decay = exp(-age_days / 30). Capped at [-0.2, +0.3].

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::handlers::{ensure_auth_rated, ensure_auth_with_caller_rated, json_error, json_response};
use crate::embeddings;
use crate::state::RuntimeState;
use std::collections::HashMap;

/// Default lookback window for agent outcome telemetry stats.
const AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS: i64 = 30;
/// Default row cap for agent outcome telemetry stats.
const AGENT_FEEDBACK_DEFAULT_LIMIT: usize = 400;
/// Recency half-life used for reliability weighting.
const AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS: f64 = 21.0;

// ─── Agent outcome telemetry (/agent-feedback*) ─────────────────────────────

#[derive(Deserialize)]
pub struct AgentFeedbackRecordRequest {
    pub agent: Option<String>,
    #[serde(alias = "taskClass")]
    pub task_class: Option<String>,
    pub outcome: Option<String>,
    #[serde(alias = "outcomeScore")]
    pub outcome_score: Option<f64>,
    #[serde(alias = "qualityScore")]
    pub quality_score: Option<f64>,
    #[serde(alias = "latencyMs")]
    pub latency_ms: Option<i64>,
    pub retries: Option<i64>,
    #[serde(alias = "tokensUsed")]
    pub tokens_used: Option<i64>,
    #[serde(alias = "memorySources")]
    pub memory_sources: Option<Vec<String>>,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct AgentFeedbackStatsQuery {
    #[serde(alias = "horizonDays")]
    pub horizon_days: Option<i64>,
    pub limit: Option<usize>,
    #[serde(alias = "taskClass")]
    pub task_class: Option<String>,
    pub agent: Option<String>,
}

#[derive(Default, Clone)]
struct FeedbackAggregate {
    count: i64,
    weighted_sum: f64,
    weight_total: f64,
    success: i64,
    partial: i64,
    failure: i64,
    latency_total: i64,
    latency_count: i64,
    retries_total: i64,
    retries_count: i64,
    tokens_total: i64,
    tokens_count: i64,
}

impl FeedbackAggregate {
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        outcome: &str,
        outcome_score: f64,
        quality_score: f64,
        age_days: f64,
        latency_ms: Option<i64>,
        retries: Option<i64>,
        tokens_used: Option<i64>,
    ) {
        self.count += 1;
        match outcome {
            "success" => self.success += 1,
            "partial" => self.partial += 1,
            _ => self.failure += 1,
        }

        let decay_lambda = (2.0f64).ln() / AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS;
        let weight = (-decay_lambda * age_days.max(0.0)).exp();
        let blended = (outcome_score * 0.6 + quality_score * 0.4).clamp(0.0, 1.0);
        self.weighted_sum += blended * weight;
        self.weight_total += weight;

        if let Some(value) = latency_ms {
            self.latency_total += value.max(0);
            self.latency_count += 1;
        }
        if let Some(value) = retries {
            self.retries_total += value.max(0);
            self.retries_count += 1;
        }
        if let Some(value) = tokens_used {
            self.tokens_total += value.max(0);
            self.tokens_count += 1;
        }
    }

    fn reliability(&self) -> f64 {
        if self.weight_total > 0.0 {
            (self.weighted_sum / self.weight_total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

fn normalize_outcome(raw: Option<&str>) -> Option<&'static str> {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "success" | "ok" | "pass" => Some("success"),
        "partial" | "mixed" | "degraded" => Some("partial"),
        "failure" | "fail" | "error" => Some("failure"),
        _ => None,
    }
}

fn default_outcome_score(outcome: &str) -> f64 {
    match outcome {
        "success" => 1.0,
        "partial" => 0.5,
        _ => 0.0,
    }
}

fn normalize_task_class(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("general")
        .to_ascii_lowercase()
}

fn normalize_agent(value: Option<&str>, fallback_agent: &str) -> String {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or(fallback_agent)
        .to_string()
}

pub(crate) fn normalize_horizon_days(value: Option<i64>) -> i64 {
    value
        .unwrap_or(AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS)
        .clamp(1, 180)
}

pub(crate) fn normalize_limit(value: Option<usize>) -> usize {
    value
        .unwrap_or(AGENT_FEEDBACK_DEFAULT_LIMIT)
        .clamp(10, 2_000)
}

fn arg_value_string(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn arg_value_f64(args: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_f64()))
}

fn arg_value_i64(args: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| args.get(*key).and_then(|value| value.as_i64()))
}

fn arg_value_string_array(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            args.get(*key).and_then(|value| {
                value.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::trim))
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
            })
        })
        .unwrap_or_default()
}

pub fn record_agent_feedback_from_value(
    conn: &Connection,
    owner_id: i64,
    args: &Value,
    fallback_agent: &str,
) -> Result<Value, String> {
    let outcome = normalize_outcome(
        arg_value_string(args, &["outcome"])
            .as_deref()
            .or_else(|| args.get("outcome").and_then(|value| value.as_str())),
    )
    .ok_or_else(|| "Missing or invalid outcome (expected success|partial|failure)".to_string())?;
    let agent = normalize_agent(
        arg_value_string(args, &["agent", "source_agent", "sourceAgent"]).as_deref(),
        fallback_agent,
    );
    let task_class =
        normalize_task_class(arg_value_string(args, &["task_class", "taskClass"]).as_deref());
    let outcome_score = arg_value_f64(args, &["outcome_score", "outcomeScore"])
        .unwrap_or_else(|| default_outcome_score(outcome))
        .clamp(0.0, 1.0);
    let quality_score = arg_value_f64(args, &["quality_score", "qualityScore"])
        .unwrap_or(0.7)
        .clamp(0.0, 1.0);
    let latency_ms = arg_value_i64(args, &["latency_ms", "latencyMs"]).map(|value| value.max(0));
    let retries = arg_value_i64(args, &["retries"]).map(|value| value.max(0));
    let tokens_used = arg_value_i64(args, &["tokens_used", "tokensUsed"]).map(|value| value.max(0));
    let memory_sources = arg_value_string_array(args, &["memory_sources", "memorySources"]);
    let notes = arg_value_string(args, &["notes"]);
    let memory_sources_json =
        serde_json::to_string(&memory_sources).map_err(|err| err.to_string())?;

    conn.execute(
        "INSERT INTO agent_feedback (
            owner_id, agent, task_class, outcome, outcome_score, quality_score,
            latency_ms, retries, tokens_used, memory_sources_json, notes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            owner_id,
            agent,
            task_class,
            outcome,
            outcome_score,
            quality_score,
            latency_ms,
            retries,
            tokens_used,
            memory_sources_json,
            notes
        ],
    )
    .map_err(|err| err.to_string())?;

    Ok(json!({
        "stored": true,
        "ownerId": owner_id,
        "agent": agent,
        "taskClass": task_class,
        "outcome": outcome,
        "outcomeScore": outcome_score,
        "qualityScore": quality_score,
        "memorySources": memory_sources,
    }))
}

fn aggregate_summary_json(name: &str, agg: &FeedbackAggregate) -> Value {
    json!({
        "name": name,
        "count": agg.count,
        "reliability": agg.reliability(),
        "success": agg.success,
        "partial": agg.partial,
        "failure": agg.failure,
        "avgLatencyMs": if agg.latency_count > 0 { Some(agg.latency_total as f64 / agg.latency_count as f64) } else { None },
        "avgRetries": if agg.retries_count > 0 { Some(agg.retries_total as f64 / agg.retries_count as f64) } else { None },
        "avgTokensUsed": if agg.tokens_count > 0 { Some(agg.tokens_total as f64 / agg.tokens_count as f64) } else { None },
    })
}

pub fn build_agent_feedback_stats_payload(
    conn: &Connection,
    owner_id: i64,
    horizon_days: i64,
    limit: usize,
    task_class_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Value, String> {
    let horizon_days = normalize_horizon_days(Some(horizon_days));
    let limit = normalize_limit(Some(limit));
    let task_filter = task_class_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let agent_filter = agent_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let mut stmt = conn
        .prepare(
            "SELECT agent, task_class, outcome, outcome_score, quality_score,
                    latency_ms, retries, tokens_used, memory_sources_json,
                    julianday('now') - julianday(created_at) AS age_days
             FROM agent_feedback
             WHERE owner_id = ?1
               AND julianday('now') - julianday(created_at) <= ?2
               AND (?3 IS NULL OR task_class = ?3)
               AND (?4 IS NULL OR agent = ?4)
             ORDER BY datetime(created_at) DESC, id DESC
             LIMIT ?5",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(
            params![
                owner_id,
                horizon_days,
                task_filter,
                agent_filter,
                limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, f64>(9)?,
                ))
            },
        )
        .map_err(|err| err.to_string())?;

    let mut overall = FeedbackAggregate::default();
    let mut by_agent: HashMap<String, FeedbackAggregate> = HashMap::new();
    let mut by_task: HashMap<String, FeedbackAggregate> = HashMap::new();
    let mut source_counts: HashMap<String, i64> = HashMap::new();
    let mut rows_with_sources = 0i64;

    for row in rows.flatten() {
        let (
            agent,
            task_class,
            outcome,
            outcome_score,
            quality_score,
            latency_ms,
            retries,
            tokens_used,
            memory_sources_json,
            age_days,
        ) = row;

        overall.observe(
            &outcome,
            outcome_score,
            quality_score,
            age_days,
            latency_ms,
            retries,
            tokens_used,
        );
        by_agent.entry(agent).or_default().observe(
            &outcome,
            outcome_score,
            quality_score,
            age_days,
            latency_ms,
            retries,
            tokens_used,
        );
        by_task.entry(task_class).or_default().observe(
            &outcome,
            outcome_score,
            quality_score,
            age_days,
            latency_ms,
            retries,
            tokens_used,
        );

        let parsed_sources = memory_sources_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
            .unwrap_or_default();
        if !parsed_sources.is_empty() {
            rows_with_sources += 1;
            for source in parsed_sources {
                *source_counts.entry(source).or_insert(0) += 1;
            }
        }
    }

    let mut by_agent_vec: Vec<Value> = by_agent
        .iter()
        .map(|(name, agg)| aggregate_summary_json(name, agg))
        .collect();
    by_agent_vec.sort_by(|left, right| {
        let left_rel = left
            .get("reliability")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let right_rel = right
            .get("reliability")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        right_rel
            .partial_cmp(&left_rel)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut by_task_vec: Vec<Value> = by_task
        .iter()
        .map(|(name, agg)| aggregate_summary_json(name, agg))
        .collect();
    by_task_vec.sort_by(|left, right| {
        let left_count = left
            .get("count")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        let right_count = right
            .get("count")
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        right_count.cmp(&left_count)
    });

    let mut top_sources: Vec<(String, i64)> = source_counts.into_iter().collect();
    top_sources.sort_by(|left, right| right.1.cmp(&left.1));
    let top_sources: Vec<Value> = top_sources
        .into_iter()
        .take(10)
        .map(|(source, hits)| json!({ "source": source, "hits": hits }))
        .collect();

    let reliability = overall.reliability();
    let recommendation = if overall.count == 0 {
        "No agent feedback telemetry recorded yet."
    } else if reliability < 0.65 {
        "Reliability is below target; tighten task decomposition and collect richer memory_sources."
    } else if reliability < 0.8 {
        "Reliability is stable but improvable; prioritize retries and conflict resolution on partial outcomes."
    } else {
        "Reliability is strong; continue reinforcing high-quality runs and memory-source coverage."
    };

    Ok(json!({
        "ownerId": owner_id,
        "horizonDays": horizon_days,
        "limit": limit,
        "sampled": overall.count,
        "reliability": reliability,
        "outcomes": {
            "success": overall.success,
            "partial": overall.partial,
            "failure": overall.failure,
        },
        "averages": {
            "latencyMs": if overall.latency_count > 0 { Some(overall.latency_total as f64 / overall.latency_count as f64) } else { None },
            "retries": if overall.retries_count > 0 { Some(overall.retries_total as f64 / overall.retries_count as f64) } else { None },
            "tokensUsed": if overall.tokens_count > 0 { Some(overall.tokens_total as f64 / overall.tokens_count as f64) } else { None },
        },
        "memorySourceCoverage": {
            "rowsWithSources": rows_with_sources,
            "ratio": if overall.count > 0 { rows_with_sources as f64 / overall.count as f64 } else { 0.0 },
        },
        "byAgent": by_agent_vec,
        "byTaskClass": by_task_vec,
        "topMemorySources": top_sources,
        "recommendation": recommendation,
    }))
}

pub fn recommend_recall_k(
    conn: &Connection,
    owner_id: i64,
    agent: &str,
    task_class: Option<&str>,
    base_k: usize,
) -> Result<Option<Value>, String> {
    let task_class = normalize_task_class(task_class).to_string();
    let mut stmt = conn
        .prepare(
            "SELECT outcome, quality_score
             FROM agent_feedback
             WHERE owner_id = ?1
               AND agent = ?2
               AND task_class = ?3
               AND julianday('now') - julianday(created_at) <= 30
             ORDER BY datetime(created_at) DESC, id DESC
             LIMIT 40",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(params![owner_id, agent, task_class], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|err| err.to_string())?;

    let mut success = 0usize;
    let mut partial = 0usize;
    let mut failure = 0usize;
    let mut quality_total = 0.0f64;
    let mut count = 0usize;
    for (outcome, quality) in rows.flatten() {
        count += 1;
        quality_total += quality.clamp(0.0, 1.0);
        match outcome.as_str() {
            "success" => success += 1,
            "partial" => partial += 1,
            _ => failure += 1,
        }
    }

    if count < 8 {
        return Ok(None);
    }

    let failure_rate = failure as f64 / count as f64;
    let partial_rate = partial as f64 / count as f64;
    let success_rate = success as f64 / count as f64;
    let avg_quality = quality_total / count as f64;

    let mut recommended_k = base_k;
    let reason = if failure_rate >= 0.3 || partial_rate >= 0.45 {
        recommended_k = (base_k + 4).min(24);
        "raise_depth_for_recovery"
    } else if success_rate >= 0.75 && avg_quality >= 0.82 {
        recommended_k = base_k.saturating_sub(2).max(6);
        "reduce_depth_for_efficiency"
    } else {
        "keep_depth_stable"
    };

    Ok(Some(json!({
        "agent": agent,
        "taskClass": task_class,
        "samples": count,
        "baseK": base_k,
        "recommendedK": recommended_k,
        "reason": reason,
        "successRate": success_rate,
        "partialRate": partial_rate,
        "failureRate": failure_rate,
        "avgQuality": avg_quality,
    })))
}

