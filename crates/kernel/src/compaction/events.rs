use super::*;
use rusqlite::{params, Connection};
pub fn rollup_old_boot_savings(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    rollup_old_boot_savings_with_retention(conn, failures, BOOT_SAVINGS_RETENTION_DAYS)
}
pub fn rollup_old_boot_savings_with_retention(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    // Read-only aggregate SELECTs stay fail-safe-swallowed: a failed read
    // yields zeros and the early-return below skips every destructive op.
    let retention_window = format!("-{retention_days} days");
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    let (old_saved, old_served, old_baseline, old_boots): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events \
             WHERE type = 'boot_savings' \
               AND created_at < datetime('now', ?1) \
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
            params![retention_window.clone(), benchmark_source_pattern.clone()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    let (rollup_saved, rollup_served, rollup_baseline, rollup_boots, rollup_rows): (
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.boots') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events \
             WHERE type = 'boot_savings_rollup'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap_or((0, 0, 0, 0, 0));
    if old_boots <= 0 && rollup_rows <= 1 {
        return 0;
    }
    let merged_saved = old_saved + rollup_saved;
    let merged_served = old_served + rollup_served;
    let merged_baseline = old_baseline + rollup_baseline;
    let merged_boots = old_boots + rollup_boots;
    let deleted_old = exec_counted(
        conn,
        failures,
        "rollup_old_boot_savings DELETE events (boot_savings)",
        "DELETE FROM events \
         WHERE type = 'boot_savings' \
           AND created_at < datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
        params![retention_window, benchmark_source_pattern],
    );
    let deleted_rollups = exec_counted(
        conn,
        failures,
        "rollup_old_boot_savings DELETE events (boot_savings_rollup)",
        "DELETE FROM events WHERE type = 'boot_savings_rollup'",
        [],
    );
    if merged_boots > 0 {
        let payload = serde_json::json!({"saved":
merged_saved,"served":merged_served,"baseline":merged_baseline,"boots":merged_boots,"retention_days":retention_days,"rolled_up_at"
:chrono::Utc::now().to_rfc3339(),})
        .to_string();
        exec_counted(
            conn,
            failures,
            "rollup_old_boot_savings INSERT boot_savings_rollup",
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings_rollup', ?1, 'compaction', datetime('now'))",
            params![payload],
        );
        let consolidated_rollups = deleted_rollups.saturating_sub(1);
        deleted_old + consolidated_rollups
    } else {
        deleted_old + deleted_rollups
    }
}
pub fn rollup_old_savings_events(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    let retention_window = format!("-{retention_days} days");
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    type SavingsRollupRow = (String, i64, String, i64, i64, i64, i64, i64, i64);
    // Candidate SELECT stays fail-safe-swallowed: no candidates -> no deletes.
    let rollup_rows:Vec<SavingsRollupRow>=conn.prepare(
"SELECT \
                 SUBSTR(created_at, 1, 10) AS day, \
                 COALESCE(CAST(strftime('%H', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER), 0) AS hour, \
                 CASE \
                     WHEN type = 'recall_query' THEN 'recall' \
                     WHEN type = 'store_savings' THEN 'store' \
                     WHEN type = 'tool_call_savings' THEN 'tool' \
                 END AS operation, \
                 COALESCE(SUM(CASE \
                     WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                     WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                     WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                     ELSE 0 END), 0) AS saved, \
                 COALESCE(SUM(CASE \
                     WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.spent') AS INTEGER), COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)) \
                     WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) \
                     WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) \
                     ELSE 0 END), 0) AS served, \
                 COALESCE(SUM(CASE \
                     WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.budget') AS INTEGER), COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)) \
                     WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) \
                     WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) \
                     ELSE 0 END), 0) AS baseline, \
                 COUNT(*) AS events, \
                 SUM(CASE \
                     WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 1 \
                     ELSE 0 END) AS hits, \
                 SUM(CASE \
                     WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 0 \
                     WHEN type = 'recall_query' THEN 1 \
                     ELSE 0 END) AS misses \
             FROM events \
             WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') \
               AND created_at IS NOT NULL \
               AND created_at < datetime('now', ?1) \
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) \
              GROUP BY day, hour, operation"
,).and_then(|mut stmt|{let rows=stmt.query_map(params![retention_window.clone(),benchmark_source_pattern.clone()],|row|{Ok((row.
get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,
row.get::<_,i64>(6)?,row.get::<_,i64>(7)?,row.get::<_,i64>(8)?,))})?;Ok(rows.flatten().collect())}).unwrap_or_default();
    if rollup_rows.is_empty() {
        return 0;
    }
    for (day, hour, operation, saved, served, baseline, events, hits, misses) in rollup_rows {
        exec_counted(
            conn,
            failures,
            "rollup_old_savings_events INSERT event_savings_rollups",
            "INSERT INTO event_savings_rollups \
                 (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now')) \
             ON CONFLICT(day, hour, operation) DO UPDATE SET \
                 saved = event_savings_rollups.saved + excluded.saved, \
                 served = event_savings_rollups.served + excluded.served, \
                 baseline = event_savings_rollups.baseline + excluded.baseline, \
                 events = event_savings_rollups.events + excluded.events, \
                 hits = event_savings_rollups.hits + excluded.hits, \
                 misses = event_savings_rollups.misses + excluded.misses, \
                 updated_at = datetime('now')",
            params![day, hour, operation, saved, served, baseline, events, hits, misses],
        );
    }
    exec_counted(
        conn,
        failures,
        "rollup_old_savings_events DELETE events (savings)",
        "DELETE FROM events \
         WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') \
           AND created_at IS NOT NULL \
           AND created_at < datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
        params![retention_window, benchmark_source_pattern],
    )
}
pub fn prune_old_event_savings_rollups(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    exec_counted(
        conn,
        failures,
        "prune_old_event_savings_rollups DELETE event_savings_rollups",
        "DELETE FROM event_savings_rollups \
         WHERE day < date('now', ?1)",
        params![format!("-{retention_days} days")],
    )
}
pub fn prune_old_events_with_retention_limit(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
    max_delete_rows: Option<i64>,
) -> usize {
    let retention_window = format!("-{retention_days} days");
    if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
        return exec_counted(
            conn,
            failures,
            "prune_old_events DELETE events (batched)",
            "DELETE FROM events \
             WHERE id IN ( \
               SELECT id \
               FROM events \
               WHERE type NOT IN ('boot_savings', 'boot_savings_rollup') \
                 AND (created_at IS NULL OR TRIM(created_at) = '' OR created_at < datetime('now', ?1)) \
               ORDER BY id ASC \
               LIMIT ?2 \
             )",
            params![retention_window, max_rows],
        );
    }
    exec_counted(
        conn,
        failures,
        "prune_old_events DELETE events",
        "DELETE FROM events \
         WHERE type NOT IN ('boot_savings', 'boot_savings_rollup') \
           AND (created_at IS NULL OR TRIM(created_at) = '' OR created_at < datetime('now', ?1))",
        params![retention_window],
    )
}
pub fn prune_event_type_caps_with_limit(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    caps: &[(&str, i64)],
    max_delete_rows: Option<i64>,
) -> usize {
    let mut total = 0usize;
    for (event_type, keep_rows) in caps.iter().copied() {
        if keep_rows <= 0 {
            continue;
        }
        let op = format!("prune_event_type_caps DELETE events ({event_type})");
        let deleted = if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
            exec_counted(
                conn,
                failures,
                &op,
                "DELETE FROM events
                 WHERE id IN (
                   SELECT id
                   FROM (
                     SELECT id
                     FROM events
                     WHERE type = ?1
                     ORDER BY id DESC
                     LIMIT -1 OFFSET ?2
                   )
                   ORDER BY id ASC
                   LIMIT ?3
                 )",
                params![event_type, keep_rows, max_rows],
            )
        } else {
            exec_counted(
                conn,
                failures,
                &op,
                "DELETE FROM events
                 WHERE id IN (
                   SELECT id
                   FROM events
                   WHERE type = ?1
                   ORDER BY id DESC
                   LIMIT -1 OFFSET ?2
                 )",
                params![event_type, keep_rows],
            )
        };
        total += deleted;
    }
    total
}
pub fn prune_nonboot_event_overflow_with_limit(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    keep_rows: i64,
    max_delete_rows: Option<i64>,
) -> usize {
    if keep_rows <= 0 {
        return 0;
    }
    // Read-only protected-rows COUNT stays fail-safe-swallowed: zero shifts
    // only the overflow threshold, never deletes a row by itself.
    let protected_analytics_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM events
             WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let keep_non_analytics_rows = keep_rows.saturating_sub(protected_analytics_rows);
    let prune_types_predicate = "type NOT IN (
        'agent_boot',
        'boot_savings',
        'boot_savings_rollup',
        'recall_query',
        'store_savings',
        'tool_call_savings'
    )";
    if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
        return exec_counted(
            conn,
            failures,
            "prune_nonboot_event_overflow DELETE events (batched)",
            &format!(
                "DELETE FROM events
                 WHERE id IN (
                   SELECT id
                   FROM (
                     SELECT id
                     FROM events
                     WHERE {prune_types_predicate}
                     ORDER BY id DESC
                     LIMIT -1 OFFSET ?1
                   )
                   ORDER BY id ASC
                   LIMIT ?2
                 )"
            ),
            params![keep_non_analytics_rows, max_rows],
        );
    }
    exec_counted(
        conn,
        failures,
        "prune_nonboot_event_overflow DELETE events",
        &format!(
            "DELETE FROM events
             WHERE id IN (
               SELECT id
               FROM events
                WHERE {prune_types_predicate}
               ORDER BY id DESC
               LIMIT -1 OFFSET ?1
             )"
        ),
        params![keep_non_analytics_rows],
    )
}
pub fn checkpoint_after_compaction(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    allow_vacuum: bool,
) {
    if allow_vacuum {
        exec_batch_counted(
            conn,
            failures,
            "checkpoint_after_compaction wal_checkpoint",
            "PRAGMA wal_checkpoint(TRUNCATE);",
        );
    } else {
        exec_batch_counted(
            conn,
            failures,
            "checkpoint_after_compaction wal_checkpoint",
            "PRAGMA wal_checkpoint(PASSIVE);",
        );
    }
}
