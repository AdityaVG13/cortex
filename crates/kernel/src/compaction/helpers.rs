use rusqlite::Connection;

use super::types::{
    EVENT_NONBOOT_SOFT_LIMIT_ROWS, STORAGE_SOFT_LIMIT_BYTES, VACUUM_FREELIST_THRESHOLD_PAGES,
};

pub fn db_size_bytes(conn: &Connection) -> i64 {
    // A failed page_count is not a 0-byte database: the governor would skip
    // while the file is at or over the soft limit. Soft-limit pressure runs
    // compaction without taking the aggressive hard-limit path.
    let Ok(page_count) = conn.query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0)) else {
        return STORAGE_SOFT_LIMIT_BYTES;
    };
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap_or(4096);
    page_count.saturating_mul(page_size)
}
pub fn freelist_count(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap_or(VACUUM_FREELIST_THRESHOLD_PAGES.saturating_add(1))
}
pub fn non_boot_event_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM events WHERE type NOT IN ('boot_savings', 'boot_savings_rollup')",
        [],
        |row| row.get(0),
    )
    .unwrap_or(EVENT_NONBOOT_SOFT_LIMIT_ROWS.saturating_add(1))
}
pub fn storage_breakdown(conn: &Connection) -> Vec<(String, i64)> {
    let tables = [
        "memories",
        "decisions",
        "embeddings",
        "events",
        "recall_feedback",
        "co_occurrence",
        "memory_clusters",
        "cluster_members",
        "event_savings_rollups",
        "context_cache",
        "feed",
    ];
    let mut breakdown = Vec::new();
    for table in &tables {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        breakdown.push((table.to_string(), count));
    }
    breakdown
}
