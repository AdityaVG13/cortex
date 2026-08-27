mod handlers;
mod recall;

use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

pub use handlers::{handle_agent_feedback_record, handle_agent_feedback_stats};
pub use recall::{compute_boosts, has_retrieval_immunity};
pub use recall::{handle_feedback, handle_feedback_stats};

const AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS: i64 = 30;
const AGENT_FEEDBACK_DEFAULT_LIMIT: usize = 400;
const AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS: f64 = 21.0;

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
struct AgentFeedbackAggregate {
    count: i64,
    weighted_sum: f64,
    weight_total: f64,
    success: i64,
    partial: i64,
    failure: i64,
    latency: (i64, i64),
    retries: (i64, i64),
    tokens: (i64, i64),
}

impl AgentFeedbackAggregate {
    fn observe(
        &mut self, outcome: &str, outcome_score: f64, quality_score: f64, age_days: f64, latency_ms: Option<i64>, retries: Option<i64>,
        tokens_used: Option<i64>,
    ) {
        self.count += 1;
        match outcome {
            "success" => self.success += 1,
            "partial" => self.partial += 1,
            _ => self.failure += 1,
        }
        let weight = (-((2.0f64).ln() / AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS) * age_days.max(0.0)).exp();
        self.weighted_sum += (outcome_score * 0.6 + quality_score * 0.4).clamp(0.0, 1.0) * weight;
        self.weight_total += weight;
        observe_optional(latency_ms, &mut self.latency);
        observe_optional(retries, &mut self.retries);
        observe_optional(tokens_used, &mut self.tokens);
    }

    fn reliability(&self) -> f64 {
        if self.weight_total > 0.0 { (self.weighted_sum / self.weight_total).clamp(0.0, 1.0) } else { 0.0 }
    }
}

fn observe_optional(value: Option<i64>, acc: &mut (i64, i64)) {
    if let Some(value) = value {
        acc.0 += value.max(0);
        acc.1 += 1;
    }
}

fn avg(acc: (i64, i64)) -> Option<f64> {
    (acc.1 > 0).then_some(acc.0 as f64 / acc.1 as f64)
}

fn normalize_outcome(raw: Option<&str>) -> Option<&'static str> {
    match raw.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "success" | "ok" | "pass" => Some("success"),
        "partial" | "mixed" | "degraded" => Some("partial"),
        "failure" | "fail" | "error" => Some("failure"),
        _ => None,
    }
}

fn normalize_task_class(value: Option<&str>) -> String {
    value.map(str::trim).filter(|value| !value.is_empty()).unwrap_or("general").to_ascii_lowercase()
}

fn normalize_agent(value: Option<&str>, fallback_agent: &str) -> String {
    value.map(str::trim).filter(|value| !value.is_empty()).unwrap_or(fallback_agent).to_string()
}

pub(crate) fn normalize_horizon_days(value: Option<i64>) -> i64 {
    value.unwrap_or(AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS).clamp(1, 180)
}

pub(crate) fn normalize_limit(value: Option<usize>) -> usize {
    value.unwrap_or(AGENT_FEEDBACK_DEFAULT_LIMIT).clamp(10, 2_000)
}

fn value_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| args.get(*key)?.as_str()).map(str::trim).filter(|value| !value.is_empty())
}

fn value_f64(args: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| args.get(*key)?.as_f64())
}

fn value_i64(args: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| args.get(*key)?.as_i64())
}

fn value_string_array(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| args.get(*key)?.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string))
        .collect()
}

pub fn record_agent_feedback_from_value(conn: &Connection, owner_id: i64, args: &Value, fallback_agent: &str) -> Result<Value, String> {
    let outcome = normalize_outcome(value_str(args, &["outcome"])).ok_or_else(|| "Missing or invalid outcome (expected success|partial|failure)".to_string())?;
    let agent = normalize_agent(value_str(args, &["agent", "source_agent", "sourceAgent"]), fallback_agent);
    let task_class = normalize_task_class(value_str(args, &["task_class", "taskClass"]));
    let outcome_score = value_f64(args, &["outcome_score", "outcomeScore"])
        .unwrap_or(match outcome { "success" => 1.0, "partial" => 0.5, _ => 0.0 })
        .clamp(0.0, 1.0);
    let quality_score = value_f64(args, &["quality_score", "qualityScore"]).unwrap_or(0.7).clamp(0.0, 1.0);
    let latency_ms = value_i64(args, &["latency_ms", "latencyMs"]).map(|value| value.max(0));
    let retries = value_i64(args, &["retries"]).map(|value| value.max(0));
    let tokens_used = value_i64(args, &["tokens_used", "tokensUsed"]).map(|value| value.max(0));
    let memory_sources = value_string_array(args, &["memory_sources", "memorySources"]);
    let notes = value_str(args, &["notes"]).map(str::to_string);
    let memory_sources_json = serde_json::to_string(&memory_sources).map_err(|err| err.to_string())?;
    conn.execute(
        "INSERT INTO agent_feedback (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json, notes],
    )
    .map_err(|err| err.to_string())?;
    Ok(json!({"stored":true,"ownerId":owner_id,"agent":agent,"taskClass":task_class,"outcome":outcome,
        "outcomeScore":outcome_score,"qualityScore":quality_score,"memorySources":memory_sources}))
}

fn aggregate_json(name: &str, agg: &AgentFeedbackAggregate) -> Value {
    json!({"name":name,"count":agg.count,"reliability":agg.reliability(),"success":agg.success,"partial":agg.partial,"failure":agg.failure,
        "avgLatencyMs":avg(agg.latency),"avgRetries":avg(agg.retries),"avgTokensUsed":avg(agg.tokens)})
}

