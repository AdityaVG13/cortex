use super::*;
use crate::db::{EXPIRED_SQL, UPDATED_CREATED_STAMP_SQL};
use rusqlite::{Connection, params};
pub fn strip_archived_text(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    strip_archived_text_with_retention(conn, failures, ARCHIVED_TEXT_RETENTION_DAYS)
}
/// Archived rows past their hot retention move to the cold segment: exact
/// bytes are kept (lossless codec) and a route marker replaces the inline
/// text. Nothing is discarded; the searchable universe keeps the row's
/// anchors and the planner discloses the cold partition.
///
/// Blank `updated_at` is not NULL. `COALESCE(updated_at, created_at)` sticks
/// on `''`, `julianday('')` is NULL, and the age predicate never matches, so those
/// rows skip cold-move forever. Fall through `NULLIF(TRIM(...))` the same way
/// aging/decay does.
pub fn strip_archived_text_with_retention(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    let mut count = 0usize;
    for (table, namespace, text_col, status_clause) in [
        ("memories", "memory", "text", "status = 'archived'"),
        (
            "decisions",
            "decision",
            "decision",
            "status IN ('archived', 'superseded')",
        ),
    ] {
        let sql = format!(
            "SELECT id FROM {table} WHERE {status_clause} AND {text_col} NOT LIKE '[cold:%' AND {text_col} != '[compacted]' AND julianday('now') - julianday({UPDATED_CREATED_STAMP_SQL}) > ?1 ORDER BY id LIMIT 500"
        );
        let ids: Vec<i64> = match conn.prepare(&sql).and_then(|mut stmt| {
            stmt.query_map(params![retention_days], |r| r.get::<_, i64>(0))
                .map(|rows| rows.flatten().collect())
        }) {
            Ok(ids) => ids,
            Err(err) => {
                failures.push(MaintenanceFailure {
                    op: format!("strip_archived_text SELECT {table}"),
                    error: err.to_string(),
                });
                continue;
            }
        };
        for id in ids {
            match crate::db::cold::move_to_cold(conn, namespace, id) {
                Ok(Some(_)) => count += 1,
                Ok(None) => {}
                Err(err) => failures.push(MaintenanceFailure {
                    op: format!("strip_archived_text UPDATE {table} (cold move {id})"),
                    error: err.to_string(),
                }),
            }
        }
    }
    count
}
pub fn prune_expired_entries(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let memories_deleted = exec_counted(
        conn,
        failures,
        "prune_expired_entries DELETE memories",
        &format!("DELETE FROM memories WHERE {EXPIRED_SQL}"),
        [],
    );
    let decisions_deleted = exec_counted(
        conn,
        failures,
        "prune_expired_entries DELETE decisions",
        &format!("DELETE FROM decisions WHERE {EXPIRED_SQL}"),
        [],
    );
    let count = memories_deleted + decisions_deleted;
    if count > 0 {
        let payload = serde_json::json!({"memories_deleted":memories_deleted,"decisions_deleted":decisions_deleted,}).to_string();
        exec_counted(
            conn,
            failures,
            "prune_expired_entries INSERT expired_entries_pruned event",
            "INSERT INTO events (type, data, source_agent, created_at) VALUES ('expired_entries_pruned', ?1, 'compaction', datetime('now'))",
            params![payload],
        );
    }
    count
}
