// SPDX-License-Identifier: MIT
use std::fs;
use std::path::PathBuf;

use super::paths::CortexPaths;

/// Returns the legacy database path: `~/cortex/cortex.db`.
pub fn legacy_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("cortex").join("cortex.db")
}

/// Migrate legacy DB from `~/cortex/cortex.db` to the canonical location.
/// Copies (never moves) to preserve the original as a safety net.
pub fn migrate_legacy_db(paths: &CortexPaths) -> Result<bool, String> {
    let legacy = legacy_db_path();
    if !legacy.exists() || paths.db.exists() {
        return Ok(false);
    }

    fs::create_dir_all(paths.db.parent().unwrap_or(&paths.home))
        .map_err(|e| format!("create dir: {e}"))?;

    fs::copy(&legacy, &paths.db).map_err(|e| format!("copy db: {e}"))?;

    // Copy WAL and SHM if present
    for ext in ["db-wal", "db-shm"] {
        let src = legacy.with_extension(ext);
        if src.exists() {
            let dst = paths.db.with_extension(ext);
            fs::copy(&src, &dst).map_err(|e| format!("copy {ext}: {e}"))?;
        }
    }

    // Verify integrity of the copy
    let conn =
        rusqlite::Connection::open(&paths.db).map_err(|e| format!("open migrated db: {e}"))?;
    let busy_timeout_ms = crate::db::SQLITE_BUSY_TIMEOUT_MS;
    conn.execute_batch(&format!("PRAGMA busy_timeout = {busy_timeout_ms};"))
        .map_err(|e| format!("configure migrated db busy timeout: {e}"))?;
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|e| format!("integrity check: {e}"))?;
    if check != "ok" {
        // Remove the bad copy, leave legacy intact
        let _ = fs::remove_file(&paths.db);
        return Err(format!("integrity check failed on migrated db: {check}"));
    }

    eprintln!(
        "[cortex] Migrated brain from {} to {}",
        legacy.display(),
        paths.db.display()
    );
    Ok(true)
}
