pub mod accounting;
use chrono::Utc;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
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
        self.total = self.total.saturating_add(1);
        let retries_value = retries.unwrap_or(0).max(0);
        self.retries_total = self.retries_total.saturating_add(retries_value);
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
        json!({"sampleCount":self.total,"taskSuccessRate":self.task_success_rate(),"firstPassSuccess":self.first_pass_success(),"medianTimeToValidResultMs":self.median_time_to_valid_result_ms(),"retryCount":self.retry_count()})
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
) -> Result<(TaskEvalAggregate, TaskEvalAggregate), String> {
    let mut baseline = TaskEvalAggregate::default();
    let mut assisted = TaskEvalAggregate::default();
    let mut stmt = conn.prepare("SELECT task_class, outcome, retries, latency_ms FROM agent_feedback WHERE julianday(created_at) >= julianday('now', ?1)").map_err(|error| format!("eval task query prepare failed: {error}"))?;
    let rows = stmt
        .query_map(params![since_modifier], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| format!("eval task query failed: {error}"))?;
    for row in rows {
        let (task_class, outcome, retries, latency_ms) =
            row.map_err(|error| format!("eval task row failed: {error}"))?;
        let agg = if is_baseline_task_class(&task_class) {
            &mut baseline
        } else {
            &mut assisted
        };
        agg.observe(&outcome, retries, latency_ms);
    }
    Ok((baseline, assisted))
}

