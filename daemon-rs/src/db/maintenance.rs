// SPDX-License-Identifier: MIT
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::{params, Connection, OptionalExtension};


use super::*;
pub fn migrate_focus_table(conn: &Connection) {
    let sql = r#"
        CREATE TABLE IF NOT EXISTS focus_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            agent TEXT NOT NULL DEFAULT 'unknown',
            status TEXT NOT NULL DEFAULT 'open',
            raw_entries TEXT NOT NULL DEFAULT '[]',
            summary TEXT,
            started_at TEXT DEFAULT (datetime('now')),
            ended_at TEXT,
            tokens_before INTEGER DEFAULT 0,
            tokens_after INTEGER DEFAULT 0
        )
    "#;
    match conn.execute_batch(sql) {
        Ok(_) => {}
        Err(e) => eprintln!("[db] Focus table migration: {e}"),
    }
}

/// Run schema migrations for progressive aging columns.
/// Safe to call repeatedly -- ALTER TABLE with IF NOT EXISTS-style error handling.
pub(crate) fn migrate_aging_columns_with_logging(conn: &Connection, log_success: bool) {
    let migrations = [
        "ALTER TABLE memories ADD COLUMN compressed_text TEXT",
        "ALTER TABLE memories ADD COLUMN age_tier TEXT DEFAULT 'fresh'",
        "ALTER TABLE decisions ADD COLUMN compressed_text TEXT",
        "ALTER TABLE decisions ADD COLUMN age_tier TEXT DEFAULT 'fresh'",
    ];
    for sql in &migrations {
        match conn.execute(sql, []) {
            Ok(_) if log_success => eprintln!("[db] Migration applied: {sql}"),
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => eprintln!("[db] Migration skipped ({e}): {sql}"),
        }
    }
}

pub(crate) fn unix_now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(now).unwrap_or(i64::MAX)
}

pub(crate) fn should_attempt_best_effort_checkpoint(now_ms: i64, last_checkpoint_ms: i64) -> bool {
    now_ms.saturating_sub(last_checkpoint_ms) >= BEST_EFFORT_CHECKPOINT_MIN_INTERVAL_MS
}

pub(crate) fn should_attempt_truncate_checkpoint(now_ms: i64, last_truncate_ms: i64) -> bool {
    if last_truncate_ms <= 0 {
        return false;
    }
    now_ms.saturating_sub(last_truncate_ms) >= BEST_EFFORT_TRUNCATE_INTERVAL_MS
}

/// Attempt a WAL checkpoint; silently ignore any error.
pub fn checkpoint_wal_best_effort(conn: &Connection) {
    let now_ms = unix_now_ms();
    let last_checkpoint_ms = LAST_BEST_EFFORT_CHECKPOINT_MS.load(Ordering::Relaxed);
    if !should_attempt_best_effort_checkpoint(now_ms, last_checkpoint_ms) {
        return;
    }
    if LAST_BEST_EFFORT_CHECKPOINT_MS
        .compare_exchange(
            last_checkpoint_ms,
            now_ms,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_err()
    {
        return;
    }

    let mut last_truncate_ms = LAST_BEST_EFFORT_TRUNCATE_MS.load(Ordering::Relaxed);
    if last_truncate_ms <= 0 {
        LAST_BEST_EFFORT_TRUNCATE_MS.store(now_ms, Ordering::Relaxed);
        last_truncate_ms = now_ms;
    }

    if should_attempt_truncate_checkpoint(now_ms, last_truncate_ms)
        && conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .is_ok()
    {
        LAST_BEST_EFFORT_TRUNCATE_MS.store(now_ms, Ordering::Relaxed);
        return;
    }

    let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExpiredCleanupCounts {
    pub memories_deleted: usize,
    pub decisions_deleted: usize,
}

/// Delete expired rows from tables that support TTL-based retention.
pub fn delete_expired_entries(conn: &Connection) -> rusqlite::Result<ExpiredCleanupCounts> {
    let memories_deleted = conn.execute(
        "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    )?;
    let decisions_deleted = conn.execute(
        "DELETE FROM decisions WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    )?;
    Ok(ExpiredCleanupCounts {
        memories_deleted,
        decisions_deleted,
    })
}

/// Rebuild FTS5 indexes from base table data. Call once after schema migration
/// on databases that predate FTS5 support.
pub fn rebuild_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags)
         SELECT id, text, source, tags FROM memories WHERE status = 'active';
         INSERT OR IGNORE INTO decisions_fts(rowid, decision, context)
         SELECT id, decision, context FROM decisions WHERE status = 'active';",
    )?;
    Ok(())
}

/// Fully reindex FTS5 tables from canonical memory/decision rows.
///
/// Unlike `rebuild_fts`, this removes stale rows first so deleted/orphaned
/// entries do not remain searchable.
pub fn reindex_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT INTO memories_fts(memories_fts) VALUES('delete-all');
         INSERT INTO decisions_fts(decisions_fts) VALUES('delete-all');",
    )?;
    rebuild_fts(conn)
}

