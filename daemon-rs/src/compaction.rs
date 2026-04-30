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

/// Raw boot savings rows older than this are compacted into a single rollup row.
/// The dashboard only needs recent raw points, while all-time totals are preserved
/// via `boot_savings_rollup`.
const BOOT_SAVINGS_RETENTION_DAYS: i64 = 45;

/// Only VACUUM when SQLite reports enough reclaimable pages to justify the IO.
const VACUUM_FREELIST_THRESHOLD_PAGES: i64 = 100;

/// Archived entries older than this have their text stripped (metadata kept).
const ARCHIVED_TEXT_RETENTION_DAYS: i64 = 90;

/// Feedback signals older than this are aggregated into summaries.
const FEEDBACK_AGGREGATION_DAYS: i64 = 60;

/// Roll analytics-heavy savings events older than this into compact hourly rows.
const SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 7;
/// Keep rolled-up savings analytics bounded; /savings only reads the recent window.
const EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 120;

/// Elevated-pressure storage governor soft limit (no hard failures, compaction only).
pub const STORAGE_SOFT_LIMIT_BYTES: i64 = 256 * 1024 * 1024; // 256MB
/// Critical-pressure storage governor hard limit (triggers aggressive safe compaction).
pub const STORAGE_HARD_LIMIT_BYTES: i64 = 512 * 1024 * 1024; // 512MB

/// Under critical pressure, compact events more aggressively.
const AGGRESSIVE_EVENT_RETENTION_DAYS: i64 = 3;
/// Under critical pressure, compact boot savings history more aggressively.
const AGGRESSIVE_BOOT_SAVINGS_RETENTION_DAYS: i64 = 14;
/// Under critical pressure, compact archived text sooner.
const AGGRESSIVE_ARCHIVED_TEXT_RETENTION_DAYS: i64 = 30;
/// Under critical pressure, aggregate feedback sooner.
const AGGRESSIVE_FEEDBACK_AGGREGATION_DAYS: i64 = 14;
/// Under critical pressure, roll savings events even sooner.
const AGGRESSIVE_SAVINGS_EVENT_ROLLUP_RETENTION_DAYS: i64 = 2;
/// Under critical pressure, keep a shorter event rollup history to reclaim space faster.
const AGGRESSIVE_EVENT_SAVINGS_ROLLUP_RETENTION_DAYS: i64 = 45;
/// Keep benchmark artifacts only briefly in production databases.
const BENCHMARK_RETENTION_DAYS: i64 = 2;
/// Tighten benchmark retention further under critical pressure.
const AGGRESSIVE_BENCHMARK_RETENTION_DAYS: i64 = 1;

/// Canonical source-agent prefix emitted by benchmark harnesses.
///
/// Keep this broad enough to match both modern namespaced agents
/// (`amb-cortex::<run>`) and legacy plain labels (`amb-cortex`).
pub const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex";

/// Non-boot event volume triggers compaction even when DB file size is moderate.
pub const EVENT_NONBOOT_SOFT_LIMIT_ROWS: i64 = 72_000;
/// Critical non-boot event pressure threshold.
pub const EVENT_NONBOOT_HARD_LIMIT_ROWS: i64 = 120_000;
/// Keep newest non-boot rows at or under this level during normal governor runs.
const EVENT_NONBOOT_SOFT_KEEP_ROWS: i64 = 52_000;
/// Keep newest non-boot rows at or under this level during critical pressure runs.
const EVENT_NONBOOT_HARD_KEEP_ROWS: i64 = 28_000;
/// Startup governor mode should avoid single huge DELETE statements that hold
/// the write lock for too long while the daemon is still coming online.
const STARTUP_EVENT_PRUNE_BATCH_ROWS: i64 = 8_000;

/// Per-event-type row caps to prevent high-frequency streams from dominating storage.
const EVENT_TYPE_SOFT_CAPS: &[(&str, i64)] = &[
    ("agent_boot", 4_000),
    ("boot_savings", 6_000),
    ("store_savings", 10_000),
    ("tool_call_savings", 10_000),
    ("decision_stored", 18_000),
    ("decision_supersede", 10_000),
    ("decision_refine_pending", 10_000),
    ("decision_agreement_merge", 8_000),
    ("decision_truncated", 8_000),
    ("recall_query", 14_000),
    ("merge", 6_000),
    ("decision_conflict", 6_000),
    ("decision_rejected_duplicate", 6_000),
    ("decision_resolve", 6_000),
    ("forget", 3_000),
    ("diary_write", 3_000),
];

/// More aggressive caps used under critical pressure.
const EVENT_TYPE_HARD_CAPS: &[(&str, i64)] = &[
    ("agent_boot", 1_500),
    ("boot_savings", 2_500),
    ("store_savings", 4_000),
    ("tool_call_savings", 4_000),
    ("decision_stored", 8_000),
    ("decision_supersede", 4_000),
    ("decision_refine_pending", 4_000),
    ("decision_agreement_merge", 3_000),
    ("decision_truncated", 3_000),
    ("recall_query", 6_000),
    ("merge", 2_000),
    ("decision_conflict", 2_000),
    ("decision_rejected_duplicate", 2_000),
    ("decision_resolve", 2_000),
    ("forget", 1_000),
    ("diary_write", 1_000),
];

// ─── Result ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct CompactionResult {
    pub events_pruned: usize,
    pub benchmark_pruned: usize,
    pub archived_text_stripped: usize,
    pub expired_pruned: usize,
    pub crystal_embeddings_pruned: usize,
    pub cluster_members_pruned: usize,
    pub feedback_aggregated: usize,
    pub stale_embeddings_pruned: usize,
    pub co_occurrence_pruned: usize,
    pub fts_optimized: bool,
    pub bytes_before: i64,
    pub bytes_after: i64,
}

#[derive(Debug, Default)]
pub struct BenchmarkPurgeResult {
    pub decisions_deleted: usize,
    pub embeddings_deleted: usize,
    pub cluster_members_deleted: usize,
    pub decision_conflicts_deleted: usize,
    pub recall_feedback_deleted: usize,
    pub co_occurrence_deleted: usize,
    pub events_deleted: usize,
    pub bytes_before: i64,
    pub bytes_after: i64,
}

