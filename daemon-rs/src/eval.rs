// SPDX-License-Identifier: MIT
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const RATE_GATED_METRICS: [(&str, bool); 6] = [
    ("taskSuccessRate", true),
    ("firstPassSuccess", true),
    ("contradictionRate", false),
    ("staleMemoryHitRate", false),
    ("lowTrustHitRate", false),
    ("consensusPromotionPrecision", true),
];

#[derive(Default, Clone)]
struct TaskEvalAggregate {
    total: i64,
    success: i64,
    first_pass_success: i64,
    retries_total: i64,
    latencies_valid_ms: Vec<i64>,
}

impl TaskEvalAggregate {
    fn observe(&mut self, outcome: &str, retries: Option<i64>, latency_ms: Option<i64>) {
        self.total += 1;
        let retries_value = retries.unwrap_or(0).max(0);
        self.retries_total += retries_value;

        if outcome == "success" {
            self.success += 1;
            if retries_value == 0 {
                self.first_pass_success += 1;
            }
        }
        if matches!(outcome, "success" | "partial") {
            if let Some(latency) = latency_ms {
                self.latencies_valid_ms.push(latency.max(0));
            }
        }
    }

    fn task_success_rate(&self) -> f64 {
        ratio(self.success, self.total)
    }

    fn first_pass_success(&self) -> f64 {
        ratio(self.first_pass_success, self.total)
    }

    fn retry_count(&self) -> f64 {
        ratio(self.retries_total, self.total)
    }

    fn median_time_to_valid_result_ms(&self) -> f64 {
        median_i64(&self.latencies_valid_ms).unwrap_or(0.0)
    }

    fn as_json(&self) -> Value {
        json!({
            "sampleCount": self.total,
            "taskSuccessRate": self.task_success_rate(),
            "firstPassSuccess": self.first_pass_success(),
            "medianTimeToValidResultMs": self.median_time_to_valid_result_ms(),
            "retryCount": self.retry_count()
        })
    }
}

fn is_baseline_task_class(task_class: &str) -> bool {
    task_class
        .trim()
        .to_ascii_lowercase()
        .starts_with("baseline")
}

fn collect_task_metrics(
    conn: &Connection,
    since_modifier: &str,
) -> (TaskEvalAggregate, TaskEvalAggregate) {
    let mut baseline = TaskEvalAggregate::default();
    let mut assisted = TaskEvalAggregate::default();
    let mut stmt = match conn.prepare(
        "SELECT task_class, outcome, retries, latency_ms
         FROM agent_feedback
         WHERE created_at >= datetime('now', ?1)",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return (baseline, assisted),
    };
    let rows = match stmt.query_map(params![since_modifier], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<i64>>(3)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return (baseline, assisted),
    };

    for row in rows.flatten() {
        let (task_class, outcome, retries, latency_ms) = row;
        if is_baseline_task_class(&task_class) {
            baseline.observe(&outcome, retries, latency_ms);
        } else {
            assisted.observe(&outcome, retries, latency_ms);
        }
    }

    (baseline, assisted)
}