/// Seed FTS indexes at most once per database.
///
/// Uses a marker row in `schema_migrations` so startup does not rescan the
/// entire corpus on every daemon restart.
pub fn rebuild_fts_if_needed(conn: &Connection) -> rusqlite::Result<bool> {
    let already_seeded = conn
        .query_row(
            "SELECT 1 FROM schema_migrations WHERE version = 'fts_seeded_v1' LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    if already_seeded.is_some() {
        return Ok(false);
    }

    rebuild_fts(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at)
         VALUES ('fts_seeded_v1', 'fts_seeded', datetime('now'))",
        [],
    )?;
    Ok(true)
}

/// Run `PRAGMA integrity_check` and return `true` when the database reports
/// `ok`.
pub fn verify_integrity(conn: &Connection) -> rusqlite::Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
}

/// Run `PRAGMA quick_check` (B-tree structure only -- faster than integrity_check).
/// Returns `true` when the database passes.
pub fn quick_check(conn: &Connection) -> bool {
    conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map(|s| s.trim().eq_ignore_ascii_case("ok"))
        .unwrap_or(false)
}

/// Attempt to recover data from a corrupted DB using dump-and-rebuild.
///
/// Steps:
///   1. Open `db_path` read-only in a *separate* connection (does not touch the live connection).
///   2. `SELECT *` from every data table -- data pages survive most B-tree corruption.
///   3. Write all rows into a fresh temp DB at `{db_path}.repair_tmp`.
///   4. Run `PRAGMA integrity_check` on the fresh DB.
///   5. Rename `db_path` → `{db_path}.corrupt.{timestamp}` (never deleted).
///   6. Rename `{db_path}.repair_tmp` → `db_path`.
///
/// The original DB is preserved in all failure paths -- if any step before 5
/// fails, `db_path` is unchanged.
///
/// FTS virtual tables are re-created and populated from data; they are not
/// exported directly.
pub fn auto_repair(db_path: &Path, timestamp: &str) -> Result<RepairResult, RepairError> {
    eprintln!(
        "[cortex] auto_repair: beginning dump-and-rebuild of {}",
        db_path.display()
    );

    // ── Step 1: open the corrupted DB read-only ────────────────────────────
    let corrupt_conn = Connection::open(db_path).map_err(RepairError::OpenCorrupt)?;
    // Open read-only: ignore any write errors on WAL frames, read what we can.
    let busy_timeout_ms = SQLITE_BUSY_TIMEOUT_MS;
    let _ = corrupt_conn.execute_batch(&format!(
        r#"
        PRAGMA busy_timeout = {busy_timeout_ms};
        PRAGMA query_only = ON;
        "#
    ));

    // ── Step 2: export all data tables ────────────────────────────────────
    // Order matters for foreign-key integrity, though FKs are off during repair.
    // Non-relational tables that carry user data are listed first; operational
    // tables that can be empty after rebuild are listed last.
    pub(crate) const DATA_TABLES: &[&str] = &[
        "memories",
        "decisions",
        "embeddings",
        "co_occurrence",
        "events",
        "activities",
        "messages",
        "sessions",
        "tasks",
        "feed",
        "feed_acks",
        "context_cache",
        "focus_sessions",
        "recall_feedback",
        "memory_clusters",
        "cluster_members",
        "locks",
    ];

    // Export: for each table collect column names and all rows as raw SQL strings.
    // We use TEXT casts for all values so we can INSERT via execute_batch without
    // complex type mapping.  This is safe because SQLite is loosely typed.
    let mut table_exports: Vec<(String, Vec<String>)> = Vec::new();
    let mut memories_recovered = 0usize;
    let mut decisions_recovered = 0usize;

    for &table in DATA_TABLES {
        // Check if the table even exists in the corrupted DB.
        let exists: bool = corrupt_conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
                params![table],
                |_| Ok(()),
            )
            .is_ok();
        if !exists {
            eprintln!("[cortex] auto_repair: table '{table}' not found in corrupt DB, skipping");
            continue;
        }

        // Get column names via PRAGMA.
        let mut col_stmt = corrupt_conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(RepairError::Export)?;
        let columns: Vec<String> = col_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(RepairError::Export)?
            .filter_map(|r| r.ok())
            .collect();

        if columns.is_empty() {
            eprintln!("[cortex] auto_repair: table '{table}' has no columns, skipping");
            continue;
        }

        // SELECT * and build INSERT statements using rusqlite's dynamic row access.
        let col_list = columns.join(", ");
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
        let placeholder_list = placeholders.join(", ");

        let mut data_stmt = match corrupt_conn.prepare(&format!("SELECT {col_list} FROM {table}")) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[cortex] auto_repair: failed to prepare SELECT on '{table}': {e}");
                continue;
            }
        };

        let query_result = data_stmt.query([]);
        let mut rows = match query_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[cortex] auto_repair: failed to query '{table}': {e}");
                continue;
            }
        };

        // Build INSERT..VALUES strings with escaped literals.
        // We quote every value as a SQL literal string using SQLite's double-single-quote
        // escaping.  BLOB columns are exported as X'' hex literals.
        let insert_prefix =
            format!("INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({placeholder_list})");

        let mut row_values: Vec<Vec<String>> = Vec::new();
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let mut vals: Vec<String> = Vec::new();
                    for i in 0..columns.len() {
                        use rusqlite::types::ValueRef;
                        let val = match row.get_ref(i) {
                            Ok(ValueRef::Null) => "NULL".to_string(),
                            Ok(ValueRef::Integer(n)) => n.to_string(),
                            Ok(ValueRef::Real(f)) => format!("{f}"),
                            Ok(ValueRef::Text(t)) => {
                                let s = String::from_utf8_lossy(t);
                                // Escape single quotes.
                                format!("'{}'", s.replace('\'', "''"))
                            }
                            Ok(ValueRef::Blob(b)) => {
                                let hex: String =
                                    b.iter().map(|byte| format!("{byte:02X}")).collect();
                                format!("X'{hex}'")
                            }
                            Err(_) => "NULL".to_string(),
                        };
                        vals.push(val);
                    }
                    row_values.push(vals);
                }
                Ok(None) => break,
                Err(e) => {
                    // Row-level corruption: skip the bad row and continue.
                    eprintln!("[cortex] auto_repair: row error in '{table}': {e} -- skipping row");
                    continue;
                }
            }
        }

        eprintln!(
            "[cortex] auto_repair: exported {} rows from '{table}'",
            row_values.len()
        );
        if table == "memories" {
            memories_recovered = row_values.len();
        } else if table == "decisions" {
            decisions_recovered = row_values.len();
        }

        // Convert to literal INSERT statements (values already SQL-safe).
        let inserts: Vec<String> = row_values
            .into_iter()
            .map(|vals| {
                let val_list = vals.join(", ");
                format!("INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({val_list});")
            })
            .collect();

        table_exports.push((insert_prefix, inserts));
    }

    drop(corrupt_conn); // Release the read lock on the corrupt DB.

    // ── Step 3: build a fresh DB at a temp path ────────────────────────────
    let tmp_path = db_path.with_extension("repair_tmp");
    // Remove any leftover temp from a previous failed attempt.
    let _ = std::fs::remove_file(&tmp_path);

    let fresh = Connection::open(&tmp_path).map_err(RepairError::OpenFresh)?;
    configure(&fresh).map_err(RepairError::Import)?;
    initialize_schema(&fresh).map_err(RepairError::Import)?;

    // Disable FK enforcement during bulk insert so we can insert in any order.
    fresh
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(RepairError::Import)?;

    // Insert all exported rows.
    for (_prefix, inserts) in &table_exports {
        for stmt in inserts {
            if let Err(e) = fresh.execute_batch(stmt) {
                // Non-fatal: log and continue -- a few bad rows are acceptable.
                eprintln!("[cortex] auto_repair: insert skipped ({e}): {stmt:.80}");
            }
        }
    }

    // Re-enable FK enforcement and re-populate FTS indexes.
    fresh
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(RepairError::Import)?;

    // Populate FTS from the newly inserted data.
    fresh
        .execute_batch(
            "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags) \
             SELECT id, text, source, tags FROM memories; \
             INSERT OR IGNORE INTO decisions_fts(rowid, decision, context) \
             SELECT id, decision, context FROM decisions;",
        )
        .map_err(RepairError::Import)?;

    // VACUUM to compact and re-linearize the B-tree.
    fresh
        .execute_batch("VACUUM;")
        .map_err(RepairError::Import)?;

    // ── Step 4: verify the fresh DB ────────────────────────────────────────
    let integrity_ok = verify_integrity(&fresh).unwrap_or(false);
    drop(fresh);

    if !integrity_ok {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("[cortex] auto_repair: repaired DB failed integrity_check -- aborting");
        return Err(RepairError::RepairIntegrityFailed);
    }

    // ── Steps 5 & 6: atomic swap ───────────────────────────────────────────
    // Rename original → .corrupt.{timestamp}
    let corrupt_archive = db_path.with_extension(format!("corrupt.{timestamp}"));
    std::fs::rename(db_path, &corrupt_archive).map_err(RepairError::Io)?;

    // Rename repaired temp → original path.
    std::fs::rename(&tmp_path, db_path).map_err(|e| {
        // Roll back: restore the original so the daemon isn't left with no DB.
        let _ = std::fs::rename(&corrupt_archive, db_path);
        RepairError::Io(e)
    })?;

    eprintln!(
        "[cortex] auto_repair: SUCCESS -- {} memories, {} decisions recovered. \
         Corrupted DB archived at {}",
        memories_recovered,
        decisions_recovered,
        corrupt_archive.display()
    );

    Ok(RepairResult {
        memories_recovered,
        decisions_recovered,
        corrupt_db_path: corrupt_archive,
    })
}