impl BenchmarkPurgeResult {
    pub fn total_deleted(&self) -> usize {
        self.decisions_deleted
            + self.embeddings_deleted
            + self.cluster_members_deleted
            + self.decision_conflicts_deleted
            + self.recall_feedback_deleted
            + self.co_occurrence_deleted
            + self.events_deleted
    }
}

fn bytes_to_mb(bytes: i64) -> i64 {
    bytes / (1024 * 1024)
}

/// Classify current storage pressure based on DB size.
/// This is advisory only; Cortex should compact automatically, not reject writes.
pub fn classify_storage_pressure(db_size_bytes: i64) -> &'static str {
    if db_size_bytes >= STORAGE_HARD_LIMIT_BYTES {
        "critical"
    } else if db_size_bytes >= STORAGE_SOFT_LIMIT_BYTES {
        "elevated"
    } else {
        "normal"
    }
}

/// Classify non-boot event pressure so callers can explain when compaction is needed.
pub fn classify_event_pressure(nonboot_event_rows: i64) -> &'static str {
    if nonboot_event_rows >= EVENT_NONBOOT_HARD_LIMIT_ROWS {
        "critical"
    } else if nonboot_event_rows >= EVENT_NONBOOT_SOFT_LIMIT_ROWS {
        "elevated"
    } else {
        "normal"
    }
}

/// FTS5 segment row count above this triggers the governor even when the
/// overall DB size is well under soft limit. Without this, FTS shadow tables
/// can balloon to hundreds of MB before the size-based trigger fires.
pub const FTS_SEGMENT_ROW_SOFT_LIMIT: i64 = 10_000;

/// Decide whether the storage governor should run compaction.
/// Runs when DB size is above soft limit or when reclaimable free pages are high.
#[cfg_attr(not(test), allow(dead_code))]
pub fn should_run_compaction_governor(db_size_bytes: i64, freelist_pages: i64) -> bool {
    should_run_compaction_governor_with_pressure(db_size_bytes, freelist_pages, 0, 0)
}

fn should_run_compaction_governor_with_pressure(
    db_size_bytes: i64,
    freelist_pages: i64,
    nonboot_event_rows: i64,
    fts_segment_rows: i64,
) -> bool {
    db_size_bytes >= STORAGE_SOFT_LIMIT_BYTES
        || freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES
        || nonboot_event_rows > EVENT_NONBOOT_SOFT_LIMIT_ROWS
        || fts_segment_rows > FTS_SEGMENT_ROW_SOFT_LIMIT
}

/// Sum of rows across all known FTS5 _data shadow tables. The _data table
/// holds one row per FTS5 segment block; runaway segment counts are the
/// dominant bloat driver in long-lived Cortex DBs.
pub fn fts_segment_row_total(conn: &Connection) -> i64 {
    let tables = ["decisions_fts_data", "memories_fts_data"];
    let mut total: i64 = 0;
    for table in tables {
        if !table_exists(conn, table) {
            continue;
        }
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        total += n;
    }
    total
}

/// Run compaction only when pressure or reclaimable space justifies IO.
/// Returns `Some(result)` when a compaction pass ran, `None` when skipped.
pub fn run_compaction_governor(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, true)
}

/// Startup-safe governor mode that relieves event pressure without forcing VACUUM.
/// This keeps startup/early-runtime lock windows shorter while still enforcing
/// retention and event-cap policies.
pub fn run_compaction_governor_startup(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, false)
}

fn run_compaction_governor_with_options(
    conn: &Connection,
    allow_vacuum: bool,
) -> Option<CompactionResult> {
    let startup_prune_limit = (!allow_vacuum).then_some(STARTUP_EVENT_PRUNE_BATCH_ROWS);
    let before = db_size_bytes(conn);
    let freelist_pages = freelist_count(conn);
    let nonboot_event_rows_before = non_boot_event_count(conn);
    let fts_segment_rows_before = fts_segment_row_total(conn);
    let pressure_before = classify_storage_pressure(before);

    if !should_run_compaction_governor_with_pressure(
        before,
        freelist_pages,
        nonboot_event_rows_before,
        fts_segment_rows_before,
    ) {
        return None;
    }

    let mut result = run_compaction_with_options(conn, allow_vacuum);

    // Critical pressure gets an additional safe-aggressive pass. We still only touch:
    // old events, archived text, and aged feedback (never active memory content).
    if before >= STORAGE_HARD_LIMIT_BYTES
        || nonboot_event_rows_before >= EVENT_NONBOOT_HARD_LIMIT_ROWS
    {
        result.events_pruned +=
            rollup_old_boot_savings_with_retention(conn, AGGRESSIVE_BOOT_SAVINGS_RETENTION_DAYS);
        result.events_pruned +=
            rollup_old_savings_events(conn, AGGRESSIVE_SAVINGS_EVENT_ROLLUP_RETENTION_DAYS);
        result.events_pruned +=
            prune_old_event_savings_rollups(conn, AGGRESSIVE_EVENT_SAVINGS_ROLLUP_RETENTION_DAYS);
        result.events_pruned += prune_old_events_with_retention_limit(
            conn,
            AGGRESSIVE_EVENT_RETENTION_DAYS,
            startup_prune_limit,
        );
        result.events_pruned +=
            prune_event_type_caps_with_limit(conn, EVENT_TYPE_HARD_CAPS, startup_prune_limit);
        result.events_pruned += prune_nonboot_event_overflow_with_limit(
            conn,
            EVENT_NONBOOT_HARD_KEEP_ROWS,
            startup_prune_limit,
        );
        result.benchmark_pruned +=
            prune_old_benchmark_artifacts(conn, AGGRESSIVE_BENCHMARK_RETENTION_DAYS, allow_vacuum);
        result.archived_text_stripped +=
            strip_archived_text_with_retention(conn, AGGRESSIVE_ARCHIVED_TEXT_RETENTION_DAYS);
        result.cluster_members_pruned += prune_orphan_cluster_members(conn);
        result.feedback_aggregated +=
            aggregate_old_feedback_with_window(conn, AGGRESSIVE_FEEDBACK_AGGREGATION_DAYS);
        let _ = if allow_vacuum {
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
        } else {
            conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
        };
        result.bytes_after = db_size_bytes(conn);
    }

    let pressure_after = classify_storage_pressure(result.bytes_after);
    let fts_segment_rows_after = fts_segment_row_total(conn);
    eprintln!(
        "[compaction] governor: pressure {} -> {}, size {}MB -> {}MB, nonboot_events {} -> {}, fts_segments {} -> {}",
        pressure_before,
        pressure_after,
        bytes_to_mb(result.bytes_before),
        bytes_to_mb(result.bytes_after),
        nonboot_event_rows_before,
        non_boot_event_count(conn),
        fts_segment_rows_before,
        fts_segment_rows_after,
    );

    Some(result)
}

