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

use super::{ensure_auth_rated, ensure_auth_with_caller_rated, json_error, json_response};
use crate::embeddings;
use crate::state::RuntimeState;
use std::collections::HashMap;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Max boost from feedback (prevents runaway amplification).
const MAX_BOOST: f64 = 0.3;
/// Max penalty from negative feedback.
const MIN_BOOST: f64 = -0.2;
/// Half-life for feedback signal decay (days).
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;
/// Minimum positive feedback signals in last 14 days to grant aging immunity.
pub const IMMUNITY_THRESHOLD: i64 = 5;
/// Window for aging immunity check (days).
pub const IMMUNITY_WINDOW_DAYS: i64 = 14;
/// Default lookback window for agent outcome telemetry stats.
const AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS: i64 = 30;
/// Default row cap for agent outcome telemetry stats.
const AGENT_FEEDBACK_DEFAULT_LIMIT: usize = 400;
/// Recency half-life used for reliability weighting.
const AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS: f64 = 21.0;

// ─── POST /feedback ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub query: Option<String>,
    pub sources: Vec<String>,
    pub signal: Option<f64>,
    pub agent: Option<String>,
}

pub async fn handle_feedback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FeedbackRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    if body.sources.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "sources array is empty");
    }

    let signal = body.signal.unwrap_or(1.0).clamp(-1.0, 1.0);
    let agent = body.agent.as_deref().unwrap_or("http");
    let query_text = body.query.as_deref().unwrap_or("");

    let query_embedding = state
        .embedding_engine
        .as_ref()
        .and_then(|e| e.embed_query(query_text))
        .map(|v| embeddings::vector_to_blob(&v));

    let conn = state.db.lock().await;
    let mut stored = 0usize;
    for source in &body.sources {
        let (result_type, result_id) = parse_source(source);
        match conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                query_text,
                query_embedding,
                source,
                result_type,
                result_id,
                signal,
                agent,
            ],
        ) {
            Ok(_) => stored += 1,
            Err(e) => eprintln!("[feedback] Failed to store for {source}: {e}"),
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "stored": stored,
            "signal": signal,
            "sources": body.sources,
        }),
    )
}

// ─── Feedback recording (called internally by unfold) ───────────────────────

/// Record positive feedback for sources that were unfolded after a recall.
/// Called from the MCP unfold handler — no HTTP round-trip needed.
pub fn record_unfold_feedback(
    conn: &Connection,
    sources: &[String],
    agent: &str,
    engine: Option<&embeddings::EmbeddingEngine>,
    query_hint: Option<&str>,
) {
    let query_text = query_hint.unwrap_or("");
    let query_blob = engine
        .and_then(|e| e.embed_query(query_text))
        .map(|v| embeddings::vector_to_blob(&v));

    for source in sources {
        let (result_type, result_id) = parse_source(source);
        let _ = conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6)",
            params![query_text, query_blob, source, result_type, result_id, agent],
        );
    }
}

// ─── Reranking: compute boost for a result source ───────────────────────────

/// Compute a relevance boost for a given result source based on historical
/// feedback. Returns a value in [MIN_BOOST, MAX_BOOST].
///
/// Algorithm:
///   boost = sum(signal_i * exp(-age_days_i / HALF_LIFE)) for all feedback rows
///   clamped to [MIN_BOOST, MAX_BOOST]
///
/// This is O(feedback_rows_for_source) which stays small because:
/// - Each unfold generates ~2-3 rows
/// - Old feedback decays naturally
/// - We only scan rows for the specific source
#[allow(dead_code)]
pub fn compute_boost(conn: &Connection, result_source: &str) -> f64 {
    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;

    let boost: f64 = conn
        .prepare(
            "SELECT signal, julianday('now') - julianday(created_at) AS age_days \
             FROM recall_feedback WHERE result_source = ?1",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![result_source], |row| {
                let signal: f64 = row.get(0)?;
                let age_days: f64 = row.get::<_, f64>(1)?.max(0.0);
                Ok(signal * (-decay_lambda * age_days).exp())
            })?;
            let mut total = 0.0f64;
            for v in rows.flatten() {
                total += v;
            }
            Ok(total)
        })
        .unwrap_or(0.0);

    boost.clamp(MIN_BOOST, MAX_BOOST)
}

