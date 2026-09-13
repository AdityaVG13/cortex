use super::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
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
pub fn migrate_aging_columns_with_logging(conn: &Connection, log_success: bool) {
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
pub fn unix_now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(now).unwrap_or(i64::MAX)
}
pub fn should_attempt_best_effort_checkpoint(now_ms: i64, last_checkpoint_ms: i64) -> bool {
    now_ms.saturating_sub(last_checkpoint_ms) >= BEST_EFFORT_CHECKPOINT_MIN_INTERVAL_MS
}
pub fn should_attempt_truncate_checkpoint(now_ms: i64, last_truncate_ms: i64) -> bool {
    if last_truncate_ms <= 0 {
        return false;
    }
    now_ms.saturating_sub(last_truncate_ms) >= BEST_EFFORT_TRUNCATE_INTERVAL_MS
}
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
pub fn delete_expired_entries(conn: &Connection) -> rusqlite::Result<ExpiredCleanupCounts> {
    let memories_deleted = conn.execute(
        "DELETE FROM memories WHERE expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) < julianday('now')",
        [],
    )?;
    let decisions_deleted = conn.execute(
        "DELETE FROM decisions WHERE expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) < julianday('now')",
        [],
    )?;
    Ok(ExpiredCleanupCounts {
        memories_deleted,
        decisions_deleted,
    })
}
pub fn rebuild_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags)
         SELECT id, text, source, tags FROM memories WHERE status = 'active';
         INSERT OR IGNORE INTO decisions_fts(rowid, decision, context)
         SELECT id, decision, context FROM decisions WHERE status = 'active';",
    )?;
    Ok(())
}
pub fn reindex_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT INTO memories_fts(memories_fts) VALUES('delete-all');
         INSERT INTO decisions_fts(decisions_fts) VALUES('delete-all');",
    )?;
    rebuild_fts(conn)
}
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
pub fn verify_integrity(conn: &Connection) -> rusqlite::Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
}
pub fn quick_check(conn: &Connection) -> bool {
    conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map(|s| s.trim().eq_ignore_ascii_case("ok"))
        .unwrap_or(false)
}
pub fn auto_repair(db_path: &Path, timestamp: &str) -> Result<RepairResult, RepairError> {
    eprintln!(
        "[cortex] auto_repair: beginning dump-and-rebuild of {}",
        db_path.display()
    );
    let corrupt_conn = Connection::open(db_path).map_err(RepairError::OpenCorrupt)?;
    let busy_timeout_ms = SQLITE_BUSY_TIMEOUT_MS;
    let _ = corrupt_conn.execute_batch(&format!(
        r#"
        PRAGMA busy_timeout = {busy_timeout_ms};
        PRAGMA query_only = ON;
        "#
    ));
    pub const DATA_TABLES: &[&str] = &[
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
        // History and governance: losing these silently rewrites provenance.
        "traces",
        "versions",
        "head_state",
        "decision_conflicts",
        "boot_audits",
        "agent_feedback",
        "client_permissions",
        "entities",
        "entity_aliases",
        "entity_mentions",
        // Authoritative records (additive schema); FK order matters.
        "brain_meta",
        "scopes",
        "commits",
        "sources",
        "records",
        "revisions",
        "revision_parents",
        "record_heads",
        "revision_sources",
        "relations",
        "digests",
        "addresses",
        "threads",
        "thread_members",
        "obligations",
        "change_items",
        "outbox",
        "guard_epochs",
        "erasures",
        "operation_ledger",
    ];
    // Team identity tables. initialize_schema does not create these, and a
    // team-mode DB that loses them during repair silently boots in solo mode
    // with every member's credential hash destroyed (all ctx_ keys die, no
    // warning). Detect team mode up front, create the team schema in the
    // fresh DB, and salvage the identity rows alongside the data tables.
    const TEAM_IDENTITY_TABLES: &[&str] = &["config", "users", "teams", "team_members"];
    // Team-ness detection must not rely solely on the config mode row: when
    // config itself is the damaged table (its rows unreadable/lost),
    // current_mode falls back to "solo" and the identity tables would be
    // silently excluded from salvage -- every credential hash dropped with no
    // warning, the same loss bed9809 fixed in the fully-readable-config case.
    // The `users` table only exists in team-mode DBs (initialize_schema does
    // not create it), so its presence disambiguates the fallback. Every writer
    // stores exactly 'team' or 'solo' (create_team_mode_tables seed,
    // migrate_to_team_mode flip, the post-salvage guard downgrade), so a
    // READABLE row whose value is neither is payload-level damage to that
    // cell, not a verdict -- it takes the same inference as the unreadable
    // arm. A plainly readable 'solo' row keeps the exact solo verdict (no
    // inference for a DB already downgraded to solo before the corruption --
    // no false warnings), and a plainly readable 'team' row stays exact.
    let corrupt_team_mode = match corrupt_conn.query_row(
        "SELECT value FROM config WHERE key = 'mode' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(mode) if mode == "team" => true,
        Ok(mode) if mode == "solo" => false,
        Ok(_) | Err(_) => corrupt_conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='users' LIMIT 1",
                params![],
                |_| Ok(()),
            )
            .is_ok(),
    };
    let tables: Vec<&str> = if corrupt_team_mode {
        DATA_TABLES
            .iter()
            .copied()
            .chain(TEAM_IDENTITY_TABLES.iter().copied())
            .collect()
    } else {
        DATA_TABLES.to_vec()
    };
    // The fresh schema must exist BEFORE salvage so each table's INSERT column
    // list can be intersected with the columns the fresh DB actually has.
    // Boot-time migrations add columns (e.g. compressed_text, age_tier) that
    // initialize_schema alone does not; naming them in the INSERT fails every
    // row ("no column named ...") while exports still counted as recovered.
    let tmp_path = db_path.with_extension("repair_tmp");
    let _ = std::fs::remove_file(&tmp_path);
    let mut fresh = Connection::open(&tmp_path).map_err(RepairError::OpenFresh)?;
    configure(&fresh).map_err(RepairError::Import)?;
    initialize_schema(&fresh).map_err(RepairError::Import)?;
    // Boot-time schema the salvage loop intersects columns against.
    cortex_logic::traces::migrate_history_tables(&fresh).map_err(RepairError::Import)?;
    crate::graph::migrate_entity_tables(&fresh).map_err(RepairError::Import)?;
    super::run_pending_migrations_quiet(&mut fresh);
    super::records::ensure_authoritative_schema(&fresh).map_err(RepairError::Import)?;
    if corrupt_team_mode {
        create_team_mode_tables(&fresh).map_err(RepairError::Import)?;
        // create_team_mode_tables seeds config.mode='solo'; the salvage loop
        // inserts with INSERT OR IGNORE, so the seeded row would outcompete
        // the corrupt DB's mode='team' row. Drop the seed before salvage; if
        // the corrupt DB's config cannot be salvaged, current_mode falls back
        // to solo below and the post-salvage guard warns loudly.
        fresh
            .execute("DELETE FROM config WHERE key = 'mode'", [])
            .map_err(RepairError::Import)?;
    }
    fresh
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(RepairError::Import)?;
    const MAX_CONSECUTIVE_ROW_ERRORS: usize = 100;
    let mut memories_recovered = 0usize;
    let mut decisions_recovered = 0usize;
    for &table in &tables {
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
        let fresh_columns: Vec<String> = fresh
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(RepairError::Import)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(RepairError::Import)?
            .filter_map(|r| r.ok())
            .collect();
        let common_columns: Vec<String> = columns
            .iter()
            .filter(|c| fresh_columns.contains(c))
            .cloned()
            .collect();
        if common_columns.is_empty() {
            eprintln!("[cortex] auto_repair: table '{table}' has no columns in common with the fresh schema, skipping");
            continue;
        }
        let col_list = common_columns.join(", ");
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
        let mut row_values: Vec<Vec<String>> = Vec::new();
        let mut consecutive_row_errors = 0usize;
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    consecutive_row_errors = 0;
                    let mut vals: Vec<String> = Vec::new();
                    for i in 0..common_columns.len() {
                        use rusqlite::types::ValueRef;
                        let val = match row.get_ref(i) {
                            Ok(ValueRef::Null) => "NULL".to_string(),
                            Ok(ValueRef::Integer(n)) => n.to_string(),
                            Ok(ValueRef::Real(f)) => format!("{f}"),
                            Ok(ValueRef::Text(t)) => {
                                let s = String::from_utf8_lossy(t);
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
                    consecutive_row_errors += 1;
                    eprintln!("[cortex] auto_repair: row error in '{table}': {e} -- skipping row");
                    if consecutive_row_errors >= MAX_CONSECUTIVE_ROW_ERRORS {
                        eprintln!(
                            "[cortex] auto_repair: '{table}' hit {MAX_CONSECUTIVE_ROW_ERRORS} consecutive row errors -- abandoning remaining rows of this table"
                        );
                        break;
                    }
                    continue;
                }
            }
        }
        // Honest counts: "recovered" means rows actually inserted into the
        // fresh DB, never rows exported from the corrupt one. A dropped
        // INSERT OR IGNORE reports changes() == 0, so silent loss is visible
        // as inserted/exported in the log line.
        let exported = row_values.len();
        let mut inserted = 0usize;
        for vals in &row_values {
            let stmt = format!(
                "INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({});",
                vals.join(", ")
            );
            if let Err(e) = fresh.execute_batch(&stmt) {
                eprintln!("[cortex] auto_repair: insert skipped ({e}): {stmt:.80}");
                continue;
            }
            inserted += usize::try_from(fresh.changes()).unwrap_or(0);
        }
        eprintln!("[cortex] auto_repair: {inserted}/{exported} rows recovered into '{table}'");
        if table == "memories" {
            memories_recovered = inserted;
        } else if table == "decisions" {
            decisions_recovered = inserted;
        }
    }
    drop(corrupt_conn);
    fresh
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(RepairError::Import)?;
    fresh
        .execute_batch(
            "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags) \
             SELECT id, text, source, tags FROM memories; \
             INSERT OR IGNORE INTO decisions_fts(rowid, decision, context) \
             SELECT id, decision, context FROM decisions;",
        )
        .map_err(RepairError::Import)?;
    fresh
        .execute_batch("VACUUM;")
        .map_err(RepairError::Import)?;
    // Team-mode safety net: booting in team mode with an empty users roster
    // bricks every credential (all ctx_ keys fail, the caller-less runtime
    // token is admin-locked), which is worse than an honest solo downgrade.
    // If team identity did not fully survive the salvage, fall back to solo
    // and say so loudly instead of flipping silently.
    if corrupt_team_mode {
        let users_survived: bool = fresh
            .query_row("SELECT 1 FROM users LIMIT 1", [], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
            .unwrap_or(false);
        if current_mode(&fresh) != "team" || !users_survived {
            let _ = fresh.execute("INSERT INTO config (key, value) VALUES ('mode', 'solo') ON CONFLICT(key) DO UPDATE SET value = 'solo'", []);
            eprintln!(
                "[cortex] auto_repair: WARNING -- corrupt DB was in TEAM MODE but team identity (config/users) \
                 could not be fully salvaged. The repaired DB starts in SOLO MODE; all team API keys are invalid. \
                 Re-run `cortex setup --team` to recreate team mode (it will re-assign salvaged rows to the new owner). \
                 The pre-repair file will be preserved as {}",
                db_path.with_extension(format!("corrupt.{timestamp}")).display()
            );
        }
    }
    let integrity_ok = verify_integrity(&fresh).unwrap_or(false);
    drop(fresh);
    if !integrity_ok {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("[cortex] auto_repair: repaired DB failed integrity_check -- aborting");
        return Err(RepairError::RepairIntegrityFailed);
    }
    let corrupt_archive = db_path.with_extension(format!("corrupt.{timestamp}"));
    std::fs::rename(db_path, &corrupt_archive).map_err(RepairError::Io)?;
    std::fs::rename(&tmp_path, db_path).map_err(|e| {
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
