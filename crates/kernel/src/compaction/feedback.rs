use super::*;
use rusqlite::{params, Connection};
pub fn aggregate_old_feedback(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    aggregate_old_feedback_with_window(conn, failures, FEEDBACK_AGGREGATION_DAYS)
}
pub fn aggregate_old_feedback_with_window(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    aggregation_days: i64,
) -> usize {
    // Candidate SELECT stays fail-safe-swallowed: no candidates -> no deletes.
    let sources: Vec<(String, f64, i64)> = conn
        .prepare(
            "SELECT result_source, SUM(signal), COUNT(*) \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) > ?1 \
             GROUP BY result_source HAVING COUNT(*) > 1",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![aggregation_days], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();
    if sources.is_empty() {
        return 0;
    }
    let mut aggregated = 0usize;
    for (source, net_signal, _count) in &sources {
        let Ok(sp) = crate::db::SqliteSavepoint::enter(conn, "agg_fb") else {
            failures.push(MaintenanceFailure {
                op: "aggregate_old_feedback SAVEPOINT".into(),
                error: "failed to enter savepoint".into(),
            });
            continue;
        };
        let deleted = exec_counted(
            conn,
            failures,
            "aggregate_old_feedback DELETE recall_feedback",
            "DELETE FROM recall_feedback \
             WHERE result_source = ?1 \
             AND julianday('now') - julianday(created_at) > ?2",
            params![source, aggregation_days],
        );
        if deleted == 0 {
            continue;
        }
        let inserted = exec_counted(
            conn,
            failures,
            "aggregate_old_feedback INSERT aggregated recall_feedback",
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent, created_at) \
             VALUES ('[aggregated]', ?1, 'aggregated', ?2, 'compaction', datetime('now'))",
            params![source, net_signal],
        );
        if inserted == 0 {
            continue;
        }
        if let Err(err) = sp.release() {
            failures.push(MaintenanceFailure {
                op: "aggregate_old_feedback RELEASE".into(),
                error: err.to_string(),
            });
            continue;
        }
        aggregated += deleted;
    }
    aggregated
}
pub fn prune_old_benchmark_artifacts(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
    allow_vacuum: bool,
) -> usize {
    let result = purge_benchmark_artifacts_with_retention(conn, Some(retention_days), allow_vacuum);
    failures.extend(result.failures.iter().cloned());
    result.total_deleted()
}
pub fn purge_benchmark_artifacts_with_retention(
    conn: &Connection,
    retention_days: Option<i64>,
    allow_vacuum: bool,
) -> BenchmarkPurgeResult {
    let mut result = BenchmarkPurgeResult {
        bytes_before: db_size_bytes(conn),
        ..BenchmarkPurgeResult::default()
    };
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    let retention_window = retention_days.map(|days| format!("-{days} days"));
    exec_batch_counted(
        conn,
        &mut result.failures,
        "benchmark_purge temp table setup",
        "DROP TABLE IF EXISTS temp._benchmark_decision_ids;
         CREATE TEMP TABLE IF NOT EXISTS _benchmark_decision_ids (
           id INTEGER PRIMARY KEY
         );
         DELETE FROM _benchmark_decision_ids;",
    );
    match retention_window.as_deref() {
        Some(window) => {
            exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge SELECT benchmark decision ids (retention)",
                "INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE (LOWER(COALESCE(type, '')) = 'benchmark' \
                        OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)) \
                   AND julianday(created_at) < julianday('now', ?2)",
                params![benchmark_source_pattern.clone(), window],
            );
        }
        None => {
            exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge SELECT benchmark decision ids (full)",
                "INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE LOWER(COALESCE(type, '')) = 'benchmark' \
                    OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)",
                params![benchmark_source_pattern.clone()],
            );
        }
    }
    result.decision_conflicts_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE decision_conflicts",
        "DELETE FROM decision_conflicts \
         WHERE source_decision_id IN (SELECT id FROM _benchmark_decision_ids) \
            OR target_decision_id IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    result.embeddings_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE embeddings",
        "DELETE FROM embeddings \
         WHERE target_type = 'decision' \
           AND target_id IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    result.cluster_members_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE cluster_members",
        "DELETE FROM cluster_members \
         WHERE target_type = 'decision' \
           AND target_id IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    result.cluster_members_deleted += prune_orphan_cluster_members(conn, &mut result.failures);
    result.recall_feedback_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE recall_feedback",
        "DELETE FROM recall_feedback \
         WHERE result_source IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
            OR result_id IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    result.co_occurrence_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE co_occurrence",
        "DELETE FROM co_occurrence \
         WHERE source_a IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
            OR source_b IN (SELECT 'decision::' || id FROM _benchmark_decision_ids)",
        [],
    );
    result.decisions_deleted = exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE decisions",
        "DELETE FROM decisions WHERE id IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    result.events_deleted += exec_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DELETE events (decision_stored)",
        "DELETE FROM events \
         WHERE type = 'decision_stored' \
           AND CAST(COALESCE(json_extract(data, '$.id'), 0) AS INTEGER) IN (SELECT id FROM _benchmark_decision_ids)",
        [],
    );
    match retention_window.as_deref() {
        Some(window) => {
            result.recall_feedback_deleted += exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge DELETE recall_feedback (retention)",
                "DELETE FROM recall_feedback \
                 WHERE (LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)) \
                   AND julianday(created_at) < julianday('now', ?2)",
                params![benchmark_source_pattern.clone(), window],
            );
            result.events_deleted += exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge DELETE events (retention)",
                "DELETE FROM events \
                 WHERE (LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark') \
                   AND julianday(created_at) < julianday('now', ?2)",
                params![benchmark_source_pattern.clone(), window],
            );
        }
        None => {
            result.recall_feedback_deleted += exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge DELETE recall_feedback (full)",
                "DELETE FROM recall_feedback \
                 WHERE LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                    OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)",
                params![benchmark_source_pattern.clone()],
            );
            result.events_deleted += exec_counted(
                conn,
                &mut result.failures,
                "benchmark_purge DELETE events (full)",
                "DELETE FROM events \
                 WHERE LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                    OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                    OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                    OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark'",
                params![benchmark_source_pattern.clone()],
            );
        }
    }
    exec_batch_counted(
        conn,
        &mut result.failures,
        "benchmark_purge DROP temp table",
        "DROP TABLE IF EXISTS temp._benchmark_decision_ids;",
    );
    exec_batch_counted(
        conn,
        &mut result.failures,
        "benchmark_purge wal_checkpoint",
        "PRAGMA wal_checkpoint(TRUNCATE);",
    );
    if allow_vacuum {
        let freelist_pages = freelist_count(conn);
        if freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES {
            exec_batch_counted(
                conn,
                &mut result.failures,
                "benchmark_purge VACUUM",
                "VACUUM;",
            );
        }
    }
    result.bytes_after = db_size_bytes(conn);
    if !result.failures.is_empty() {
        eprintln!(
            "[compaction] benchmark purge: {} destructive op(s) FAILED; see FAILED lines above, reported in BenchmarkPurgeResult.failures",
            result.failures.len()
        );
    }
    result
}