/// Batch compute boosts for multiple sources at once (avoids N queries).
/// Returns a map of source → boost value.
pub fn compute_boosts(
    conn: &Connection,
    sources: &[String],
    query_vector: Option<&[f32]>,
) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    if sources.is_empty() {
        return boosts;
    }

    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;

    // Single query: fetch all feedback for any of the requested sources
    let placeholders = sources
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT result_source, signal, query_embedding, julianday('now') - julianday(created_at) AS age_days \
         FROM recall_feedback WHERE result_source IN ({placeholders})"
    );

    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::types::ToSql> = sources
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
            let source: String = row.get(0)?;
            let signal: f64 = row.get(1)?;
            let query_blob: Option<Vec<u8>> = row.get(2)?;
            let age_days: f64 = row.get::<_, f64>(3)?.max(0.0);
            let query_weight = query_similarity_weight(query_vector, query_blob.as_deref());
            Ok((
                source,
                signal * query_weight * (-decay_lambda * age_days).exp(),
            ))
        }) {
            for row in rows.flatten() {
                *boosts.entry(row.0).or_insert(0.0) += row.1;
            }
        }
    }

    // Clamp all values
    for v in boosts.values_mut() {
        *v = v.clamp(MIN_BOOST, MAX_BOOST);
    }

    boosts
}

fn query_similarity_weight(current_query: Option<&[f32]>, stored_blob: Option<&[u8]>) -> f64 {
    let Some(current_query) = current_query else {
        return 1.0;
    };
    let Some(stored_blob) = stored_blob else {
        return 0.6;
    };
    let stored_vec = embeddings::blob_to_vector(stored_blob);
    if stored_vec.is_empty() {
        return 0.6;
    }
    let sim = embeddings::cosine_similarity(current_query, &stored_vec).clamp(0.0, 1.0);
    // Keep a non-zero floor so sparse historic signal still contributes lightly.
    0.2 + (sim as f64 * 0.8)
}

/// Check if a source has enough recent positive feedback to be immune from aging.
pub fn has_retrieval_immunity(conn: &Connection, source: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM recall_feedback \
         WHERE result_source = ?1 AND signal > 0 \
         AND julianday('now') - julianday(created_at) <= ?2",
        params![source, IMMUNITY_WINDOW_DAYS],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        >= IMMUNITY_THRESHOLD
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Parse a source string into (type, optional ID).
/// "decision::42" → ("decision", Some(42))
/// "memory::my_project.md" → ("memory", None)
fn parse_source(source: &str) -> (String, Option<i64>) {
    if let Some(rest) = source.strip_prefix("decision::") {
        let id = rest.parse::<i64>().ok();
        ("decision".to_string(), id)
    } else if let Some(_rest) = source.strip_prefix("memory::") {
        ("memory".to_string(), None)
    } else {
        ("unknown".to_string(), None)
    }
}

// ─── GET /feedback/stats ────────────────────────────────────────────────────

pub async fn handle_feedback_stats(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM recall_feedback", [], |row| row.get(0))
        .unwrap_or(0);
    let positive: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recall_feedback WHERE signal > 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let negative: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recall_feedback WHERE signal < 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let unique_sources: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT result_source) FROM recall_feedback",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Top boosted sources
    let top: Vec<Value> = conn
        .prepare(
            "SELECT result_source, SUM(signal) as total_signal, COUNT(*) as hits \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) <= 30 \
             GROUP BY result_source ORDER BY total_signal DESC LIMIT 10",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| {
                Ok(json!({
                    "source": row.get::<_, String>(0)?,
                    "totalSignal": row.get::<_, f64>(1)?,
                    "hits": row.get::<_, i64>(2)?,
                }))
            })?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        json!({
            "total": total,
            "positive": positive,
            "negative": negative,
            "uniqueSources": unique_sources,
            "topBoosted": top,
        }),
    )
}

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

fn normalize_horizon_days(value: Option<i64>) -> i64 {
    value
        .unwrap_or(AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS)
        .clamp(1, 180)
}