// ─── Main entry point ───────────────────────────────────────────────────────

/// Run one compaction pass. Safe to call repeatedly.
pub fn run_compaction(conn: &Connection) -> CompactionResult {
    run_compaction_with_options(conn, true)
}

fn run_compaction_with_options(conn: &Connection, allow_vacuum: bool) -> CompactionResult {
    let startup_prune_limit = (!allow_vacuum).then_some(STARTUP_EVENT_PRUNE_BATCH_ROWS);
    let mut result = CompactionResult {
        bytes_before: db_size_bytes(conn),
        ..CompactionResult::default()
    };

    // 1. Event log rotation
    result.events_pruned = rollup_old_boot_savings(conn);
    result.events_pruned += rollup_old_savings_events(conn, SAVINGS_EVENT_ROLLUP_RETENTION_DAYS);
    result.events_pruned +=
        prune_old_event_savings_rollups(conn, EVENT_SAVINGS_ROLLUP_RETENTION_DAYS);
    result.events_pruned +=
        prune_old_events_with_retention_limit(conn, EVENT_RETENTION_DAYS, startup_prune_limit);
    result.events_pruned +=
        prune_event_type_caps_with_limit(conn, EVENT_TYPE_SOFT_CAPS, startup_prune_limit);
    result.events_pruned += prune_nonboot_event_overflow_with_limit(
        conn,
        EVENT_NONBOOT_SOFT_KEEP_ROWS,
        startup_prune_limit,
    );
    result.benchmark_pruned =
        prune_old_benchmark_artifacts(conn, BENCHMARK_RETENTION_DAYS, allow_vacuum);

    // 2. Archived entry text cleanup
    result.archived_text_stripped = strip_archived_text(conn);

    // 3. Hard-expiration cleanup
    result.expired_pruned = prune_expired_entries(conn);

    // 4. Crystal member embedding pruning
    result.crystal_embeddings_pruned = prune_crystal_member_embeddings(conn);
    result.cluster_members_pruned = prune_orphan_cluster_members(conn);

    // 5. Feedback aggregation
    result.feedback_aggregated = aggregate_old_feedback(conn);

    // 6. Stale-model embedding pruning. The active embedding model can change
    // (we just switched the default to BGE); embeddings tagged with retired
    // model keys cannot serve any current recall and only exist to satisfy a
    // potential future re-embed. Once the active model has good coverage we
    // prune the rest. ~30 bytes saved per row × thousands of stale rows.
    result.stale_embeddings_pruned = prune_stale_embeddings(conn);

    // 7. Sparse co-occurrence pruning. Pairs seen exactly once are noise that
    // never influence recall; the table is one of the largest by row count.
    result.co_occurrence_pruned = prune_singleton_co_occurrence(conn);

    // 8. FTS5 segment optimize. Without this the contentless FTS shadow tables
    // accumulate one segment per write — for our DB that bloated
    // `decisions_fts_data` to >300MB despite only ~640 source rows. Running
    // FTS5 'optimize' merges all segments into one, recovering the bulk of
    // the file size. Cheap on small N, expensive on huge N — but our N is
    // small in absolute terms; the bloat is in the segment overhead.
    result.fts_optimized = optimize_fts_indexes(conn);

    // 9. Reclaim space
    checkpoint_after_compaction(conn, allow_vacuum);
    // VACUUM is expensive. Use SQLite's freelist_count instead of raw delete
    // volume so we only pay the cost when pages are actually reclaimable.
    let freelist_pages = freelist_count(conn);
    let total_deleted = result.events_pruned
        + result.benchmark_pruned
        + result.archived_text_stripped
        + result.expired_pruned
        + result.crystal_embeddings_pruned
        + result.feedback_aggregated
        + result.stale_embeddings_pruned
        + result.co_occurrence_pruned;
    if allow_vacuum
        && (freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES || result.fts_optimized)
    {
        let _ = conn.execute_batch("VACUUM;");
    }

    result.bytes_after = db_size_bytes(conn);

    if total_deleted > 0 || result.fts_optimized {
        let saved_kb = (result.bytes_before - result.bytes_after) / 1024;
        eprintln!(
            "[compaction] Pruned: {} events, {} benchmark rows, {} archived texts, {} expired rows, {} crystal embeddings, {} orphan cluster members, {} feedback rows, {} stale embeddings, {} singleton co-occurrence pairs; fts_optimized={}. Saved {}KB",
            result.events_pruned,
            result.benchmark_pruned,
            result.archived_text_stripped,
            result.expired_pruned,
            result.crystal_embeddings_pruned,
            result.cluster_members_pruned,
            result.feedback_aggregated,
            result.stale_embeddings_pruned,
            result.co_occurrence_pruned,
            result.fts_optimized,
            saved_kb
        );
    }

    result
}

