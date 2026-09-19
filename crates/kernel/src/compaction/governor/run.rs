use super::*;
use rusqlite::Connection;

pub fn run_compaction_governor(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, true)
}
pub fn run_compaction_governor_startup(conn: &Connection) -> Option<CompactionResult> {
    run_compaction_governor_with_options(conn, false)
}
pub fn run_compaction_governor_with_options(
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
        result.events_pruned += rollup_old_boot_savings_with_retention(
            conn,
            &mut result.failures,
            AGGRESSIVE_BOOT_SAVINGS_RETENTION_DAYS,
        );
        result.events_pruned += rollup_old_savings_events(
            conn,
            &mut result.failures,
            AGGRESSIVE_SAVINGS_EVENT_ROLLUP_RETENTION_DAYS,
        );
        result.events_pruned += prune_old_event_savings_rollups(
            conn,
            &mut result.failures,
            AGGRESSIVE_EVENT_SAVINGS_ROLLUP_RETENTION_DAYS,
        );
        result.events_pruned += prune_old_events_with_retention_limit(
            conn,
            &mut result.failures,
            AGGRESSIVE_EVENT_RETENTION_DAYS,
            startup_prune_limit,
        );
        result.events_pruned += prune_event_type_caps_with_limit(
            conn,
            &mut result.failures,
            EVENT_TYPE_HARD_CAPS,
            startup_prune_limit,
        );
        result.events_pruned += prune_nonboot_event_overflow_with_limit(
            conn,
            &mut result.failures,
            EVENT_NONBOOT_HARD_KEEP_ROWS,
            startup_prune_limit,
        );
        result.benchmark_pruned += prune_old_benchmark_artifacts(
            conn,
            &mut result.failures,
            AGGRESSIVE_BENCHMARK_RETENTION_DAYS,
            allow_vacuum,
        );
        result.archived_text_stripped += strip_archived_text_with_retention(
            conn,
            &mut result.failures,
            AGGRESSIVE_ARCHIVED_TEXT_RETENTION_DAYS,
        );
        result.cluster_members_pruned += prune_orphan_cluster_members(conn, &mut result.failures);
        result.feedback_aggregated += aggregate_old_feedback_with_window(
            conn,
            &mut result.failures,
            AGGRESSIVE_FEEDBACK_AGGREGATION_DAYS,
        );
        let vacuum_sql = if allow_vacuum {
            "PRAGMA wal_checkpoint(TRUNCATE); VACUUM;"
        } else {
            "PRAGMA wal_checkpoint(PASSIVE);"
        };
        exec_batch_counted(
            conn,
            &mut result.failures,
            "governor aggressive checkpoint+VACUUM",
            vacuum_sql,
        );
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
        fts_segment_rows_after
    );
    if !result.failures.is_empty() {
        eprintln!(
            "[compaction] governor: {} destructive op(s) FAILED; see FAILED lines above, reported in CompactionResult.failures",
            result.failures.len()
        );
    }
    Some(result)
}
pub fn run_compaction(conn: &Connection) -> CompactionResult {
    run_compaction_with_options(conn, true)
}
pub fn run_compaction_with_options(conn: &Connection, allow_vacuum: bool) -> CompactionResult {
    let startup_prune_limit = (!allow_vacuum).then_some(STARTUP_EVENT_PRUNE_BATCH_ROWS);
    let mut result = CompactionResult {
        bytes_before: db_size_bytes(conn),
        ..CompactionResult::default()
    };
    result.events_pruned = rollup_old_boot_savings(conn, &mut result.failures);
    result.events_pruned += rollup_old_savings_events(
        conn,
        &mut result.failures,
        SAVINGS_EVENT_ROLLUP_RETENTION_DAYS,
    );
    result.events_pruned += prune_old_event_savings_rollups(
        conn,
        &mut result.failures,
        EVENT_SAVINGS_ROLLUP_RETENTION_DAYS,
    );
    result.events_pruned += prune_old_events_with_retention_limit(
        conn,
        &mut result.failures,
        EVENT_RETENTION_DAYS,
        startup_prune_limit,
    );
    result.events_pruned += prune_event_type_caps_with_limit(
        conn,
        &mut result.failures,
        EVENT_TYPE_SOFT_CAPS,
        startup_prune_limit,
    );
    result.events_pruned += prune_nonboot_event_overflow_with_limit(
        conn,
        &mut result.failures,
        EVENT_NONBOOT_SOFT_KEEP_ROWS,
        startup_prune_limit,
    );
    result.benchmark_pruned = prune_old_benchmark_artifacts(
        conn,
        &mut result.failures,
        BENCHMARK_RETENTION_DAYS,
        allow_vacuum,
    );
    result.archived_text_stripped = strip_archived_text(conn, &mut result.failures);
    result.expired_pruned = prune_expired_entries(conn, &mut result.failures);
    result.crystal_embeddings_pruned = prune_crystal_member_embeddings(conn);
    result.cluster_members_pruned = prune_orphan_cluster_members(conn, &mut result.failures);
    result.feedback_aggregated = aggregate_old_feedback(conn, &mut result.failures);
    result.stale_embeddings_pruned = prune_stale_embeddings(conn, &mut result.failures);
    result.co_occurrence_pruned = prune_singleton_co_occurrence(conn, &mut result.failures);
    result.legacy_embeddings_migrated = migrate_legacy_embeddings_to_pq8(conn);
    result.fts_optimized = optimize_fts_indexes(conn, &mut result.failures);
    checkpoint_after_compaction(conn, &mut result.failures, allow_vacuum);
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
        exec_batch_counted(
            conn,
            &mut result.failures,
            "post-compaction VACUUM",
            "VACUUM;",
        );
    }
    result.bytes_after = db_size_bytes(conn);
    if total_deleted > 0 || result.fts_optimized {
        let saved_kb = (result.bytes_before - result.bytes_after) / 1024;
        eprintln!(
            "[compaction] Pruned: {} events, {} benchmark rows, {} archived texts, {} expired rows, {} crystal embeddings, {} orphan cluster members, {} feedback rows, {} stale embeddings, {} singleton co-occurrence pairs, {} legacy embeddings migrated; fts_optimized={}. Saved {}KB",
            result.events_pruned,
            result.benchmark_pruned,
            result.archived_text_stripped,
            result.expired_pruned,
            result.crystal_embeddings_pruned,
            result.cluster_members_pruned,
            result.feedback_aggregated,
            result.stale_embeddings_pruned,
            result.co_occurrence_pruned,
            result.legacy_embeddings_migrated,
            result.fts_optimized,
            saved_kb
        );
    }
    if !result.failures.is_empty() {
        eprintln!(
            "[compaction] {} destructive op(s) FAILED; see FAILED lines above, reported in CompactionResult.failures",
            result.failures.len()
        );
    }
    result
}