/// Set `status = 'archived'` for all rows in `table` whose `id` is in `ids`.
/// Only `memories` and `decisions` are supported; other table names return an
/// error. Returns the number of rows actually updated. If `owner_id` is
/// provided, updates are restricted to that owner.
pub fn archive_entries_scoped(
    conn: &Connection,
    table: &str,
    ids: &[i64],
    owner_id: Option<i64>,
) -> rusqlite::Result<usize> {
    if table != "memories" && table != "decisions" {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "archive_entries: unsupported table '{table}'"
        )));
    }
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders = ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = if owner_id.is_some() {
        format!(
            "UPDATE {table} SET status = 'archived' WHERE owner_id = ?{} AND id IN ({placeholders})",
            ids.len() + 1
        )
    } else {
        format!("UPDATE {table} SET status = 'archived' WHERE id IN ({placeholders})")
    };

    let mut stmt = conn.prepare(&sql)?;
    let affected = if let Some(owner_id) = owner_id {
        let mut values: Vec<rusqlite::types::Value> = ids
            .iter()
            .copied()
            .map(rusqlite::types::Value::Integer)
            .collect();
        values.push(rusqlite::types::Value::Integer(owner_id));
        stmt.execute(rusqlite::params_from_iter(values.iter()))?
    } else {
        stmt.execute(rusqlite::params_from_iter(ids.iter()))?
    };
    Ok(affected)
}

#[allow(dead_code)]
pub fn archive_entries(conn: &Connection, table: &str, ids: &[i64]) -> rusqlite::Result<usize> {
    archive_entries_scoped(conn, table, ids, None)
}