/// Build a local reliability/memory-quality snapshot over the requested horizon.
pub fn build_eval_snapshot(conn: &Connection, horizon_days: i64) -> Value {
    let horizon_days = horizon_days.clamp(1, 180);
    let since_modifier = format!("-{horizon_days} days");

    let open_conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'disputed' AND disputes_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let active_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let active_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let decayed_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active' AND score < 0.5 AND pinned = 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let decayed_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active' AND score < 0.5 AND pinned = 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let recent_conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'decision_conflict' AND created_at >= datetime('now', ?1)",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let recent_resolutions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'decision_resolve' AND created_at >= datetime('now', ?1)",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let recent_recalls: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'recall_query' AND created_at >= datetime('now', ?1)",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let recent_memory_hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories
             WHERE status = 'active'
               AND retrievals > 0
               AND last_accessed IS NOT NULL
               AND last_accessed >= datetime('now', ?1)",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let stale_memory_hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories
             WHERE status = 'active'
               AND retrievals > 0
               AND last_accessed IS NOT NULL
               AND last_accessed >= datetime('now', ?1)
               AND (score < 0.5 OR (expires_at IS NOT NULL AND expires_at <= datetime('now')))",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let recent_total_hits: i64 = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM memories
                 WHERE status = 'active'
                   AND retrievals > 0
                   AND last_accessed IS NOT NULL
                   AND last_accessed >= datetime('now', ?1))
              + (SELECT COUNT(*) FROM decisions
                 WHERE status = 'active'
                   AND retrievals > 0
                   AND last_accessed IS NOT NULL
                   AND last_accessed >= datetime('now', ?1))",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let recent_low_trust_hits: i64 = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM memories
                 WHERE status = 'active'
                   AND retrievals > 0
                   AND last_accessed IS NOT NULL
                   AND last_accessed >= datetime('now', ?1)
                   AND trust_score < 0.5)
              + (SELECT COUNT(*) FROM decisions
                 WHERE status = 'active'
                   AND retrievals > 0
                   AND last_accessed IS NOT NULL
                   AND last_accessed >= datetime('now', ?1)
                   AND trust_score < 0.5)",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let promoted_consensus: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(CAST(json_extract(data, '$.promoted') AS INTEGER)), 0)
             FROM events
             WHERE type = 'consensus'
               AND created_at >= datetime('now', ?1)
               AND json_extract(data, '$.action') = 'promoted'",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let failed_consensus: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(CAST(json_extract(data, '$.failed') AS INTEGER)), 0)
             FROM events
             WHERE type = 'consensus'
               AND created_at >= datetime('now', ?1)
               AND json_extract(data, '$.action') = 'promoted'",
            params![since_modifier.as_str()],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let (baseline_tasks, assisted_tasks) = collect_task_metrics(conn, since_modifier.as_str());
    let baseline_json = baseline_tasks.as_json();
    let assisted_json = assisted_tasks.as_json();

    let total_active = active_memories + active_decisions;
    let conflict_burden = ratio(open_conflicts, active_decisions);
    let decay_burden = ratio(decayed_memories + decayed_decisions, total_active);
    let resolution_velocity = recent_resolutions as f64 / horizon_days as f64;
    let contradiction_rate = ratio(recent_conflicts, recent_recalls);
    let stale_memory_hit_rate = ratio(stale_memory_hits, recent_memory_hits);
    let low_trust_hit_rate = ratio(recent_low_trust_hits, recent_total_hits);
    let consensus_promotion_precision =
        ratio(promoted_consensus, promoted_consensus + failed_consensus);

    let success_rate_delta = diff_signal(
        assisted_json.get("taskSuccessRate").and_then(Value::as_f64),
        baseline_json.get("taskSuccessRate").and_then(Value::as_f64),
    );
    let first_pass_delta = diff_signal(
        assisted_json
            .get("firstPassSuccess")
            .and_then(Value::as_f64),
        baseline_json
            .get("firstPassSuccess")
            .and_then(Value::as_f64),
    );
    let median_latency_delta_ms = diff_signal(
        assisted_json
            .get("medianTimeToValidResultMs")
            .and_then(Value::as_f64),
        baseline_json
            .get("medianTimeToValidResultMs")
            .and_then(Value::as_f64),
    );
    let retry_delta = diff_signal(
        assisted_json.get("retryCount").and_then(Value::as_f64),
        baseline_json.get("retryCount").and_then(Value::as_f64),
    );

    json!({
        "ok": true,
        "windowDays": horizon_days,
        "snapshotAt": Utc::now().to_rfc3339(),
        "totals": {
            "activeMemories": active_memories,
            "activeDecisions": active_decisions,
            "openConflicts": open_conflicts
        },
        "window": {
            "recentConflicts": recent_conflicts,
            "recentResolutions": recent_resolutions,
            "recentRecallQueries": recent_recalls,
            "recentMemoryHits": recent_memory_hits,
            "recentTotalHits": recent_total_hits,
            "recentLowTrustHits": recent_low_trust_hits,
            "recentConsensusPromotions": promoted_consensus,
            "recentConsensusPromotionFailures": failed_consensus
        },
        "taskMetrics": {
            "baseline": baseline_json,
            "assisted": assisted_json,
            "delta": {
                "taskSuccessRate": success_rate_delta,
                "firstPassSuccess": first_pass_delta,
                "medianTimeToValidResultMs": median_latency_delta_ms,
                "retryCount": retry_delta
            }
        },
        "signals": {
            "conflictBurden": conflict_burden,
            "decayBurden": decay_burden,
            "resolutionVelocity": resolution_velocity,
            "contradictionRate": contradiction_rate,
            "taskSuccessRate": assisted_tasks.task_success_rate(),
            "firstPassSuccess": assisted_tasks.first_pass_success(),
            "medianTimeToValidResultMs": assisted_tasks.median_time_to_valid_result_ms(),
            "retryCount": assisted_tasks.retry_count(),
            "staleMemoryHitRate": stale_memory_hit_rate,
            "lowTrustHitRate": low_trust_hit_rate,
            "consensusPromotionPrecision": consensus_promotion_precision
        }
    })
}

