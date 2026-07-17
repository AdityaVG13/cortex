use super::*;
use rusqlite::{params, Connection};
pub(crate) fn strip_archived_text(conn: &Connection) -> usize {
    strip_archived_text_with_retention(conn, ARCHIVED_TEXT_RETENTION_DAYS)
}
pub(crate) fn strip_archived_text_with_retention(conn: &Connection, retention_days: i64) -> usize {
    let mut count = 0usize;
    count += conn
        .execute(
            "UPDATE memories SET text = '[compacted]', tags = NULL \
             WHERE status = 'archived' \
             AND text != '[compacted]' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
            params![retention_days],
        )
        .unwrap_or(0);
    count += conn
        .execute(
            "UPDATE decisions SET decision = '[compacted]', context = NULL \
             WHERE status IN ('archived', 'superseded') \
             AND decision != '[compacted]' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
            params![retention_days],
        )
        .unwrap_or(0);
    count
}
pub(crate) fn prune_expired_entries(conn: &Connection) -> usize {
    let memories_deleted = conn
        .execute("DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')", [])
        .unwrap_or(0);
    let decisions_deleted = conn
        .execute("DELETE FROM decisions WHERE expires_at IS NOT NULL AND expires_at < datetime('now')", [])
        .unwrap_or(0);
    let count = memories_deleted + decisions_deleted;
    if count > 0 {
        let payload = serde_json::json!({"memories_deleted":memories_deleted,
"decisions_deleted":decisions_deleted,})
        .to_string();
        let _ = conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('expired_entries_pruned', ?1, 'compaction', datetime('now'))",
            params![payload],
        );
    }
    count
}