pub fn build_agent_feedback_stats_payload(
    conn: &Connection, owner_id: i64, horizon_days: i64, limit: usize, task_class_filter: Option<&str>, agent_filter: Option<&str>,
) -> Result<Value, String> {
    let horizon_days = normalize_horizon_days(Some(horizon_days));
    let limit = normalize_limit(Some(limit));
    let task_filter = task_class_filter.map(str::trim).filter(|value| !value.is_empty()).map(str::to_ascii_lowercase);
    let agent_filter = agent_filter.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
    let mut stmt = conn
        .prepare(
            "SELECT agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json,
                    julianday('now') - julianday(created_at)
             FROM agent_feedback
             WHERE owner_id = ?1 AND julianday('now') - julianday(created_at) <= ?2
               AND (?3 IS NULL OR task_class = ?3) AND (?4 IS NULL OR agent = ?4)
             ORDER BY datetime(created_at) DESC, id DESC LIMIT ?5",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![owner_id, horizon_days, task_filter, agent_filter, limit as i64], |row| {
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
        })
        .map_err(|err| err.to_string())?;
    let mut overall = AgentFeedbackAggregate::default();
    let mut by_agent = HashMap::<String, AgentFeedbackAggregate>::new();
    let mut by_task = HashMap::<String, AgentFeedbackAggregate>::new();
    let mut source_counts = HashMap::<String, i64>::new();
    let mut rows_with_sources = 0;
    for row in rows.flatten() {
        let (agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, sources_json, age_days) = row;
        for agg in [&mut overall, by_agent.entry(agent).or_default(), by_task.entry(task_class).or_default()] {
            agg.observe(&outcome, outcome_score, quality_score, age_days, latency_ms, retries, tokens_used);
        }
        let sources = sources_json.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok()).unwrap_or_default();
        if !sources.is_empty() {
            rows_with_sources += 1;
            for source in sources {
                *source_counts.entry(source).or_default() += 1;
            }
        }
    }
    let mut by_agent_vec: Vec<_> = by_agent.iter().map(|(name, agg)| aggregate_json(name, agg)).collect();
    by_agent_vec.sort_by(|left, right| value_f64_for_key(right, "reliability").partial_cmp(&value_f64_for_key(left, "reliability")).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_task_vec: Vec<_> = by_task.iter().map(|(name, agg)| aggregate_json(name, agg)).collect();
    by_task_vec.sort_by(|left, right| value_i64_for_key(right, "count").cmp(&value_i64_for_key(left, "count")));
    let mut top_sources: Vec<_> = source_counts.into_iter().collect();
    top_sources.sort_by(|left, right| right.1.cmp(&left.1));
    let top_sources: Vec<_> = top_sources.into_iter().take(10).map(|(source, hits)| json!({"source":source,"hits":hits})).collect();
    let reliability = overall.reliability();
    let recommendation = match (overall.count, reliability) {
        (0, _) => "No agent feedback telemetry recorded yet.",
        (_, r) if r < 0.65 => "Reliability is below target; tighten task decomposition and collect richer memory_sources.",
        (_, r) if r < 0.8 => "Reliability is stable but improvable; prioritize retries and conflict resolution on partial outcomes.",
        _ => "Reliability is strong; continue reinforcing high-quality runs and memory-source coverage.",
    };
    Ok(json!({"ownerId":owner_id,"horizonDays":horizon_days,"limit":limit,"sampled":overall.count,"reliability":reliability,
        "outcomes":{"success":overall.success,"partial":overall.partial,"failure":overall.failure},
        "averages":{"latencyMs":avg(overall.latency),"retries":avg(overall.retries),"tokensUsed":avg(overall.tokens)},
        "memorySourceCoverage":{"rowsWithSources":rows_with_sources,"ratio":if overall.count > 0 { rows_with_sources as f64 / overall.count as f64 } else { 0.0 }},
        "byAgent":by_agent_vec,"byTaskClass":by_task_vec,"topMemorySources":top_sources,"recommendation":recommendation}))
}

fn value_f64_for_key(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn value_i64_for_key(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

pub fn recommend_recall_k(conn: &Connection, owner_id: i64, agent: &str, task_class: Option<&str>, base_k: usize) -> Result<Option<Value>, String> {
    let task_class = normalize_task_class(task_class);
    let mut stmt = conn
        .prepare(
            "SELECT outcome, quality_score FROM agent_feedback
             WHERE owner_id = ?1 AND agent = ?2 AND task_class = ?3
               AND julianday('now') - julianday(created_at) <= 30
             ORDER BY datetime(created_at) DESC, id DESC LIMIT 40",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt.query_map(params![owner_id, agent, task_class], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))).map_err(|err| err.to_string())?;
    let (mut success, mut partial, mut failure, mut quality_total, mut count) = (0usize, 0usize, 0usize, 0.0f64, 0usize);
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
    let (recommended_k, reason) = if failure_rate >= 0.3 || partial_rate >= 0.45 {
        ((base_k + 4).min(24), "raise_depth_for_recovery")
    } else if success_rate >= 0.75 && avg_quality >= 0.82 {
        (base_k.saturating_sub(2).max(6), "reduce_depth_for_efficiency")
    } else {
        (base_k, "keep_depth_stable")
    };
    Ok(Some(json!({"agent":agent,"taskClass":task_class,"samples":count,"baseK":base_k,"recommendedK":recommended_k,"reason":reason,
        "successRate":success_rate,"partialRate":partial_rate,"failureRate":failure_rate,"avgQuality":avg_quality})))
}
