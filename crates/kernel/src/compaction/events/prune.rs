use super::*;
use rusqlite::{Connection, params};

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
                 AND (created_at IS NULL OR TRIM(created_at) = '' OR julianday(created_at) < julianday('now', ?1)) \
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
           AND (created_at IS NULL OR TRIM(created_at) = '' OR julianday(created_at) < julianday('now', ?1))",
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
    let protected_analytics_rows = crate::db::count_or_zero(
        conn,
        "SELECT COUNT(*) FROM events WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings')",
    );
    let keep_non_analytics_rows = keep_rows.saturating_sub(protected_analytics_rows);
    let prune_types_predicate = "type NOT IN ('agent_boot', 'boot_savings', 'boot_savings_rollup', 'recall_query', 'store_savings', 'tool_call_savings')";
    if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
        return exec_counted(
            conn,
            failures,
            "prune_nonboot_event_overflow DELETE events (batched)",
            &format!(
                "DELETE FROM events WHERE id IN (SELECT id FROM (SELECT id FROM events WHERE {prune_types_predicate} ORDER BY id DESC LIMIT -1 OFFSET ?1) ORDER BY id ASC LIMIT ?2)"
            ),
            params![keep_non_analytics_rows, max_rows],
        );
    }
    exec_counted(
        conn,
        failures,
        "prune_nonboot_event_overflow DELETE events",
        &format!(
            "DELETE FROM events WHERE id IN (SELECT id FROM events WHERE {prune_types_predicate} ORDER BY id DESC LIMIT -1 OFFSET ?1)"
        ),
        params![keep_non_analytics_rows],
    )
}
pub fn checkpoint_after_compaction(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    allow_vacuum: bool,
) {
    exec_batch_counted(
        conn,
        failures,
        "checkpoint_after_compaction wal_checkpoint",
        if allow_vacuum {
            "PRAGMA wal_checkpoint(TRUNCATE);"
        } else {
            "PRAGMA wal_checkpoint(PASSIVE);"
        },
    );
}
