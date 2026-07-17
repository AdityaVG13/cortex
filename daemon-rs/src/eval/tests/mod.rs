// SPDX-License-Identifier: MIT
use super::*;

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
