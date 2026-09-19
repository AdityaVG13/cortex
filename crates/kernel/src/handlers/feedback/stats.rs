use super::{AgentFeedbackAggregate, avg};
use crate::protocol::nonempty_opt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::HashMap;

fn aggregate_json(name: &str, agg: &AgentFeedbackAggregate) -> Value {
    json!({"name":name,"count":agg.count,"reliability":agg.reliability(),"success":agg.success,"partial":agg.partial,"failure":agg.failure,"avgLatencyMs":avg(agg.latency),"avgRetries":avg(agg.retries),"avgTokensUsed":avg(agg.tokens)})
}

fn value_f64_for_key(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

fn value_i64_for_key(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}

pub fn build_agent_feedback_stats_payload(
    conn: &Connection,
    owner_id: i64,
    horizon_days: i64,
    limit: usize,
    task_class_filter: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<Value, String> {
    let horizon_days = super::normalize_horizon_days(Some(horizon_days));
    let limit = super::normalize_limit(Some(limit));
    let task_filter = nonempty_opt(task_class_filter).map(str::to_ascii_lowercase);
    let agent_filter = nonempty_opt(agent_filter);
    let agent_params = agent_filter.and_then(crate::handlers::store::agent_match_params);
    let (agent_ident, agent_like) = match agent_params.as_ref() {
        Some((ident, like)) => (Some(ident.as_str()), Some(like.as_str())),
        None if agent_filter.is_some() => (Some("\u{0}"), Some("\u{0} (%")),
        None => (None, None),
    };
    let mut stmt = conn.prepare(&format!("SELECT agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, memory_sources_json, julianday('now') - julianday(created_at) FROM agent_feedback WHERE owner_id = ?1 AND julianday('now') - julianday(created_at) <= ?2 AND (?3 IS NULL OR task_class = ?3) AND {} ORDER BY datetime(created_at) DESC, id DESC LIMIT ?6", crate::handlers::optional_ident_match_sql("agent", 4))).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(
            params![
                owner_id,
                horizon_days,
                task_filter,
                agent_ident,
                agent_like,
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
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    let mut overall = AgentFeedbackAggregate::default();
    let mut by_agent = HashMap::<String, AgentFeedbackAggregate>::new();
    let mut by_task = HashMap::<String, AgentFeedbackAggregate>::new();
    let mut source_counts = HashMap::<String, i64>::new();
    let mut rows_with_sources = 0;
    for row in rows {
        let (
            agent,
            task_class,
            outcome,
            outcome_score,
            quality_score,
            latency_ms,
            retries,
            tokens_used,
            sources_json,
            age_days,
        ) = row;
        let agent = agent.trim().to_ascii_lowercase();
        for agg in [
            &mut overall,
            by_agent.entry(agent).or_default(),
            by_task.entry(task_class).or_default(),
        ] {
            agg.observe(
                &outcome,
                outcome_score,
                quality_score,
                age_days,
                latency_ms,
                retries,
                tokens_used,
            );
        }
        let sources = sources_json
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default();
        if !sources.is_empty() {
            rows_with_sources += 1;
            for source in sources {
                *source_counts.entry(source).or_default() += 1;
            }
        }
    }
    let mut by_agent_vec: Vec<_> = by_agent
        .iter()
        .map(|(name, agg)| aggregate_json(name, agg))
        .collect();
    by_agent_vec.sort_by(|left, right| {
        value_f64_for_key(right, "reliability")
            .partial_cmp(&value_f64_for_key(left, "reliability"))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut by_task_vec: Vec<_> = by_task
        .iter()
        .map(|(name, agg)| aggregate_json(name, agg))
        .collect();
    by_task_vec.sort_by(|left, right| {
        value_i64_for_key(right, "count").cmp(&value_i64_for_key(left, "count"))
    });
    let mut top_sources: Vec<_> = source_counts.into_iter().collect();
    top_sources.sort_by(|left, right| right.1.cmp(&left.1));
    let top_sources: Vec<_> = top_sources
        .into_iter()
        .take(10)
        .map(|(source, hits)| json!({"source":source,"hits":hits}))
        .collect();
    let reliability = overall.reliability();
    let recommendation = match (overall.count, reliability) {
        (0, _) => "No agent feedback telemetry recorded yet.",
        (_, r) if r < 0.65 => {
            "Reliability is below target; tighten task decomposition and collect richer memory_sources."
        }
        (_, r) if r < 0.8 => {
            "Reliability is stable but improvable; prioritize retries and conflict resolution on partial outcomes."
        }
        _ => {
            "Reliability is strong; continue reinforcing high-quality runs and memory-source coverage."
        }
    };
    Ok(
        json!({"ownerId":owner_id,"horizonDays":horizon_days,"limit":limit,"sampled":overall.count,"reliability":reliability,"outcomes":{"success":overall.success,"partial":overall.partial,"failure":overall.failure},"averages":{"latencyMs":avg(overall.latency),"retries":avg(overall.retries),"tokensUsed":avg(overall.tokens)},"memorySourceCoverage":{"rowsWithSources":rows_with_sources,"ratio":if overall.count > 0 { rows_with_sources as f64 / overall.count as f64 } else { 0.0 }},"byAgent":by_agent_vec,"byTaskClass":by_task_vec,"topMemorySources":top_sources,"recommendation":recommendation}),
    )
}