/// Run FTS5 'optimize' on every contentless FTS shadow table. This collapses
/// the per-write segment list into a single merged segment, recovering the
/// dominant share of bytes in heavily-used databases. Returns true iff at
/// least one table was optimized successfully.
fn optimize_fts_indexes(conn: &Connection) -> bool {
    let tables = ["decisions_fts", "memories_fts"];
    let mut any = false;
    for table in tables {
        if !table_exists(conn, table) {
            continue;
        }
        // FTS5 optimize is invoked via a no-op insert with a special command
        // payload. Errors here should not abort the whole compaction pass.
        let sql = format!("INSERT INTO {table}({table}) VALUES ('optimize')");
        match conn.execute_batch(&sql) {
            Ok(()) => {
                any = true;
            }
            Err(err) => {
                eprintln!("[compaction] FTS optimize failed for {table}: {err}");
            }
        }
    }
    any
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        params![name],
        |_| Ok(()),
    )
    .is_ok()
}

/// Delete embeddings whose `model` column does not match the currently
/// selected embedding model. Stale-model rows cannot satisfy any active
/// recall and only persist to support potential re-embeds; once the active
/// model has produced coverage the legacy rows are pure dead weight.
///
/// Comparison is case-insensitive: legacy rows in the wild use mixed casings
/// of the same model key ("all-MiniLM-L6-v2" vs "all-minilm-l6-v2"). NULL
/// model rows are also pruned — they predate model tagging entirely and have
/// no way to match any current model.
fn prune_stale_embeddings(conn: &Connection) -> usize {
    let active = crate::embeddings::selected_model_key().to_ascii_lowercase();
    // Guardrail: only prune if the active model has at least some coverage.
    // Otherwise we'd torch every embedding on a fresh model switch before the
    // backfill has a chance to populate replacements.
    let active_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embeddings WHERE LOWER(model) = ?1",
            params![active],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if active_count < 50 {
        return 0;
    }
    conn.execute(
        "DELETE FROM embeddings WHERE model IS NULL OR LOWER(model) != ?1",
        params![active],
    )
    .unwrap_or(0)
}

/// Delete co-occurrence pairs that have only ever been observed once. They
/// contribute no signal to ranking and dominate the row count; for our DB the
/// pruned set is typically >50% of the table.
fn prune_singleton_co_occurrence(conn: &Connection) -> usize {
    if !table_exists(conn, "co_occurrence") {
        return 0;
    }
    conn.execute("DELETE FROM co_occurrence WHERE \"count\" <= 1", [])
        .unwrap_or(0)
}

/// Purge all benchmark artifacts immediately.
/// Use this after benchmark runs so production DB stats reflect real-user traffic.
pub fn purge_benchmark_artifacts(conn: &Connection) -> BenchmarkPurgeResult {
    purge_benchmark_artifacts_with_retention(conn, None, true)
}

// ─── Event log rotation ─────────────────────────────────────────────────────

fn rollup_old_boot_savings(conn: &Connection) -> usize {
    rollup_old_boot_savings_with_retention(conn, BOOT_SAVINGS_RETENTION_DAYS)
}

