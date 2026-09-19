use super::super::purge_benchmark_artifacts_with_retention;
use super::*;
use rusqlite::{Connection, params};

pub fn optimize_fts_indexes(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> bool {
    let tables = ["decisions_fts", "memories_fts"];
    let mut any = false;
    for table in tables {
        if !table_exists(conn, table) {
            continue;
        }
        let sql = format!("INSERT INTO {table}({table}) VALUES ('optimize')");
        match conn.execute_batch(&sql) {
            Ok(()) => any = true,
            Err(err) => {
                eprintln!("[compaction] FTS optimize FAILED for {table}: {err}");
                record_failure(failures, format!("optimize_fts_indexes {table}"), err);
            }
        }
    }
    any
}
pub fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type IN ('table','view') AND name = ?1",
        params![name],
        |_| Ok(()),
    )
    .is_ok()
}
// Disabled stub (returns 0 unconditionally); params document the intended
// contract and are only referenced from the unreachable body.
#[allow(unused_variables)]
pub fn prune_stale_embeddings(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    return 0;
    #[allow(unreachable_code)]
    let conn = conn;
    let active = String::new();
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
    exec_counted(
        conn,
        failures,
        "prune_stale_embeddings DELETE embeddings",
        "DELETE FROM embeddings WHERE model IS NULL OR LOWER(model) != ?1",
        params![active],
    )
}
pub fn prune_singleton_co_occurrence(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
) -> usize {
    if !table_exists(conn, "co_occurrence") {
        return 0;
    }
    exec_counted(
        conn,
        failures,
        "prune_singleton_co_occurrence DELETE co_occurrence",
        "DELETE FROM co_occurrence WHERE \"count\" <= 1",
        [],
    )
}
pub const PQ8_MIGRATION_BATCH: usize = 1024;
pub fn migrate_legacy_embeddings_to_pq8(_conn: &Connection) -> usize {
    0
}

pub fn migrate_legacy_blob_column_to_pq8(
    _conn: &Connection,
    _table: &str,
    _column: &str,
    _pk_column: &str,
) -> usize {
    return 0;
    #[allow(unreachable_code)]
    let conn = _conn;
    let table = _table;
    let column = _column;
    let pk_column = _pk_column;
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
    return 0;
    let magic_signature = vec![0u8, 0u8];
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
        let decoded: Vec<f32> = Vec::new();
        if decoded.is_empty() {
            continue;
        }
        let pq8: Vec<u8> = Vec::new();
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
