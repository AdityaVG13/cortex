use super::*;
use rusqlite::{params, Connection};
pub(crate) fn strip_archived_text(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    strip_archived_text_with_retention(conn, failures, ARCHIVED_TEXT_RETENTION_DAYS)
}
pub(crate) fn strip_archived_text_with_retention(conn: &Connection, failures: &mut Vec<MaintenanceFailure>, retention_days: i64) -> usize {
    let mut count = 0usize;
    count += exec_counted(
        conn,
        failures,
        "strip_archived_text UPDATE memories",
        "UPDATE memories SET text = '[compacted]', tags = NULL \
         WHERE status = 'archived' \
         AND text != '[compacted]' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![retention_days],
    );
    count += exec_counted(
        conn,
        failures,
        "strip_archived_text UPDATE decisions",
        "UPDATE decisions SET decision = '[compacted]', context = NULL \
         WHERE status IN ('archived', 'superseded') \
         AND decision != '[compacted]' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![retention_days],
    );
    count
}
pub(crate) fn prune_expired_entries(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let memories_deleted = exec_counted(
        conn,
        failures,
        "prune_expired_entries DELETE memories",
        "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    );
    let decisions_deleted = exec_counted(
        conn,
        failures,
        "prune_expired_entries DELETE decisions",
        "DELETE FROM decisions WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    );
    let count = memories_deleted + decisions_deleted;
    if count > 0 {
        let payload = serde_json::json!({"memories_deleted":memories_deleted,
"decisions_deleted":decisions_deleted,})
        .to_string();
        exec_counted(
            conn,
            failures,
            "prune_expired_entries INSERT expired_entries_pruned event",
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('expired_entries_pruned', ?1, 'compaction', datetime('now'))",
            params![payload],
        );
    }
    count
}
