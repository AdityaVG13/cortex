// SPDX-License-Identifier: MIT
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;


use super::*;
// ─── Helpers ────────────────────────────────────────────────────────────────

pub(crate) fn db_size_bytes(conn: &Connection) -> i64 {
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap_or(0);
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap_or(4096);
    page_count * page_size
}

pub(crate) fn freelist_count(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .unwrap_or(0)
}

pub(crate) fn non_boot_event_count(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM events WHERE type NOT IN ('boot_savings', 'boot_savings_rollup')",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Get storage breakdown by table (for diagnostics).
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
        // Approximate row size * count
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        breakdown.push((table.to_string(), count));
    }
    breakdown
}