/// Compare two eval snapshots and report whether current metrics stay within the
/// allowed regression envelope.
pub fn build_eval_regression_gate(current: &Value, baseline: &Value, max_regression: f64) -> Value {
    let max_regression = max_regression.clamp(0.0, 1.0);
    let mut checks = Vec::new();
    let mut failed = Vec::new();

    for (metric, higher_is_better) in RATE_GATED_METRICS {
        let current_value = current
            .get("signals")
            .and_then(|signals| signals.get(metric))
            .and_then(Value::as_f64);
        let baseline_value = baseline
            .get("signals")
            .and_then(|signals| signals.get(metric))
            .and_then(Value::as_f64);
        let status = evaluate_regression(
            metric,
            higher_is_better,
            current_value,
            baseline_value,
            max_regression,
        );
        if status.get("regressed").and_then(Value::as_bool) == Some(true) {
            failed.push(status.clone());
        }
        checks.push(status);
    }

    json!({
        "ok": failed.is_empty(),
        "maxRegression": max_regression,
        "checkedMetrics": checks,
        "failedMetrics": failed
    })
}

fn evaluate_regression(
    metric: &str,
    higher_is_better: bool,
    current_value: Option<f64>,
    baseline_value: Option<f64>,
    max_regression: f64,
) -> Value {
    let (Some(current), Some(baseline)) = (current_value, baseline_value) else {
        return json!({
            "metric": metric,
            "direction": if higher_is_better { "higher_is_better" } else { "lower_is_better" },
            "status": "skipped_missing_value",
            "current": current_value,
            "baseline": baseline_value,
            "regressed": false
        });
    };

    let raw_delta = current - baseline;
    let relative_delta = if baseline.abs() > f64::EPSILON {
        raw_delta / baseline.abs()
    } else {
        raw_delta
    };
    let regressed = if higher_is_better {
        -relative_delta > max_regression
    } else {
        relative_delta > max_regression
    };

    json!({
        "metric": metric,
        "direction": if higher_is_better { "higher_is_better" } else { "lower_is_better" },
        "status": if regressed { "regressed" } else { "ok" },
        "current": current,
        "baseline": baseline,
        "delta": raw_delta,
        "relativeDelta": relative_delta,
        "regressed": regressed
    })
}

fn diff_signal(current: Option<f64>, baseline: Option<f64>) -> Value {
    match (current, baseline) {
        (Some(current), Some(baseline)) => json!(current - baseline),
        _ => Value::Null,
    }
}

fn median_i64(values: &[i64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        Some((sorted[mid - 1] as f64 + sorted[mid] as f64) / 2.0)
    } else {
        Some(sorted[mid] as f64)
    }
}

fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator <= 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_snapshot_computes_expected_signals() {
        let conn = Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories
             (text, source, status, score, trust_score, retrievals, last_accessed, pinned, created_at, updated_at)
             VALUES ('m1', 'tests::eval', 'active', 0.2, 0.8, 3, datetime('now'), 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert memory m1");
        conn.execute(
            "INSERT INTO memories
             (text, source, status, score, trust_score, retrievals, last_accessed, pinned, created_at, updated_at)
             VALUES ('m2', 'tests::eval', 'active', 0.9, 0.4, 1, datetime('now'), 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert memory m2");
        conn.execute(
            "INSERT INTO decisions
             (decision, context, status, score, trust_score, retrievals, last_accessed, pinned, created_at, updated_at)
             VALUES ('d1', 'ctx', 'active', 0.3, 0.3, 1, datetime('now'), 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert decision d1");
        conn.execute(
            "INSERT INTO decisions
             (decision, context, status, score, pinned, disputes_id, created_at, updated_at)
             VALUES ('d2', 'ctx', 'disputed', 0.9, 0, 1, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert disputed decision");

        conn.execute(
            "INSERT INTO agent_feedback
             (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, created_at)
             VALUES (0, 'codex', 'baseline:debug', 'success', 0.8, 0.8, 500, 1, 1200, datetime('now'))",
            [],
        )
        .expect("insert baseline success");
        conn.execute(
            "INSERT INTO agent_feedback
             (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, created_at)
             VALUES (0, 'codex', 'baseline:debug', 'failure', 0.2, 0.2, 700, 2, 1300, datetime('now'))",
            [],
        )
        .expect("insert baseline failure");
        conn.execute(
            "INSERT INTO agent_feedback
             (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, created_at)
             VALUES (0, 'codex', 'debug', 'success', 0.9, 0.9, 300, 0, 1000, datetime('now'))",
            [],
        )
        .expect("insert assisted success");
        conn.execute(
            "INSERT INTO agent_feedback
             (owner_id, agent, task_class, outcome, outcome_score, quality_score, latency_ms, retries, tokens_used, created_at)
             VALUES (0, 'codex', 'debug', 'partial', 0.7, 0.7, 400, 1, 1100, datetime('now'))",
            [],
        )
        .expect("insert assisted partial");

        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('decision_conflict', '{}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert conflict event");
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('decision_resolve', '{}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert resolve event");
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('recall_query', '{}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert recall event");
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('recall_query', '{}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert second recall event");
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('consensus', '{\"action\":\"promoted\",\"promoted\":2,\"failed\":1}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert consensus event");
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at)
             VALUES ('consensus', '{\"action\":\"promoted\",\"promoted\":1,\"failed\":0}', 'tests::eval', datetime('now'))",
            [],
        )
        .expect("insert second consensus event");

        let snapshot = build_eval_snapshot(&conn, 30);
        let totals = snapshot.get("totals").expect("totals");
        let window = snapshot.get("window").expect("window");
        let signals = snapshot.get("signals").expect("signals");
        let tasks = snapshot.get("taskMetrics").expect("task metrics");

        assert_eq!(
            totals.get("activeMemories").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            totals.get("activeDecisions").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(totals.get("openConflicts").and_then(Value::as_i64), Some(1));
        assert_eq!(
            window.get("recentConflicts").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            window.get("recentResolutions").and_then(Value::as_i64),
            Some(1)
        );
        assert_eq!(
            window.get("recentRecallQueries").and_then(Value::as_i64),
            Some(2)
        );
        assert_eq!(
            signals.get("conflictBurden").and_then(Value::as_f64),
            Some(1.0)
        );
        let decay_burden = signals
            .get("decayBurden")
            .and_then(Value::as_f64)
            .expect("decay burden");
        assert!(
            (decay_burden - (2.0 / 3.0)).abs() < 0.0001,
            "expected 2/3 decay burden, got {decay_burden}"
        );
        assert_eq!(
            signals.get("contradictionRate").and_then(Value::as_f64),
            Some(0.5)
        );
        assert_eq!(
            signals.get("taskSuccessRate").and_then(Value::as_f64),
            Some(0.5)
        );
        assert_eq!(
            signals.get("firstPassSuccess").and_then(Value::as_f64),
            Some(0.5)
        );
        assert_eq!(
            signals
                .get("medianTimeToValidResultMs")
                .and_then(Value::as_f64),
            Some(350.0)
        );
        assert_eq!(signals.get("retryCount").and_then(Value::as_f64), Some(0.5));
        let stale_memory_hit_rate = signals
            .get("staleMemoryHitRate")
            .and_then(Value::as_f64)
            .expect("stale memory hit rate");
        assert!(
            (stale_memory_hit_rate - 0.5).abs() < 0.0001,
            "expected stale memory hit rate 0.5, got {stale_memory_hit_rate}"
        );
        let low_trust_hit_rate = signals
            .get("lowTrustHitRate")
            .and_then(Value::as_f64)
            .expect("low trust hit rate");
        assert!(
            (low_trust_hit_rate - (2.0 / 3.0)).abs() < 0.0001,
            "expected low trust hit rate 2/3, got {low_trust_hit_rate}"
        );
        let consensus_precision = signals
            .get("consensusPromotionPrecision")
            .and_then(Value::as_f64)
            .expect("consensus precision");
        assert!(
            (consensus_precision - 0.75).abs() < 0.0001,
            "expected consensus precision 0.75, got {consensus_precision}"
        );
        assert_eq!(
            tasks["assisted"]["sampleCount"].as_i64(),
            Some(2),
            "assisted task sample count"
        );
        assert_eq!(
            tasks["baseline"]["sampleCount"].as_i64(),
            Some(2),
            "baseline task sample count"
        );
    }

    #[test]
    fn eval_regression_gate_flags_rate_regressions() {
        let baseline = json!({
            "signals": {
                "taskSuccessRate": 0.8,
                "firstPassSuccess": 0.7,
                "contradictionRate": 0.10,
                "staleMemoryHitRate": 0.10,
                "lowTrustHitRate": 0.20,
                "consensusPromotionPrecision": 0.9
            }
        });
        let current = json!({
            "signals": {
                "taskSuccessRate": 0.5,
                "firstPassSuccess": 0.65,
                "contradictionRate": 0.14,
                "staleMemoryHitRate": 0.08,
                "lowTrustHitRate": 0.18,
                "consensusPromotionPrecision": 0.88
            }
        });

        let gate = build_eval_regression_gate(&current, &baseline, 0.20);
        assert_eq!(gate["ok"].as_bool(), Some(false));
        let failed = gate["failedMetrics"]
            .as_array()
            .expect("failed metrics list should be present");
        assert!(
            failed
                .iter()
                .any(|entry| entry.get("metric").and_then(Value::as_str) == Some("taskSuccessRate")),
            "taskSuccessRate regression should be reported"
        );
    }
}
