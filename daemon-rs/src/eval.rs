// SPDX-License-Identifier: MIT
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

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

    let total_active = active_memories + active_decisions;
    let conflict_burden = ratio(open_conflicts, active_decisions);
    let decay_burden = ratio(decayed_memories + decayed_decisions, total_active);
    let resolution_velocity = recent_resolutions as f64 / horizon_days as f64;
    let contradiction_rate = ratio(recent_conflicts, recent_recalls);

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
            "recentRecallQueries": recent_recalls
        },
        "signals": {
            "conflictBurden": conflict_burden,
            "decayBurden": decay_burden,
            "resolutionVelocity": resolution_velocity,
            "contradictionRate": contradiction_rate
        }
    })
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
            "INSERT INTO memories (text, source, status, score, pinned, created_at, updated_at)
             VALUES ('m1', 'tests::eval', 'active', 0.2, 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert memory m1");
        conn.execute(
            "INSERT INTO memories (text, source, status, score, pinned, created_at, updated_at)
             VALUES ('m2', 'tests::eval', 'active', 0.9, 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert memory m2");
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, pinned, created_at, updated_at)
             VALUES ('d1', 'ctx', 'active', 0.3, 0, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert decision d1");
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, pinned, disputes_id, created_at, updated_at)
             VALUES ('d2', 'ctx', 'disputed', 0.9, 0, 1, datetime('now'), datetime('now'))",
            [],
        )
        .expect("insert disputed decision");
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

        let snapshot = build_eval_snapshot(&conn, 30);
        let totals = snapshot.get("totals").expect("totals");
        let window = snapshot.get("window").expect("window");
        let signals = snapshot.get("signals").expect("signals");

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
    }
}
