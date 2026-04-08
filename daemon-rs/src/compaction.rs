// SPDX-License-Identifier: MIT
//! Storage Compaction — keeps Cortex's database lean at scale.
//!
//! Strategies (ordered by impact):
//!   1. Event log rotation: delete events older than 30 days
//!   2. Archived entry cleanup: drop text/embeddings for ancient entries (keep metadata)
//!   3. Crystal member embedding pruning: members served via crystal, not individual search
//!   4. Feedback aggregation: compact old per-signal rows into per-source summaries
//!   5. WAL + VACUUM: reclaim freed pages
//!
//! Designed for teams of 10+ agents doing hundreds of stores/day.
//! Target: keep DB under 500MB regardless of usage volume.

use rusqlite::{params, Connection};

// ─── Constants ──────────────────────────────────────────────────────────────

/// Non-boot events older than this are deleted.
const EVENT_RETENTION_DAYS: i64 = 14;

/// Only VACUUM when SQLite reports enough reclaimable pages to justify the IO.
const VACUUM_FREELIST_THRESHOLD_PAGES: i64 = 100;

/// Archived entries older than this have their text stripped (metadata kept).
const ARCHIVED_TEXT_RETENTION_DAYS: i64 = 90;

/// Feedback signals older than this are aggregated into summaries.
const FEEDBACK_AGGREGATION_DAYS: i64 = 60;

// ─── Result ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CompactionResult {
    pub events_pruned: usize,
    pub archived_text_stripped: usize,
    pub expired_pruned: usize,
    pub crystal_embeddings_pruned: usize,
    pub feedback_aggregated: usize,
    pub bytes_before: i64,
    pub bytes_after: i64,
}

// ─── Main entry point ───────────────────────────────────────────────────────

/// Run one compaction pass. Safe to call repeatedly.
pub fn run_compaction(conn: &Connection) -> CompactionResult {
    let mut result = CompactionResult {
        bytes_before: db_size_bytes(conn),
        ..CompactionResult::default()
    };

    // 1. Event log rotation
    result.events_pruned = prune_old_events(conn);

    // 2. Archived entry text cleanup
    result.archived_text_stripped = strip_archived_text(conn);

    // 3. Hard-expiration cleanup
    result.expired_pruned = prune_expired_entries(conn);

    // 4. Crystal member embedding pruning
    result.crystal_embeddings_pruned = prune_crystal_member_embeddings(conn);

    // 5. Feedback aggregation
    result.feedback_aggregated = aggregate_old_feedback(conn);

    // 6. Reclaim space
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    // VACUUM is expensive. Use SQLite's freelist_count instead of raw delete
    // volume so we only pay the cost when pages are actually reclaimable.
    let freelist_pages = freelist_count(conn);
    let total_deleted = result.events_pruned
        + result.archived_text_stripped
        + result.expired_pruned
        + result.crystal_embeddings_pruned
        + result.feedback_aggregated;
    if freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES {
        let _ = conn.execute_batch("VACUUM;");
    }

    result.bytes_after = db_size_bytes(conn);

    if total_deleted > 0 {
        let saved_kb = (result.bytes_before - result.bytes_after) / 1024;
        eprintln!(
            "[compaction] Pruned: {} events, {} archived texts, {} expired rows, {} crystal embeddings, {} feedback rows. Saved {}KB",
            result.events_pruned, result.archived_text_stripped,
            result.expired_pruned,
            result.crystal_embeddings_pruned, result.feedback_aggregated,
            saved_kb
        );
    }

    result
}

// ─── Event log rotation ─────────────────────────────────────────────────────

fn prune_old_events(conn: &Connection) -> usize {
    conn.execute(
        "DELETE FROM events \
         WHERE type NOT IN ('agent_boot', 'boot_savings') \
         AND created_at < datetime('now', ?1)",
        params![format!("-{EVENT_RETENTION_DAYS} days")],
    )
    .unwrap_or(0)
}

// ─── Archived entry text cleanup ────────────────────────────────────────────

/// Strip full text from archived entries older than retention period.
/// Keeps: id, source, type, status, created_at, score (for audit).
/// Drops: text, compressed_text, tags, context (saves space).
fn strip_archived_text(conn: &Connection) -> usize {
    let mut count = 0usize;

    count += conn
        .execute(
            "UPDATE memories SET text = '[compacted]', tags = NULL \
             WHERE status = 'archived' \
             AND text != '[compacted]' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
            params![ARCHIVED_TEXT_RETENTION_DAYS],
        )
        .unwrap_or(0);

    count += conn
        .execute(
            "UPDATE decisions SET decision = '[compacted]', context = NULL \
             WHERE status = 'archived' \
             AND decision != '[compacted]' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
            params![ARCHIVED_TEXT_RETENTION_DAYS],
        )
        .unwrap_or(0);

    count
}

