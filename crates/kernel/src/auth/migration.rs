use super::paths::CortexPaths;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn remove_incomplete_dest(db: &Path) {
    let _ = fs::remove_file(db);
    let _ = fs::remove_file(db.with_extension("db-wal"));
    let _ = fs::remove_file(db.with_extension("db-shm"));
}
pub fn legacy_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("cortex").join("cortex.db")
}
pub fn migrate_legacy_db(paths: &CortexPaths) -> Result<bool, String> {
    let legacy = legacy_db_path();
    if !legacy.exists() || paths.db.exists() {
        return Ok(false);
    }
    fs::create_dir_all(paths.db.parent().unwrap_or(&paths.home))
        .map_err(|e| format!("create dir: {e}"))?;
    // `fs::copy` overwrites. If another process created the dest between the
    // exists check and the copy, that new brain must not be replaced.
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.db)
    {
        Ok(_) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => return Err(format!("create dest db: {err}")),
    }
    if let Err(err) = fs::copy(&legacy, &paths.db) {
        remove_incomplete_dest(&paths.db);
        return Err(format!("copy db: {err}"));
    }
    for ext in ["db-wal", "db-shm"] {
        let src = legacy.with_extension(ext);
        if src.exists() {
            let dst = paths.db.with_extension(ext);
            if let Err(e) = fs::copy(&src, &dst) {
                remove_incomplete_dest(&paths.db);
                return Err(format!("copy {ext}: {e}"));
            }
        }
    }
    let conn = match rusqlite::Connection::open(&paths.db) {
        Ok(conn) => conn,
        Err(e) => {
            remove_incomplete_dest(&paths.db);
            return Err(format!("open migrated db: {e}"));
        }
    };
    let busy_timeout_ms = crate::db::SQLITE_BUSY_TIMEOUT_MS;
    if let Err(e) = conn.execute_batch(&format!("PRAGMA busy_timeout = {busy_timeout_ms};")) {
        drop(conn);
        remove_incomplete_dest(&paths.db);
        return Err(format!("configure migrated db busy timeout: {e}"));
    }
    let check: String = match conn.query_row("PRAGMA integrity_check", [], |row| row.get(0)) {
        Ok(check) => check,
        Err(e) => {
            drop(conn);
            remove_incomplete_dest(&paths.db);
            return Err(format!("integrity check: {e}"));
        }
    };
    if check != "ok" {
        drop(conn);
        remove_incomplete_dest(&paths.db);
        return Err(format!("integrity check failed on migrated db: {check}"));
    }
    eprintln!(
        "[cortex] Migrated brain from {} to {}",
        legacy.display(),
        paths.db.display()
    );
    Ok(true)
}
