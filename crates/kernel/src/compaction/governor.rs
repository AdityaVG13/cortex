use super::*;
use rusqlite::Connection;

/// One failed destructive/maintenance operation.
///
/// Contract: `op` names the operation and its target table; `error` is the
/// driver message. An empty `failures` vec on a result struct means every
/// destructive op succeeded — a failed purge must never be indistinguishable
/// from an empty one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceFailure {
    pub op: String,
    pub error: String,
}

pub fn record_failure(
    failures: &mut Vec<MaintenanceFailure>,
    op: impl Into<String>,
    error: impl ToString,
) {
    failures.push(MaintenanceFailure {
        op: op.into(),
        error: error.to_string(),
    });
}

pub fn fail_zero(
    failures: &mut Vec<MaintenanceFailure>,
    op: impl Into<String>,
    error: impl ToString,
) -> usize {
    record_failure(failures, op, error);
    0
}

#[derive(Debug, Default)]
pub struct CompactionResult {
    pub events_pruned: usize,
    pub benchmark_pruned: usize,
    pub archived_text_stripped: usize,
    pub expired_pruned: usize,
    pub crystal_embeddings_pruned: usize,
    pub cluster_members_pruned: usize,
    pub feedback_aggregated: usize,
    pub query_memory_trimmed: usize,
    pub term_bridges_trimmed: usize,
    pub stale_embeddings_pruned: usize,
    pub co_occurrence_pruned: usize,
    pub legacy_embeddings_migrated: usize,
    pub fts_optimized: bool,
    pub bytes_before: i64,
    pub bytes_after: i64,
    /// Destructive ops executed by this module that failed. Empty = all
    /// succeeded. Sibling-module prunes (events/feedback/crystals/archived)
    /// are outside this report (no-claim boundary).
    pub failures: Vec<MaintenanceFailure>,
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
    /// Destructive/maintenance ops executed by this purge that failed. Empty =
    /// all succeeded. Additive visibility only: `total_deleted` counts exactly
    /// the deletions that committed, so purge pass/fail semantics are
    /// unchanged.
    pub failures: Vec<MaintenanceFailure>,
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
pub fn bytes_to_mb(bytes: i64) -> i64 {
    bytes / (1024 * 1024)
}

/// Runs one destructive maintenance statement, returning rows affected.
/// On failure the error is logged (op + driver message) and recorded in
/// `failures` so the outcome struct reflects it; returns 0 — a failed purge
/// is never counted as work.
pub fn exec_counted(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> usize {
    exec_counted_named("compaction", conn, failures, op, sql, params)
}

pub fn exec_counted_named(
    channel: &str,
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> usize {
    match conn.execute(sql, params) {
        Ok(n) => n,
        Err(err) => {
            eprintln!("[{channel}] {op} FAILED: {err}");
            record_failure(failures, op, err);
            0
        }
    }
}

pub fn exec_batch_counted(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    sql: &str,
) {
    if let Err(err) = conn.execute_batch(sql) {
        eprintln!("[compaction] {op} FAILED: {err}");
        record_failure(failures, op, err);
    }
}
pub fn classify_storage_pressure(db_size_bytes: i64) -> &'static str {
    classify_pressure(
        db_size_bytes,
        STORAGE_HARD_LIMIT_BYTES,
        STORAGE_SOFT_LIMIT_BYTES,
    )
}
pub fn classify_event_pressure(nonboot_event_rows: i64) -> &'static str {
    classify_pressure(
        nonboot_event_rows,
        EVENT_NONBOOT_HARD_LIMIT_ROWS,
        EVENT_NONBOOT_SOFT_LIMIT_ROWS,
    )
}
fn classify_pressure(value: i64, hard: i64, soft: i64) -> &'static str {
    if value >= hard {
        "critical"
    } else if value >= soft {
        "elevated"
    } else {
        "normal"
    }
}
pub const FTS_SEGMENT_ROW_SOFT_LIMIT: i64 = 10_000;
#[cfg_attr(not(test), allow(dead_code))]
pub fn should_run_compaction_governor(db_size_bytes: i64, freelist_pages: i64) -> bool {
    should_run_compaction_governor_with_pressure(db_size_bytes, freelist_pages, 0, 0)
}
pub fn should_run_compaction_governor_with_pressure(
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
pub fn fts_segment_row_total(conn: &Connection) -> i64 {
    let tables = ["decisions_fts_data", "memories_fts_data"];
    let mut total: i64 = 0;
    for table in tables {
        if !table_exists(conn, table) {
            continue;
        }
        match conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
            row.get::<_, i64>(0)
        }) {
            Ok(n) => total += n,
            // A failed COUNT is not "no FTS pressure": that would skip the
            // governor while segment tables are unreadable or locked.
            Err(_) => return FTS_SEGMENT_ROW_SOFT_LIMIT.saturating_add(1),
        }
    }
    total
}
mod prune;
pub use prune::*;
mod run;
pub use run::*;
