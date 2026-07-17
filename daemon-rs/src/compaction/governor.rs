use super::*;
use rusqlite::{params, Connection};
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
    pub legacy_embeddings_migrated: usize,
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
pub(crate) fn bytes_to_mb(bytes: i64) -> i64 {
    bytes / (1024 * 1024)
}
pub fn classify_storage_pressure(db_size_bytes: i64) -> &'static str {
    if db_size_bytes >= STORAGE_HARD_LIMIT_BYTES {
        "critical"
    } else if db_size_bytes >= STORAGE_SOFT_LIMIT_BYTES {
        "elevated"
    } else {
        "normal"
    }
}
pub fn classify_event_pressure(nonboot_event_rows: i64) -> &'static str {
    if nonboot_event_rows >= EVENT_NONBOOT_HARD_LIMIT_ROWS {
        "critical"
    } else if nonboot_event_rows >= EVENT_NONBOOT_SOFT_LIMIT_ROWS {
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
pub(crate) fn should_run_compaction_governor_with_pressure(
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
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        total += n;
    }
    total
}
pub fn run_compaction_governor(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, true)
}
pub fn run_compaction_governor_startup(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, false)
}
pub(crate) fn run_compaction_governor_with_options(
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
"[compaction] governor: pressure {} -> {}, size {}MB -> {}MB, nonboot_events {} -> {}, fts_segments {} -> {}",pressure_before,
pressure_after,bytes_to_mb(result.bytes_before),bytes_to_mb(result.bytes_after),nonboot_event_rows_before,non_boot_event_count(
conn),fts_segment_rows_before,fts_segment_rows_after,);
    Some(result)
}
pub fn run_compaction(conn: &Connection) -> CompactionResult {
    run_compaction_with_options(conn, true)
}
pub(crate) fn run_compaction_with_options(
    conn: &Connection,
    allow_vacuum: bool,
) -> CompactionResult {
    let startup_prune_limit = (!allow_vacuum).then_some(STARTUP_EVENT_PRUNE_BATCH_ROWS);
    let mut result = CompactionResult {
        bytes_before: db_size_bytes(conn),
        ..CompactionResult::default()
    };
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
    result.archived_text_stripped = strip_archived_text(conn);
    result.expired_pruned = prune_expired_entries(conn);
    result.crystal_embeddings_pruned = prune_crystal_member_embeddings(conn);
    result.cluster_members_pruned = prune_orphan_cluster_members(conn);
    result.feedback_aggregated = aggregate_old_feedback(conn);
    result.stale_embeddings_pruned = prune_stale_embeddings(conn);
    result.co_occurrence_pruned = prune_singleton_co_occurrence(conn);
    result.legacy_embeddings_migrated = migrate_legacy_embeddings_to_pq8(conn);
    result.fts_optimized = optimize_fts_indexes(conn);
    checkpoint_after_compaction(conn, allow_vacuum);
    let freelist_pages = freelist_count(conn);
    let total_deleted = result.events_pruned
        + result.benchmark_pruned
        + result.archived_text_stripped
        + result.expired_pruned
        + result.crystal_embeddings_pruned
        + result.feedback_aggregated
        + result.stale_embeddings_pruned
        + result.co_occurrence_pruned
        + result.legacy_embeddings_migrated;
    if allow_vacuum && (freelist_pages > VACUUM_FREELIST_THRESHOLD_PAGES || result.fts_optimized) {
        let _ = conn.execute_batch("VACUUM;");
    }
    result.bytes_after = db_size_bytes(conn);
    if total_deleted > 0 || result.fts_optimized {
        let saved_kb = (result.bytes_before - result.bytes_after) / 1024;
        eprintln!(
"[compaction] Pruned: {} events, {} benchmark rows, {} archived texts, {} expired rows, {} crystal embeddings, {} orphan cluster members, {} feedback rows, {} stale embeddings, {} singleton co-occurrence pairs, {} legacy embeddings migrated; fts_optimized={}. Saved {}KB"
,result.events_pruned,result.benchmark_pruned,result.archived_text_stripped,result.expired_pruned,result.crystal_embeddings_pruned
,result.cluster_members_pruned,result.feedback_aggregated,result.stale_embeddings_pruned,result.co_occurrence_pruned,result.
legacy_embeddings_migrated,result.fts_optimized,saved_kb);
    }
    result
}
pub(crate) fn optimize_fts_indexes(conn: &Connection) -> bool {
    let tables = ["decisions_fts", "memories_fts"];
    let mut any = false;
    for table in tables {
        if !table_exists(conn, table) {
            continue;
        }
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
pub(crate) fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        params![name],
        |_| Ok(()),
    )
    .is_ok()
}
pub(crate) fn prune_stale_embeddings(conn: &Connection) -> usize {
    let active = crate::embeddings::selected_model_key().to_ascii_lowercase();
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
pub(crate) fn prune_singleton_co_occurrence(conn: &Connection) -> usize {
    if !table_exists(conn, "co_occurrence") {
        return 0;
    }
    conn.execute("DELETE FROM co_occurrence WHERE \"count\" <= 1", [])
        .unwrap_or(0)
}
pub(crate) const PQ8_MIGRATION_BATCH: usize = 1024;
pub(crate) fn migrate_legacy_embeddings_to_pq8(conn: &Connection) -> usize {
    let from_embeddings = migrate_legacy_blob_column_to_pq8(conn, "embeddings", "vector", "id");
    let from_clusters =
        migrate_legacy_blob_column_to_pq8(conn, "memory_clusters", "centroid", "id");
    from_embeddings + from_clusters
}
pub(crate) fn migrate_legacy_blob_column_to_pq8(
    conn: &Connection,
    table: &str,
    column: &str,
    pk_column: &str,
) -> usize {
    if !table_exists(conn, table) {
        return 0;
    }
    let select_sql = format!(
        "SELECT \"{pk}\", \"{col}\" FROM \"{tbl}\" \
         WHERE \"{col}\" IS NOT NULL \
           AND substr(\"{col}\", 1, 2) != ?1 \
           AND (LENGTH(\"{col}\") % 4) = 0 \
         LIMIT ?2",
        pk = pk_column,
        col = column,
        tbl = table,
    );
    let mut stmt = match conn.prepare(&select_sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            eprintln!("[compaction] PQ8 migration prepare failed for {table}.{column}: {err}");
            return 0;
        }
    };
    let magic_signature = vec![
        crate::embeddings::PQ8_MAGIC_BYTE,
        crate::embeddings::PQ8_FORMAT_VERSION,
    ];
    let candidates: Vec<(i64, Vec<u8>)> = match stmt.query_map(
        params![magic_signature, PQ8_MIGRATION_BATCH as i64],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
    ) {
        Ok(rows) => rows.flatten().collect(),
        Err(err) => {
            eprintln!("[compaction] PQ8 migration query failed for {table}.{column}: {err}");
            return 0;
        }
    };
    drop(stmt);
    if candidates.is_empty() {
        return 0;
    }
    let update_sql = format!(
        "UPDATE \"{tbl}\" SET \"{col}\" = ?1 WHERE \"{pk}\" = ?2",
        pk = pk_column,
        col = column,
        tbl = table,
    );
    let mut migrated = 0usize;
    for (id, blob) in candidates {
        let decoded = crate::embeddings::legacy_f32_blob_to_vector(&blob);
        if decoded.is_empty() {
            continue;
        }
        let pq8 = crate::embeddings::vector_to_pq8_blob(&decoded);
        match conn.execute(&update_sql, params![pq8, id]) {
            Ok(_) => migrated += 1,
            Err(err) => {
                eprintln!(
                    "[compaction] PQ8 migration update failed for {table}.{column} id={id}: {err}"
                );
            }
        }
    }
    migrated
}
pub fn purge_benchmark_artifacts(conn: &Connection) -> BenchmarkPurgeResult {
    purge_benchmark_artifacts_with_retention(conn, None, true)
}
