mod recall;
mod stats;

use crate::protocol::{arg_f64, arg_str, nonempty_opt, nonempty_str};
use rusqlite::{Connection, params};
use serde::Deserialize;
use serde_json::{Value, json};

pub use stats::build_agent_feedback_stats_payload;

pub use recall::{
    IMMUNITY_THRESHOLD, IMMUNITY_WINDOW_DAYS, compute_boosts, has_retrieval_immunity, parse_source,
};

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
pub(super) struct AgentFeedbackAggregate {
    pub(super) count: i64,
    weighted_sum: f64,
    weight_total: f64,
    pub(super) success: i64,
    pub(super) partial: i64,
    pub(super) failure: i64,
    pub(super) latency: (i64, i64),
    pub(super) retries: (i64, i64),
    pub(super) tokens: (i64, i64),
}

impl AgentFeedbackAggregate {
    pub(super) fn observe(
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
        let weight =
            (-((2.0f64).ln() / AGENT_FEEDBACK_DECAY_HALF_LIFE_DAYS) * age_days.max(0.0)).exp();
        self.weighted_sum += (outcome_score * 0.6 + quality_score * 0.4).clamp(0.0, 1.0) * weight;
        self.weight_total += weight;
        observe_optional(latency_ms, &mut self.latency);
        observe_optional(retries, &mut self.retries);
        observe_optional(tokens_used, &mut self.tokens);
    }

    pub(super) fn reliability(&self) -> f64 {
        (self.weight_total > 0.0)
            .then(|| (self.weighted_sum / self.weight_total).clamp(0.0, 1.0))
            .unwrap_or(0.0)
    }
}

fn observe_optional(value: Option<i64>, acc: &mut (i64, i64)) {
    if let Some(value) = value {
        acc.0 += value.max(0);
        acc.1 += 1;
    }
}

pub(super) fn avg(acc: (i64, i64)) -> Option<f64> {
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

fn nonempty_ascii_lower(value: Option<&str>, fallback: &str) -> String {
    nonempty_opt(value).unwrap_or(fallback).to_ascii_lowercase()
}

fn normalize_task_class(value: Option<&str>) -> String {
    nonempty_ascii_lower(value, "general")
}

fn normalize_agent(value: Option<&str>, fallback_agent: &str) -> String {
    nonempty_ascii_lower(value, fallback_agent.trim())
}

pub fn normalize_horizon_days(value: Option<i64>) -> i64 {
    value
        .unwrap_or(AGENT_FEEDBACK_DEFAULT_HORIZON_DAYS)
        .clamp(1, 180)
}

pub fn normalize_limit(value: Option<usize>) -> usize {
    value
        .unwrap_or(AGENT_FEEDBACK_DEFAULT_LIMIT)
        .clamp(10, 2_000)
}

fn value_i64(args: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let v = args.get(*key)?;
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
            .or_else(|| {
                v.as_f64().and_then(|x| {
                    let rounded = x.round();
                    rounded.is_finite().then_some(rounded as i64)
                })
            })
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

fn value_string_array(args: &Value, keys: &[&str]) -> Vec<String> {
    let tokens = |iter: &mut dyn Iterator<Item = &str>| {
        iter.filter_map(nonempty_str)
            .map(str::to_string)
            .take(crate::clockwork::MAX_QUERY_TOKENS)
            .collect()
    };
    keys.iter()
        .find_map(|key| args.get(*key))
        .map(|v| match v {
            Value::Array(items) => tokens(&mut items.iter().filter_map(Value::as_str)),
            Value::String(s) => tokens(&mut s.split(',')),
            _ => Vec::new(),
        })
        .unwrap_or_default()
}

pub fn record_agent_feedback_from_value(
    conn: &Connection,
    owner_id: i64,
    args: &Value,
    fallback_agent: &str,
) -> Result<Value, String> {
    let outcome = normalize_outcome(arg_str(args, &["outcome"])).ok_or_else(|| {
        "Missing or invalid outcome (expected success|partial|failure)".to_string()
    })?;
    let agent = normalize_agent(
        arg_str(args, &["agent", "source_agent", "sourceAgent"]),
        fallback_agent,
    );
    let task_class = normalize_task_class(arg_str(args, &["task_class", "taskClass"]));
    let outcome_score = arg_f64(args, &["outcome_score", "outcomeScore"])
        .unwrap_or(match outcome {
            "success" => 1.0,
            "partial" => 0.5,
            _ => 0.0,
        })
        .clamp(0.0, 1.0);
    let quality_score = arg_f64(args, &["quality_score", "qualityScore"])
        .unwrap_or(0.7)
        .clamp(0.0, 1.0);
    let latency_ms = value_i64(args, &["latency_ms", "latencyMs"]).map(|value| value.max(0));
    let retries = value_i64(args, &["retries"]).map(|value| value.max(0));
    let tokens_used = value_i64(args, &["tokens_used", "tokensUsed"]).map(|value| value.max(0));
    let memory_sources = value_string_array(args, &["memory_sources", "memorySources"]);
    let notes = arg_str(args, &["notes"]).map(str::to_string);
    let memory_sources_json =
        serde_json::to_string(&memory_sources).map_err(|err| err.to_string())?;
    conn.execute("INSERT INTO agent_feedback (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json, notes]).map_err(|err| err.to_string())?;
    Ok(
        json!({"stored":true,"ownerId":owner_id,"agent":agent,"taskClass":task_class,"outcome":outcome,"outcomeScore":outcome_score,"qualityScore":quality_score,"memorySources":memory_sources}),
    )
}

pub fn recommend_recall_k(
    conn: &Connection,
    owner_id: i64,
    agent: &str,
    task_class: Option<&str>,
    base_k: usize,
) -> Result<Option<Value>, String> {
    let task_class = normalize_task_class(task_class);
    let Some((ident, like)) = crate::handlers::store::agent_match_params(agent) else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(&format!("SELECT outcome, quality_score FROM agent_feedback WHERE owner_id = ?1 AND {} AND task_class = ?4 AND julianday('now') - julianday(created_at) <= 30 ORDER BY datetime(created_at) DESC, id DESC LIMIT 40", crate::handlers::ident_match_sql("agent", 2))).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![owner_id, ident, like, task_class], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    let (mut success, mut partial, mut failure, mut quality_total, mut count) =
        (0usize, 0usize, 0usize, 0.0f64, 0usize);
    for (outcome, quality) in rows {
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
        (
            base_k.saturating_sub(2).max(6),
            "reduce_depth_for_efficiency",
        )
    } else {
        (base_k, "keep_depth_stable")
    };
    Ok(Some(
        json!({"agent":agent,"taskClass":task_class,"samples":count,"baseK":base_k,"recommendedK":recommended_k,"reason":reason,"successRate":success_rate,"partialRate":partial_rate,"failureRate":failure_rate,"avgQuality":avg_quality}),
    ))
}
