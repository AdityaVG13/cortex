use super::paths::{
    default_home_root, CortexPaths, CORTEX_DIR_NAME, CORTEX_GLOBAL_LOCK_HOME_ENV,
    CORTEX_GLOBAL_LOCK_NAME,
};
use fs2::FileExt;
use std::fs;
use std::path::{Path, PathBuf};
pub fn acquire_daemon_lock(paths: &CortexPaths) -> Result<fs::File, String> {
    fs::create_dir_all(&paths.home).map_err(|e| format!("create home: {e}"))?;
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)
        .map_err(|e| format!("open lock: {e}"))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| "another cortex instance holds the lock".to_string())?;
    Ok(lock_file)
}
fn global_lock_path() -> PathBuf {
    if let Ok(explicit) = std::env::var(CORTEX_GLOBAL_LOCK_HOME_ENV) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join(CORTEX_GLOBAL_LOCK_NAME);
        }
    }
    default_home_root()
        .join(CORTEX_DIR_NAME)
        .join(CORTEX_GLOBAL_LOCK_NAME)
}
fn acquire_global_daemon_lock_at(path: &Path) -> Result<fs::File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create global lock dir: {e}"))?;
    }
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("open global lock: {e}"))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| "another cortex instance holds the lock".to_string())?;
    Ok(lock_file)
}
pub fn acquire_global_daemon_lock() -> Result<fs::File, String> {
    acquire_global_daemon_lock_at(&global_lock_path())
}