fn rollup_old_boot_savings_with_retention(conn: &Connection, retention_days: i64) -> usize {
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

    let deleted_old = conn
        .execute(
            "DELETE FROM events \
             WHERE type = 'boot_savings' \
               AND created_at < datetime('now', ?1) \
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
            params![retention_window, benchmark_source_pattern],
        )
        .unwrap_or(0);

    let deleted_rollups = conn
        .execute("DELETE FROM events WHERE type = 'boot_savings_rollup'", [])
        .unwrap_or(0);

    if merged_boots > 0 {
        let payload = serde_json::json!({
            "saved": merged_saved,
            "served": merged_served,
            "baseline": merged_baseline,
            "boots": merged_boots,
            "retention_days": retention_days,
            "rolled_up_at": chrono::Utc::now().to_rfc3339(),
        })
        .to_string();
        let _ = conn.execute(
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

fn rollup_old_savings_events(conn: &Connection, retention_days: i64) -> usize {
    let retention_window = format!("-{retention_days} days");
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    type SavingsRollupRow = (String, i64, String, i64, i64, i64, i64, i64, i64);
    let rollup_rows: Vec<SavingsRollupRow> = conn
        .prepare(
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
              GROUP BY day, hour, operation",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(
                params![retention_window.clone(), benchmark_source_pattern.clone()],
                |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            },
            )?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();

    if rollup_rows.is_empty() {
        return 0;
    }

    for (day, hour, operation, saved, served, baseline, events, hits, misses) in rollup_rows {
        let _ = conn.execute(
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

    conn.execute(
        "DELETE FROM events \
         WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') \
           AND created_at IS NOT NULL \
           AND created_at < datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
        params![retention_window, benchmark_source_pattern],
    )
    .unwrap_or(0)
}

fn prune_old_event_savings_rollups(conn: &Connection, retention_days: i64) -> usize {
    conn.execute(
        "DELETE FROM event_savings_rollups \
         WHERE day < date('now', ?1)",
        params![format!("-{retention_days} days")],
    )
    .unwrap_or(0)
}

#[cfg(test)]
fn prune_old_events(conn: &Connection) -> usize {
    prune_old_events_with_retention_limit(conn, EVENT_RETENTION_DAYS, None)
}

#[cfg(test)]
fn prune_old_events_with_retention(conn: &Connection, retention_days: i64) -> usize {
    prune_old_events_with_retention_limit(conn, retention_days, None)
}

fn prune_old_events_with_retention_limit(
    conn: &Connection,
    retention_days: i64,
    max_delete_rows: Option<i64>,
) -> usize {
    let retention_window = format!("-{retention_days} days");
    if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
        return conn
            .execute(
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
            )
            .unwrap_or(0);
    }
    conn.execute(
        "DELETE FROM events \
         WHERE type NOT IN ('boot_savings', 'boot_savings_rollup') \
           AND (created_at IS NULL OR TRIM(created_at) = '' OR created_at < datetime('now', ?1))",
        params![retention_window],
    )
    .unwrap_or(0)
}

#[cfg(test)]
fn prune_event_type_caps(conn: &Connection, caps: &[(&str, i64)]) -> usize {
    prune_event_type_caps_with_limit(conn, caps, None)
}

fn prune_event_type_caps_with_limit(
    conn: &Connection,
    caps: &[(&str, i64)],
    max_delete_rows: Option<i64>,
) -> usize {
    let mut total = 0usize;
    for (event_type, keep_rows) in caps.iter().copied() {
        if keep_rows <= 0 {
            continue;
        }
        let deleted = if let Some(max_rows) = max_delete_rows.filter(|rows| *rows > 0) {
            conn.execute(
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
            .unwrap_or(0)
        } else {
            conn.execute(
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
            .unwrap_or(0)
        };
        total += deleted;
    }
    total
}

#[cfg(test)]
fn prune_nonboot_event_overflow(conn: &Connection, keep_rows: i64) -> usize {
    prune_nonboot_event_overflow_with_limit(conn, keep_rows, None)
}

fn prune_nonboot_event_overflow_with_limit(
    conn: &Connection,
    keep_rows: i64,
    max_delete_rows: Option<i64>,
) -> usize {
    if keep_rows <= 0 {
        return 0;
    }
    // Keep recall/store/tool savings events out of global overflow pruning so
    // /savings can rely on their short-horizon raw rows until rollup runs.
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
        return conn
            .execute(
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
            )
            .unwrap_or(0);
    }
    conn.execute(
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
    .unwrap_or(0)
}

fn checkpoint_after_compaction(conn: &Connection, allow_vacuum: bool) {
    let _ = if allow_vacuum {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
    } else {
        // Startup governor mode: avoid TRUNCATE stalls while still nudging WAL forward.
        conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")
    };
}

// ─── Archived entry text cleanup ────────────────────────────────────────────

/// Strip full text from archived entries older than retention period.
/// Keeps: id, source, type, status, created_at, score (for audit).
/// Drops: text, compressed_text, tags, context (saves space).
fn strip_archived_text(conn: &Connection) -> usize {
    strip_archived_text_with_retention(conn, ARCHIVED_TEXT_RETENTION_DAYS)
}

fn strip_archived_text_with_retention(conn: &Connection, retention_days: i64) -> usize {
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

fn prune_orphan_cluster_members(conn: &Connection) -> usize {
    let mut count = 0usize;
    count += conn
        .execute(
            "DELETE FROM cluster_members \
             WHERE target_type = 'memory' \
               AND NOT EXISTS (SELECT 1 FROM memories WHERE memories.id = cluster_members.target_id)",
            [],
        )
        .unwrap_or(0);
    count += conn
        .execute(
            "DELETE FROM cluster_members \
             WHERE target_type = 'decision' \
               AND NOT EXISTS (SELECT 1 FROM decisions WHERE decisions.id = cluster_members.target_id)",
            [],
        )
        .unwrap_or(0);
    count += conn
        .execute(
            "DELETE FROM cluster_members \
             WHERE target_type NOT IN ('memory', 'decision')",
            [],
        )
        .unwrap_or(0);
    count += conn
        .execute(
            "DELETE FROM cluster_members \
             WHERE NOT EXISTS (SELECT 1 FROM memory_clusters WHERE memory_clusters.id = cluster_members.cluster_id)",
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
    aggregate_old_feedback_with_window(conn, FEEDBACK_AGGREGATION_DAYS)
}

fn aggregate_old_feedback_with_window(conn: &Connection, aggregation_days: i64) -> usize {
    // Find sources with old feedback to aggregate
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
        // Delete old individual rows
        let deleted = conn
            .execute(
                "DELETE FROM recall_feedback \
                 WHERE result_source = ?1 \
                 AND julianday('now') - julianday(created_at) > ?2",
                params![source, aggregation_days],
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

fn prune_old_benchmark_artifacts(
    conn: &Connection,
    retention_days: i64,
    allow_vacuum: bool,
) -> usize {
    purge_benchmark_artifacts_with_retention(conn, Some(retention_days), allow_vacuum)
        .total_deleted()
}

fn purge_benchmark_artifacts_with_retention(
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

    let _ = conn.execute_batch(
        "DROP TABLE IF EXISTS temp._benchmark_decision_ids;
         CREATE TEMP TABLE IF NOT EXISTS _benchmark_decision_ids (
           id INTEGER PRIMARY KEY
         );
         DELETE FROM _benchmark_decision_ids;",
    );

    match retention_window.as_deref() {
        Some(window) => {
            let _ = conn.execute(
                "INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE (LOWER(COALESCE(type, '')) = 'benchmark' \
                        OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)) \
                   AND created_at < datetime('now', ?2)",
                params![benchmark_source_pattern.clone(), window],
            );
        }
        None => {
            let _ = conn.execute(
                "INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE LOWER(COALESCE(type, '')) = 'benchmark' \
                    OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)",
                params![benchmark_source_pattern.clone()],
            );
        }
    }

    result.decision_conflicts_deleted = conn
        .execute(
            "DELETE FROM decision_conflicts \
             WHERE source_decision_id IN (SELECT id FROM _benchmark_decision_ids) \
                OR target_decision_id IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    result.embeddings_deleted = conn
        .execute(
            "DELETE FROM embeddings \
             WHERE target_type = 'decision' \
               AND target_id IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    result.cluster_members_deleted = conn
        .execute(
            "DELETE FROM cluster_members \
             WHERE target_type = 'decision' \
               AND target_id IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);
    result.cluster_members_deleted += prune_orphan_cluster_members(conn);

    result.recall_feedback_deleted = conn
        .execute(
            "DELETE FROM recall_feedback \
             WHERE result_source IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
                OR result_id IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    result.co_occurrence_deleted = conn
        .execute(
            "DELETE FROM co_occurrence \
             WHERE source_a IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
                OR source_b IN (SELECT 'decision::' || id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    result.decisions_deleted = conn
        .execute(
            "DELETE FROM decisions WHERE id IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    result.events_deleted += conn
        .execute(
            "DELETE FROM events \
             WHERE type = 'decision_stored' \
               AND CAST(COALESCE(json_extract(data, '$.id'), 0) AS INTEGER) IN (SELECT id FROM _benchmark_decision_ids)",
            [],
        )
        .unwrap_or(0);

    match retention_window.as_deref() {
        Some(window) => {
            result.recall_feedback_deleted += conn
                .execute(
                    "DELETE FROM recall_feedback \
                     WHERE (LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)) \
                       AND created_at < datetime('now', ?2)",
                    params![benchmark_source_pattern.clone(), window],
                )
                .unwrap_or(0);
            result.events_deleted += conn
                .execute(
                    "DELETE FROM events \
                     WHERE (LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark') \
                       AND created_at < datetime('now', ?2)",
                    params![benchmark_source_pattern.clone(), window],
                )
                .unwrap_or(0);
        }
        None => {
            result.recall_feedback_deleted += conn
                .execute(
                    "DELETE FROM recall_feedback \
                     WHERE LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)",
                    params![benchmark_source_pattern.clone()],
                )
                .unwrap_or(0);
            result.events_deleted += conn
                .execute(
                    "DELETE FROM events \
                     WHERE LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark'",
                    params![benchmark_source_pattern.clone()],
                )
                .unwrap_or(0);
        }
    }

    let _ = conn.execute_batch("DROP TABLE IF EXISTS temp._benchmark_decision_ids;");
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    if allow_vacuum {
        let freelist_pages = freelist_count(conn);
        if freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES {
            let _ = conn.execute_batch("VACUUM;");
        }
    }
    result.bytes_after = db_size_bytes(conn);
    result
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
        assert_eq!(
            pruned, 2,
            "Should prune old non-savings events, including stale agent_boot rows"
        );

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
    }

    #[test]
    fn test_rollup_old_boot_savings_compacts_history_and_keeps_recent_rows() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'test', datetime('now', '-60 days'))",
            params![r#"{"saved":100,"served":50,"baseline":150}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'test', datetime('now'))",
            params![r#"{"saved":20,"served":10,"baseline":30}"#],
        )
        .unwrap();

        let pruned = rollup_old_boot_savings_with_retention(&conn, 30);
        assert_eq!(pruned, 1);

        let raw_boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            raw_boot_rows, 1,
            "recent raw boot_savings row should remain"
        );

        let rollup: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT \
                    COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.boots') AS INTEGER), 0) \
                 FROM events WHERE type = 'boot_savings_rollup' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(rollup, (100, 50, 150, 1));
    }

    #[test]
    fn test_rollup_old_boot_savings_excludes_benchmark_agent_payloads() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-60 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 999,
                "served": 1,
                "baseline": 1000
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-60 days'))",
            params![serde_json::json!({
                "agent": "codex",
                "saved": 50,
                "served": 10,
                "baseline": 60
            })
            .to_string()],
        )
        .unwrap();

        let pruned = rollup_old_boot_savings_with_retention(&conn, 30);
        assert_eq!(
            pruned, 1,
            "only non-benchmark boot_savings rows should roll up"
        );

        let rollup_saved: i64 = conn
            .query_row(
                "SELECT COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 FROM events WHERE type = 'boot_savings_rollup' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rollup_saved, 50);

        let benchmark_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE type = 'boot_savings' \
                   AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(benchmark_rows, 1);
    }

    #[test]
    fn test_prune_old_events_keeps_boot_rollups() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings_rollup', ?1, 'compaction', datetime('now', '-90 days'))",
            params![r#"{"saved":1000,"served":500,"baseline":1500,"boots":10}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('decision_stored', '{}', 'test', datetime('now', '-90 days'))",
            [],
        )
        .unwrap();

        let pruned = prune_old_events_with_retention(&conn, 30);
        assert_eq!(
            pruned, 1,
            "only non-rollup historical events should be pruned"
        );

        let rollup_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings_rollup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rollup_rows, 1, "boot_savings_rollup must be retained");
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
    fn test_strip_archived_text_compacts_superseded_decisions() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, updated_at) \
             VALUES ('superseded text', 'old context', 'superseded', datetime('now', '-120 days'))",
            [],
        )
        .unwrap();

        let stripped = strip_archived_text(&conn);
        assert_eq!(stripped, 1);

        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE status = 'superseded' LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "[compacted]");
        assert!(row.1.is_none());
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
    fn test_storage_pressure_classification() {
        assert_eq!(
            classify_storage_pressure(STORAGE_SOFT_LIMIT_BYTES - 1),
            "normal"
        );
        assert_eq!(
            classify_storage_pressure(STORAGE_SOFT_LIMIT_BYTES),
            "elevated"
        );
        assert_eq!(
            classify_storage_pressure(STORAGE_HARD_LIMIT_BYTES),
            "critical"
        );
    }

    #[test]
    fn test_event_pressure_classification() {
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_SOFT_LIMIT_ROWS - 1),
            "normal"
        );
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_SOFT_LIMIT_ROWS),
            "elevated"
        );
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_HARD_LIMIT_ROWS),
            "critical"
        );
    }

    #[test]
    fn test_storage_governor_thresholds() {
        assert!(!should_run_compaction_governor(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES
        ));
        assert!(should_run_compaction_governor(STORAGE_SOFT_LIMIT_BYTES, 0));
        assert!(should_run_compaction_governor(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES + 1
        ));
        assert!(should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            EVENT_NONBOOT_SOFT_LIMIT_ROWS + 1,
            0,
        ));
    }

    #[test]
    fn test_governor_triggers_on_fts_segment_pressure() {
        // No size pressure, no freelist pressure, no event pressure — but FTS
        // segment count above the soft limit MUST still trigger the governor,
        // because that is the bloat dimension a healthy DB can hide.
        assert!(!should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            0,
            FTS_SEGMENT_ROW_SOFT_LIMIT,
        ));
        assert!(should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            0,
            FTS_SEGMENT_ROW_SOFT_LIMIT + 1,
        ));
    }

    #[test]
    fn test_fts_segment_row_total_counts_known_tables() {
        let conn = setup();
        // Fresh schema: FTS shadow tables exist but should be near-empty.
        let baseline = fts_segment_row_total(&conn);
        // Force several inserts + updates to grow segments.
        for i in 0..50 {
            conn.execute(
                "INSERT INTO decisions (decision, context, type, source_agent, status) \
                 VALUES (?1, 'ctx', 'decision', 'test', 'active')",
                params![format!("decision-{i} alpha beta gamma delta epsilon")],
            )
            .unwrap();
        }
        let after_inserts = fts_segment_row_total(&conn);
        assert!(
            after_inserts > baseline,
            "decisions inserts should bump fts_segment_row_total ({baseline} -> {after_inserts})"
        );
    }

    #[test]
    fn test_fts_optimize_drops_segment_rows() {
        let conn = setup();
        // Generate UPDATE churn so the FTS shadow accumulates segment rows.
        for i in 0..30 {
            conn.execute(
                "INSERT INTO decisions (decision, context, type, source_agent, status) \
                 VALUES (?1, 'ctx', 'decision', 'test', 'active')",
                params![format!("seed-{i} apple banana cherry")],
            )
            .unwrap();
        }
        for round in 0..5 {
            conn.execute(
                "UPDATE decisions SET decision = decision || ?1 WHERE source_agent = 'test'",
                params![format!(" round{round}")],
            )
            .unwrap();
        }
        let pre = fts_segment_row_total(&conn);
        assert!(pre > 0, "fixture should produce FTS segment rows");
        let optimized = optimize_fts_indexes(&conn);
        assert!(optimized, "optimize should report success on populated FTS");
        let post = fts_segment_row_total(&conn);
        assert!(
            post < pre,
            "optimize should reduce segment row count: {pre} -> {post}"
        );
    }

    #[test]
    fn test_startup_governor_relieves_pressure_without_vacuum() {
        let conn = setup();
        let payload = "x".repeat(4096);
        for i in 0..600 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{payload}{i}")],
            )
            .unwrap();
        }
        conn.execute("DELETE FROM events WHERE type = 'decision_stored'", [])
            .unwrap();
        let freelist_before = freelist_count(&conn);
        assert!(
            freelist_before > VACUUM_FREELIST_THRESHOLD_PAGES,
            "fixture should create enough reclaimable pages to trigger governor"
        );

        let result = run_compaction_governor_startup(&conn);
        assert!(
            result.is_some(),
            "startup governor should run when freelist pressure is high"
        );
        let freelist_after = freelist_count(&conn);
        assert!(
            freelist_after > 0,
            "startup governor should skip VACUUM to keep early lock windows shorter"
        );
    }

    #[test]
    fn test_event_type_caps_prune_oldest_rows() {
        let conn = setup();
        for i in 0..10 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent, created_at)
                 VALUES ('decision_stored', ?1, 'test', datetime('now', ?2))",
                params![format!("{{\"i\":{i}}}"), format!("-{} minutes", 10 - i)],
            )
            .unwrap();
        }

        let pruned = prune_event_type_caps(&conn, &[("decision_stored", 3)]);
        assert_eq!(pruned, 7);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn test_nonboot_event_overflow_prunes_only_nonboot_rows() {
        let conn = setup();
        for i in 0..8 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('agent_boot', '{}', 'test')",
                [],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('boot_savings', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('boot_savings_rollup', '{}', 'test')",
            [],
        )
        .unwrap();

        let pruned = prune_nonboot_event_overflow(&conn, 2);
        assert_eq!(pruned, 6);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let agent_boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'agent_boot'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rollup_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings_rollup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 2);
        assert_eq!(agent_boot_rows, 3);
        assert_eq!(boot_rows, 1);
        assert_eq!(boot_rollup_rows, 1);
    }

    #[test]
    fn test_nonboot_event_overflow_preserves_savings_analytics_rows() {
        let conn = setup();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('recall_query', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('store_savings', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('tool_call_savings', '{}', 'test')",
            [],
        )
        .unwrap();

        // keep_rows=3 equals the number of protected analytics rows, so all
        // non-analytics non-boot rows should be pruned.
        let pruned = prune_nonboot_event_overflow(&conn, 3);
        assert_eq!(pruned, 5);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let protected_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 0);
        assert_eq!(protected_rows, 3);
    }

    #[test]
    fn test_nonboot_event_overflow_limit_batches_deletes() {
        let conn = setup();
        for i in 0..8 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }

        let pruned = prune_nonboot_event_overflow_with_limit(&conn, 2, Some(3));
        assert_eq!(pruned, 3, "startup mode should batch overflow pruning");

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 5);
    }

    #[test]
    fn test_prune_old_events_treats_missing_created_at_as_old() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) VALUES ('decision_stored', '{}', 'test', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) VALUES ('boot_savings', '{}', 'test', NULL)",
            [],
        )
        .unwrap();

        let pruned = prune_old_events_with_retention(&conn, 14);
        assert_eq!(pruned, 1);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 0);
        assert_eq!(boot_rows, 1);
    }

    #[test]
    fn test_prune_old_events_limit_batches_deletes() {
        let conn = setup();
        for i in 0..6 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent, created_at) \
                 VALUES ('decision_stored', ?1, 'test', datetime('now', '-40 days', ?2))",
                params![format!("{{\"i\":{i}}}"), format!("+{} minutes", i)],
            )
            .unwrap();
        }

        let pruned = prune_old_events_with_retention_limit(&conn, 14, Some(2));
        assert_eq!(pruned, 2);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 4);
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
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source = 'ttl::mem'",
                [],
                |r| r.get(0),
            )
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

    #[test]
    fn test_rollup_old_savings_events_compacts_rows() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'test', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "saved": 80,
                "spent": 20,
                "budget": 100,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('store_savings', ?1, 'test', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "saved": 50,
                "served": 25,
                "baseline": 75
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'test', datetime('now', '-1 days'))",
            params![serde_json::json!({
                "saved": 9,
                "spent": 1,
                "budget": 10,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();

        let rolled = rollup_old_savings_events(&conn, 7);
        assert_eq!(rolled, 2);

        let remaining_old: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE created_at < datetime('now', '-7 days') \
                   AND type IN ('recall_query', 'store_savings', 'tool_call_savings')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_old, 0);

        let remaining_recent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE created_at >= datetime('now', '-7 days') \
                   AND type = 'recall_query'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_recent, 1);

        let (saved, served, baseline, events, hits, misses): (i64, i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT \
                     COALESCE(SUM(saved), 0), \
                     COALESCE(SUM(served), 0), \
                     COALESCE(SUM(baseline), 0), \
                     COALESCE(SUM(events), 0), \
                     COALESCE(SUM(hits), 0), \
                     COALESCE(SUM(misses), 0) \
                 FROM event_savings_rollups",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(saved, 130);
        assert_eq!(served, 45);
        assert_eq!(baseline, 175);
        assert_eq!(events, 2);
        assert_eq!(hits, 1);
        assert_eq!(misses, 0);
    }

    #[test]
    fn test_rollup_old_savings_events_ignores_benchmark_agent_payloads() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('store_savings', ?1, 'rust-daemon', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 500,
                "served": 100,
                "baseline": 600
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('store_savings', ?1, 'rust-daemon', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "agent": "codex",
                "saved": 50,
                "served": 25,
                "baseline": 75
            })
            .to_string()],
        )
        .unwrap();

        let rolled = rollup_old_savings_events(&conn, 7);
        assert_eq!(
            rolled, 1,
            "benchmark rows should not be rolled into production rollups"
        );

        let rollup_saved: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(saved), 0) FROM event_savings_rollups WHERE operation = 'store'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rollup_saved, 50);

        let benchmark_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE type = 'store_savings' \
                   AND created_at < datetime('now', '-7 days') \
                   AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(benchmark_rows, 1);
    }

    #[test]
    fn test_prune_old_event_savings_rollups_respects_retention() {
        let conn = setup();
        conn.execute(
            "INSERT INTO event_savings_rollups (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) \
             VALUES (date('now', '-200 days'), 1, 'recall', 10, 5, 15, 1, 1, 0, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO event_savings_rollups (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) \
             VALUES (date('now', '-1 days'), 2, 'store', 20, 10, 30, 2, 0, 0, datetime('now'))",
            [],
        )
        .unwrap();

        let deleted = prune_old_event_savings_rollups(&conn, 120);
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM event_savings_rollups", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_purge_benchmark_artifacts_removes_benchmark_rows_and_dependencies() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memory_clusters (label, consolidated_text, member_count) VALUES ('bench', 'x', 0)",
            [],
        )
        .unwrap();
        let cluster_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (id, decision, context, type, source_agent, status) VALUES (1, 'bench', 'ctx', 'benchmark', 'amb-cortex::run-a', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (id, decision, context, type, source_agent, status) VALUES (2, 'prod', 'ctx', 'decision', 'codex', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decision_conflicts (source_decision_id, target_decision_id, classification) VALUES (1, 2, 'REFINES')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', 1, X'0102', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', 2, X'0304', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 1, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 2, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, result_id, signal, agent) VALUES ('q', 'decision::1', 'decision', 1, 1.0, 'amb-cortex::run-a')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, result_id, signal, agent) VALUES ('q', 'decision::2', 'decision', 2, 1.0, 'codex')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('decision::1', 'memory::x', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('decision::2', 'memory::x', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{\"id\":1,\"source_agent\":\"amb-cortex::run-a\"}', 'rust-daemon')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('merge', '{\"source_agent\":\"amb-cortex::run-a\"}', 'rust-daemon')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('merge', '{\"source_agent\":\"codex\"}', 'rust-daemon')",
            [],
        )
        .unwrap();

        let result = purge_benchmark_artifacts(&conn);
        assert_eq!(result.decisions_deleted, 1);
        assert_eq!(result.embeddings_deleted, 1);
        assert!(result.events_deleted >= 2);

        let decisions_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(decisions_remaining, 1);
        let feedback_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM recall_feedback", [], |row| row.get(0))
            .unwrap();
        assert_eq!(feedback_remaining, 1);
        let cooccurrence_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cooccurrence_remaining, 1);
    }

    #[test]
    fn test_prune_old_benchmark_artifacts_respects_retention_window() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, created_at, updated_at) VALUES ('old-bench', 'ctx', 'benchmark', 'amb-cortex::run-old', 'active', datetime('now', '-10 days'), datetime('now', '-10 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, created_at, updated_at) VALUES ('new-bench', 'ctx', 'benchmark', 'amb-cortex::run-new', 'active', datetime('now', '-1 days'), datetime('now', '-1 days'))",
            [],
        )
        .unwrap();

        let deleted = prune_old_benchmark_artifacts(&conn, 7, true);
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE LOWER(type) = 'benchmark'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_prune_old_benchmark_artifacts_removes_data_agent_marked_events() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-5 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 10,
                "served": 5,
                "baseline": 15
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'rust-daemon', datetime('now', '-5 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 5,
                "spent": 2,
                "budget": 7,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();

        let deleted = prune_old_benchmark_artifacts(&conn, 2, true);
        assert!(deleted >= 2);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_prune_orphan_cluster_members_removes_missing_targets() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memory_clusters (label, consolidated_text, member_count) VALUES ('c1', 'x', 0)",
            [],
        )
        .unwrap();
        let cluster_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, status) VALUES ('m1', 'memory::1', 'active')",
            [],
        )
        .unwrap();
        let memory_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, status) VALUES ('d1', 'ctx', 'active')",
            [],
        )
        .unwrap();
        let decision_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'memory', ?2, 1.0)",
            params![cluster_id, memory_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', ?2, 1.0)",
            params![cluster_id, decision_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 999999, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'memory', 999999, 1.0)",
            params![cluster_id],
        )
        .unwrap();

        let pruned = prune_orphan_cluster_members(&conn);
        assert_eq!(pruned, 2);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM cluster_members", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
    }
}