fn query_i64(conn: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<i64, String> {
    conn.query_row(sql, params, |row| row.get(0))
        .map_err(|error| format!("eval count failed: {error}"))
}

fn count_active(conn: &Connection, table: &str, extra: &str) -> Result<i64, String> {
    query_i64(
        conn,
        &format!("SELECT COUNT(*) FROM {table} WHERE status = 'active'{extra}"),
        [],
    )
}

fn count_events(conn: &Connection, event_type: &str, since: &str) -> Result<i64, String> {
    query_i64(
        conn,
        "SELECT COUNT(*) FROM events WHERE type = ?1 AND julianday(created_at) >= julianday('now', ?2)",
        params![event_type, since],
    )
}

const HIT_WHERE: &str = "status = 'active' AND retrievals > 0 AND last_accessed IS NOT NULL AND julianday(last_accessed) >= julianday('now', ?1)";
const STALE_EXTRA: &str = " AND (score < 0.5 OR (expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) <= julianday('now')))";

fn count_hits(conn: &Connection, table: &str, extra: &str, since: &str) -> Result<i64, String> {
    query_i64(
        conn,
        &format!("SELECT COUNT(*) FROM {table} WHERE {HIT_WHERE}{extra}"),
        params![since],
    )
}

fn count_hits_both(conn: &Connection, extra: &str, since: &str) -> Result<i64, String> {
    Ok(count_hits(conn, "memories", extra, since)? + count_hits(conn, "decisions", extra, since)?)
}

fn consensus_sum(conn: &Connection, action: &str, since: &str) -> Result<i64, String> {
    query_i64(
        conn,
        &format!(
            "SELECT COALESCE(SUM(CAST(json_extract(data, '$.{action}') AS INTEGER)), 0) FROM events WHERE type = 'consensus' AND julianday(created_at) >= julianday('now', ?1) AND json_extract(data, '$.action') = '{action}'"
        ),
        params![since],
    )
}

fn json_delta(assisted: &Value, baseline: &Value, key: &str) -> Value {
    diff_signal(
        assisted.get(key).and_then(Value::as_f64),
        baseline.get(key).and_then(Value::as_f64),
    )
}

fn eval_snapshot_error(horizon_days: i64, error: String) -> Value {
    json!({"ok":false,"error":error,"windowDays":horizon_days,"snapshotAt":Utc::now().to_rfc3339()})
}

pub fn build_eval_snapshot(conn: &Connection, horizon_days: i64) -> Value {
    let horizon_days = horizon_days.clamp(1, 180);
    match build_eval_snapshot_inner(conn, horizon_days) {
        Ok(value) => value,
        Err(error) => eval_snapshot_error(horizon_days, error),
    }
}

fn build_eval_snapshot_inner(conn: &Connection, horizon_days: i64) -> Result<Value, String> {
    let since_modifier = format!("-{horizon_days} days");
    let since = since_modifier.as_str();
    let open_conflicts = query_i64(
        conn,
        "SELECT COUNT(*) FROM decisions WHERE status = 'disputed' AND disputes_id IS NOT NULL",
        [],
    )?;
    let active_memories = count_active(conn, "memories", "")?;
    let active_decisions = count_active(conn, "decisions", "")?;
    let decayed_memories = count_active(conn, "memories", " AND score < 0.5 AND pinned = 0")?;
    let decayed_decisions = count_active(conn, "decisions", " AND score < 0.5 AND pinned = 0")?;
    let recent_conflicts = count_events(conn, "decision_conflict", since)?;
    let recent_resolutions = count_events(conn, "decision_resolve", since)?;
    let recent_recalls = count_events(conn, "recall_query", since)?;
    let recent_memory_hits = count_hits(conn, "memories", "", since)?;
    let stale_memory_hits = count_hits(conn, "memories", STALE_EXTRA, since)?;
    let recent_total_hits = count_hits_both(conn, "", since)?;
    let recent_low_trust_hits = count_hits_both(conn, " AND trust_score < 0.5", since)?;
    let promoted_consensus = consensus_sum(conn, "promoted", since)?;
    let failed_consensus = consensus_sum(conn, "failed", since)?;
    let (baseline_tasks, assisted_tasks) = collect_task_metrics(conn, since_modifier.as_str())?;
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
    let success_rate_delta = json_delta(&assisted_json, &baseline_json, "taskSuccessRate");
    let first_pass_delta = json_delta(&assisted_json, &baseline_json, "firstPassSuccess");
    let median_latency_delta_ms =
        json_delta(&assisted_json, &baseline_json, "medianTimeToValidResultMs");
    let retry_delta = json_delta(&assisted_json, &baseline_json, "retryCount");
    Ok(
        json!({"ok":true,"windowDays":horizon_days,"snapshotAt":Utc::now().to_rfc3339(),"totals":{"activeMemories":active_memories,"activeDecisions":active_decisions,"openConflicts":open_conflicts},"window":{"recentConflicts":recent_conflicts,"recentResolutions":recent_resolutions,"recentRecallQueries":recent_recalls,"recentMemoryHits":recent_memory_hits,"recentTotalHits":recent_total_hits,"recentLowTrustHits":recent_low_trust_hits,"recentConsensusPromotions":promoted_consensus,"recentConsensusPromotionFailures":failed_consensus},"taskMetrics":{"baseline":baseline_json,"assisted":assisted_json,"delta":{"taskSuccessRate":success_rate_delta,"firstPassSuccess":first_pass_delta,"medianTimeToValidResultMs":median_latency_delta_ms,"retryCount":retry_delta}},"signals":{"conflictBurden":conflict_burden,"decayBurden":decay_burden,"resolutionVelocity":resolution_velocity,"contradictionRate":contradiction_rate,"taskSuccessRate":assisted_tasks.task_success_rate(),"firstPassSuccess":assisted_tasks.first_pass_success(),"medianTimeToValidResultMs":assisted_tasks.median_time_to_valid_result_ms(),"retryCount":assisted_tasks.retry_count(),"staleMemoryHitRate":stale_memory_hit_rate,"lowTrustHitRate":low_trust_hit_rate,"consensusPromotionPrecision":consensus_promotion_precision}}),
    )
}

fn signal_f64(root: &Value, metric: &str) -> Option<f64> {
    root.get("signals")
        .and_then(|signals| signals.get(metric))
        .and_then(crate::protocol::json_f64)
}

pub fn build_eval_regression_gate(current: &Value, baseline: &Value, max_regression: f64) -> Value {
    let max_regression = max_regression.clamp(0.0, 1.0);
    let mut checks = Vec::new();
    let mut failed = Vec::new();
    for (metric, higher_is_better) in RATE_GATED_METRICS {
        let status = evaluate_regression(
            metric,
            higher_is_better,
            signal_f64(current, metric),
            signal_f64(baseline, metric),
            max_regression,
        );
        if status.get("regressed").and_then(Value::as_bool) == Some(true) {
            failed.push(status.clone());
        }
        checks.push(status);
    }
    json!({"ok":failed.is_empty(),"maxRegression":max_regression,"checkedMetrics":checks,"failedMetrics":failed})
}
fn evaluate_regression(
    metric: &str,
    higher_is_better: bool,
    current_value: Option<f64>,
    baseline_value: Option<f64>,
    max_regression: f64,
) -> Value {
    let (Some(current), Some(baseline)) = (current_value, baseline_value) else {
        // A regression gate that skips a missing metric fail-opens under
        // `--fail-on-regression`. Incomplete baselines cannot prove stability.
        return json!({"metric":metric,"direction":if higher_is_better{"higher_is_better"}else{"lower_is_better"},"status":"missing_value","current":current_value,"baseline":baseline_value,"regressed":true});
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
    json!({"metric":metric,"direction":if higher_is_better{"higher_is_better"}else{"lower_is_better"},"status":if regressed{"regressed"}else{"ok"},"current":current,"baseline":baseline,"delta":raw_delta,"relativeDelta":relative_delta,"regressed":regressed})
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