fn prune_expired_entries(conn: &Connection) -> usize {
    let memories_deleted = conn
        .execute(
            "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
            [],
        )
        .unwrap_or(0);

    let decisions_deleted = conn
        .execute(
            "DELETE FROM decisions WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
            [],
        )
        .unwrap_or(0);

    let count = memories_deleted + decisions_deleted;
    if count > 0 {
        let payload = serde_json::json!({
            "memories_deleted": memories_deleted,
            "decisions_deleted": decisions_deleted,
        })
        .to_string();
        let _ = conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('expired_entries_pruned', ?1, 'compaction', datetime('now'))",
            params![payload],
        );
    }

    count
}

// ─── Crystal member embedding pruning ───────────────────────────────────────

/// Remove individual embeddings for entries that are members of a crystal.
/// The crystal's embedding handles recall; individual members are found by
/// ID lookup through cluster_members, not semantic search.
fn prune_crystal_member_embeddings(conn: &Connection) -> usize {
    let mut count = 0usize;

    count += conn
        .execute(
            "DELETE FROM embeddings WHERE target_type = 'memory' AND target_id IN (\
                SELECT target_id FROM cluster_members WHERE target_type = 'memory'\
             )",
            [],
        )
        .unwrap_or(0);

    count += conn
        .execute(
            "DELETE FROM embeddings WHERE target_type = 'decision' AND target_id IN (\
                SELECT target_id FROM cluster_members WHERE target_type = 'decision'\
             )",
            [],
        )
        .unwrap_or(0);

    count
}

// ─── Feedback aggregation ───────────────────────────────────────────────────

/// Compact old individual feedback signals into per-source aggregates.
/// Before: 50 rows for "memory::foo" with signal 1.0, -0.5, 1.0, ...
/// After:  1 row for "memory::foo" with signal = net_sum, query_text = "[aggregated]"
fn aggregate_old_feedback(conn: &Connection) -> usize {
    // Find sources with old feedback to aggregate
    let sources: Vec<(String, f64, i64)> = conn
        .prepare(
            "SELECT result_source, SUM(signal), COUNT(*) \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) > ?1 \
             GROUP BY result_source HAVING COUNT(*) > 1",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![FEEDBACK_AGGREGATION_DAYS], |row| {
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
        // Delete old individual rows
        let deleted = conn
            .execute(
                "DELETE FROM recall_feedback \
                 WHERE result_source = ?1 \
                 AND julianday('now') - julianday(created_at) > ?2",
                params![source, FEEDBACK_AGGREGATION_DAYS],
            )
            .unwrap_or(0);

        // Insert one aggregated row
        if deleted > 0 {
            let _ = conn.execute(
                "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent, created_at) \
                 VALUES ('[aggregated]', ?1, 'aggregated', ?2, 'compaction', datetime('now'))",
                params![source, net_signal],
            );
            aggregated += deleted;
        }
    }

    aggregated
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn db_size_bytes(conn: &Connection) -> i64 {
    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .unwrap_or(0);
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |row| row.get(0))
        .unwrap_or(4096);
    page_count * page_size
}

fn freelist_count(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA freelist_count", [], |row| row.get(0))
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

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);
        conn
    }

    #[test]
    fn test_prune_old_events() {
        let conn = setup();
        // Insert an old event
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('test', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('agent_boot', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        // Insert a recent event
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('test', '{}', 'test')",
            [],
        )
        .unwrap();

        let pruned = prune_old_events(&conn);
        assert_eq!(pruned, 1, "Should prune only the old event");

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn test_strip_archived_text() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memories (text, source, status, updated_at) \
             VALUES ('important data', 'test', 'archived', datetime('now', '-120 days'))",
            [],
        )
        .unwrap();

        let stripped = strip_archived_text(&conn);
        assert_eq!(stripped, 1);

        let text: String = conn
            .query_row("SELECT text FROM memories WHERE source = 'test'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(text, "[compacted]");
    }

    #[test]
    fn test_compaction_full_pass() {
        let conn = setup();
        let result = run_compaction(&conn);
        // Empty DB should compact cleanly
        assert_eq!(result.events_pruned, 0);
        assert_eq!(result.archived_text_stripped, 0);
        assert_eq!(result.expired_pruned, 0);
    }

    #[test]
    fn test_storage_breakdown() {
        let conn = setup();
        let breakdown = storage_breakdown(&conn);
        assert!(!breakdown.is_empty());
        // All counts should be 0 for empty DB
        assert!(breakdown.iter().all(|(_, count)| *count == 0));
    }

    #[test]
    fn test_prune_expired_entries() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memories (text, source, status, expires_at) VALUES ('expired memory', 'ttl::mem', 'active', datetime('now', '-1 second'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at) VALUES ('expired decision', 'ttl::dec', 'active', datetime('now', '-1 second'))",
            [],
        )
        .unwrap();

        let deleted = prune_expired_entries(&conn);
        assert_eq!(deleted, 2);

        let mem_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE source = 'ttl::mem'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let dec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context = 'ttl::dec'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 0);
        assert_eq!(dec_count, 0);

        let event: (String, String) = conn
            .query_row(
                "SELECT type, data FROM events WHERE source_agent = 'compaction' ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(event.0, "expired_entries_pruned");
        assert!(event.1.contains("\"memories_deleted\":1"));
        assert!(event.1.contains("\"decisions_deleted\":1"));
    }
}
