mod repair;

use super::*;
pub use repair::auto_repair;
use rusqlite::{Connection, OptionalExtension};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
pub fn migrate_focus_table(conn: &Connection) {
    let sql = "CREATE TABLE IF NOT EXISTS focus_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, agent TEXT NOT NULL DEFAULT 'unknown', status TEXT NOT NULL DEFAULT 'open', raw_entries TEXT NOT NULL DEFAULT '[]', summary TEXT, started_at TEXT DEFAULT (datetime('now')), ended_at TEXT, tokens_before INTEGER DEFAULT 0, tokens_after INTEGER DEFAULT 0)";
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
    let delete = |table: &str| {
        conn.execute(
            &format!("DELETE FROM {table} WHERE {}", super::EXPIRED_SQL),
            [],
        )
    };
    Ok(ExpiredCleanupCounts {
        memories_deleted: delete("memories")?,
        decisions_deleted: delete("decisions")?,
    })
}
pub fn rebuild_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags) SELECT id, text, source, tags FROM memories WHERE status = 'active'; INSERT OR IGNORE INTO decisions_fts(rowid, decision, context) SELECT id, decision, context FROM decisions WHERE status = 'active';")?;
    Ok(())
}
pub fn reindex_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("INSERT INTO memories_fts(memories_fts) VALUES('delete-all'); INSERT INTO decisions_fts(decisions_fts) VALUES('delete-all');")?;
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
    conn.execute("INSERT OR IGNORE INTO schema_migrations (version, name, applied_at) VALUES ('fts_seeded_v1', 'fts_seeded', datetime('now'))", [])?;
    Ok(true)
}
fn pragma_reports_ok(conn: &Connection, pragma: &str) -> rusqlite::Result<bool> {
    let result: String = conn.query_row(pragma, [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
}
pub fn verify_integrity(conn: &Connection) -> rusqlite::Result<bool> {
    pragma_reports_ok(conn, "PRAGMA integrity_check")
}
pub fn quick_check(conn: &Connection) -> bool {
    pragma_reports_ok(conn, "PRAGMA quick_check").unwrap_or(false)
}