fn normalize_limit(value: Option<usize>) -> usize {
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

pub async fn handle_agent_feedback_record(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<AgentFeedbackRecordRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
        );
    }
    let owner_id = if state.team_mode {
        caller_id.unwrap_or_default()
    } else {
        0
    };

    let args = json!({
        "agent": body.agent,
        "task_class": body.task_class,
        "outcome": body.outcome,
        "outcome_score": body.outcome_score,
        "quality_score": body.quality_score,
        "latency_ms": body.latency_ms,
        "retries": body.retries,
        "tokens_used": body.tokens_used,
        "memory_sources": body.memory_sources.unwrap_or_default(),
        "notes": body.notes,
    });

    let conn = state.db.lock().await;
    match record_agent_feedback_from_value(&conn, owner_id, &args, "http") {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_error(StatusCode::BAD_REQUEST, &err),
    }
}

pub async fn handle_agent_feedback_stats(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<AgentFeedbackStatsQuery>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
        );
    }
    let owner_id = if state.team_mode {
        caller_id.unwrap_or_default()
    } else {
        0
    };

    let horizon_days = normalize_horizon_days(query.horizon_days);
    let limit = normalize_limit(query.limit);
    let conn = state.db.lock().await;
    match build_agent_feedback_stats_payload(
        &conn,
        owner_id,
        horizon_days,
        limit,
        query.task_class.as_deref(),
        query.agent.as_deref(),
    ) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_error(StatusCode::BAD_REQUEST, &err),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_compute_boost_no_feedback() {
        let conn = setup_test_db();
        let boost = compute_boost(&conn, "memory::nonexistent");
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn test_compute_boost_positive() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::foo', 'memory', 1.0, 'test')",
            [],
        )
        .unwrap();
        let boost = compute_boost(&conn, "memory::foo");
        assert!(boost > 0.0, "Positive signal should produce positive boost");
        assert!(boost <= MAX_BOOST, "Boost should be capped at MAX_BOOST");
    }

    #[test]
    fn test_compute_boost_negative() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::bar', 'memory', -1.0, 'test')",
            [],
        )
        .unwrap();
        let boost = compute_boost(&conn, "memory::bar");
        assert!(boost < 0.0, "Negative signal should produce negative boost");
        assert!(boost >= MIN_BOOST, "Boost should be capped at MIN_BOOST");
    }

    #[test]
    fn test_compute_boosts_batch() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::a', 'memory', 1.0, 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::b', 'memory', -0.5, 'test')",
            [],
        )
        .unwrap();

        let sources = vec![
            "memory::a".to_string(),
            "memory::b".to_string(),
            "memory::c".to_string(),
        ];
        let boosts = compute_boosts(&conn, &sources, None);
        assert!(boosts["memory::a"] > 0.0);
        assert!(boosts["memory::b"] < 0.0);
        assert!(!boosts.contains_key("memory::c"), "No feedback = no entry");
    }

    #[test]
    fn test_compute_boosts_prefers_similar_query_embeddings() {
        let conn = setup_test_db();
        let source = "memory::ranked";
        let similar = vec![1.0_f32, 0.0, 0.0];
        let dissimilar = vec![0.0_f32, 1.0, 0.0];
        let similar_blob = embeddings::vector_to_blob(&similar);
        let dissimilar_blob = embeddings::vector_to_blob(&dissimilar);

        conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, signal, agent) \
             VALUES ('similar', ?1, ?2, 'memory', 0.1, 'test')",
            params![similar_blob, source],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, signal, agent) \
             VALUES ('dissimilar', ?1, ?2, 'memory', 0.1, 'test')",
            params![dissimilar_blob, source],
        )
        .unwrap();

        let scoped = compute_boosts(&conn, &[source.to_string()], Some(&similar));
        let baseline = compute_boosts(&conn, &[source.to_string()], None);

        assert!(
            scoped[source] < baseline[source],
            "dissimilar feedback should be down-weighted for current query"
        );
        assert!(
            scoped[source] > 0.0,
            "similar feedback should still contribute positive boost"
        );
    }

    #[test]
    fn test_parse_source() {
        assert_eq!(
            parse_source("decision::42"),
            ("decision".to_string(), Some(42))
        );
        assert_eq!(parse_source("memory::foo.md"), ("memory".to_string(), None));
        assert_eq!(parse_source("other"), ("unknown".to_string(), None));
    }

    #[test]
    fn test_has_retrieval_immunity_below_threshold() {
        let conn = setup_test_db();
        // Insert fewer than IMMUNITY_THRESHOLD signals
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
                 VALUES ('q', 'memory::x', 'memory', 1.0, 'test')",
                [],
            ).unwrap();
        }
        assert!(!has_retrieval_immunity(&conn, "memory::x"));
    }

    #[test]
    fn test_has_retrieval_immunity_above_threshold() {
        let conn = setup_test_db();
        for _ in 0..IMMUNITY_THRESHOLD {
            conn.execute(
                "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
                 VALUES ('q', 'memory::y', 'memory', 1.0, 'test')",
                [],
            ).unwrap();
        }
        assert!(has_retrieval_immunity(&conn, "memory::y"));
    }

    #[test]
    fn test_record_agent_feedback_from_value_rejects_invalid_outcome() {
        let conn = setup_test_db();
        let payload = json!({
            "agent": "codex",
            "taskClass": "recall",
            "outcome": "unknown"
        });
        let err = record_agent_feedback_from_value(&conn, 0, &payload, "mcp").unwrap_err();
        assert!(err.contains("invalid outcome"));
    }

    #[test]
    fn test_record_agent_feedback_from_value_persists_entry() {
        let conn = setup_test_db();
        let payload = json!({
            "agent": "codex",
            "taskClass": "recall",
            "outcome": "success",
            "qualityScore": 0.9,
            "latencyMs": 120,
            "tokensUsed": 280,
            "memorySources": ["decision::42"]
        });
        let result = record_agent_feedback_from_value(&conn, 0, &payload, "mcp").unwrap();
        assert_eq!(result["stored"].as_bool(), Some(true));
        assert_eq!(result["agent"].as_str(), Some("codex"));

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_feedback WHERE owner_id = 0 AND agent = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_build_agent_feedback_stats_payload_aggregates_by_owner() {
        let conn = setup_test_db();
        record_agent_feedback_from_value(
            &conn,
            0,
            &json!({
                "agent": "codex",
                "taskClass": "recall",
                "outcome": "success",
                "qualityScore": 0.8,
                "memorySources": ["decision::1"]
            }),
            "mcp",
        )
        .unwrap();
        record_agent_feedback_from_value(
            &conn,
            9,
            &json!({
                "agent": "claude",
                "taskClass": "store",
                "outcome": "failure",
                "qualityScore": 0.2
            }),
            "mcp",
        )
        .unwrap();

        let stats = build_agent_feedback_stats_payload(&conn, 0, 30, 100, None, None).unwrap();
        assert_eq!(stats["sampled"].as_i64(), Some(1));
        assert_eq!(stats["outcomes"]["success"].as_i64(), Some(1));
        assert_eq!(stats["outcomes"]["failure"].as_i64(), Some(0));
        assert_eq!(
            stats["byAgent"][0]["name"].as_str(),
            Some("codex"),
            "owner-scoped stats should exclude other owners"
        );
    }

    #[test]
    fn test_recommend_recall_k_increases_depth_for_struggling_task_class() {
        let conn = setup_test_db();
        for _ in 0..10 {
            record_agent_feedback_from_value(
                &conn,
                0,
                &json!({
                    "agent": "codex",
                    "taskClass": "debug",
                    "outcome": "failure",
                    "qualityScore": 0.25
                }),
                "mcp",
            )
            .unwrap();
        }
        let policy = recommend_recall_k(&conn, 0, "codex", Some("debug"), 10)
            .unwrap()
            .expect("policy expected");
        assert_eq!(policy["recommendedK"].as_u64(), Some(14));
        assert_eq!(policy["reason"].as_str(), Some("raise_depth_for_recovery"));
    }

    #[test]
    fn test_recommend_recall_k_reduces_depth_for_stable_high_quality_runs() {
        let conn = setup_test_db();
        for _ in 0..12 {
            record_agent_feedback_from_value(
                &conn,
                0,
                &json!({
                    "agent": "codex",
                    "taskClass": "refactor",
                    "outcome": "success",
                    "qualityScore": 0.95
                }),
                "mcp",
            )
            .unwrap();
        }
        let policy = recommend_recall_k(&conn, 0, "codex", Some("refactor"), 12)
            .unwrap()
            .expect("policy expected");
        assert_eq!(policy["recommendedK"].as_u64(), Some(10));
        assert_eq!(
            policy["reason"].as_str(),
            Some("reduce_depth_for_efficiency")
        );
    }
}
