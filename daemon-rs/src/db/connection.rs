// SPDX-License-Identifier: MIT
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::{params, Connection, OptionalExtension};


use super::*;
pub(crate) const BEST_EFFORT_CHECKPOINT_MIN_INTERVAL_MS: i64 = 5_000;
pub(crate) const BEST_EFFORT_TRUNCATE_INTERVAL_MS: i64 = 5 * 60 * 1_000;
pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;
pub const SQLITE_WAL_AUTOCHECKPOINT_PAGES: u64 = 1_000;
pub(crate) static LAST_BEST_EFFORT_CHECKPOINT_MS: AtomicI64 = AtomicI64::new(0);
pub(crate) static LAST_BEST_EFFORT_TRUNCATE_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteVecStatus {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

static SQLITE_VEC_REGISTRATION: OnceLock<Result<(), String>> = OnceLock::new();

pub(crate) fn ensure_sqlite_vec_registered() -> Result<(), String> {
    SQLITE_VEC_REGISTRATION
        .get_or_init(|| {
            type SqliteVecEntryPoint = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;

            unsafe extern "C" {
                #[link_name = "sqlite3_vec_init"]
                pub(crate) fn sqlite3_vec_init_auto_extension(
                    db: *mut rusqlite::ffi::sqlite3,
                    err_msg: *mut *mut std::os::raw::c_char,
                    api: *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int;
            }

            // Keep the `sqlite-vec` crate referenced: its build script supplies
            // the native `sqlite_vec0` library that defines this symbol.
            let _sqlite_vec_symbol: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
            let init: SqliteVecEntryPoint = sqlite3_vec_init_auto_extension;
            // SAFETY: `init` points to `sqlite3_vec_init` with SQLite's
            // required auto-extension ABI and remains valid for the process.
            let rc = unsafe { rusqlite::ffi::sqlite3_auto_extension(Some(init)) };
            if rc == 0 {
                Ok(())
            } else {
                Err(format!("sqlite3_auto_extension returned {rc}"))
            }
        })
        .clone()
}

/// Result of an auto-repair attempt.
#[derive(Debug)]
pub struct RepairResult {
    pub memories_recovered: usize,
    pub decisions_recovered: usize,
    pub corrupt_db_path: std::path::PathBuf,
}

/// Error type for auto-repair failures.
pub enum RepairError {
    /// Could not open the corrupted DB for reading.
    OpenCorrupt(rusqlite::Error),
    /// Could not create a fresh DB for the repaired copy.
    OpenFresh(rusqlite::Error),
    /// Data export from the corrupted DB failed.
    Export(rusqlite::Error),
    /// Import into the fresh DB failed.
    Import(rusqlite::Error),
    /// The repaired DB itself failed integrity_check.
    RepairIntegrityFailed,
    /// File-system rename/copy operations failed.
    Io(std::io::Error),
}

impl std::fmt::Debug for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepairError::OpenCorrupt(e) => write!(f, "RepairError::OpenCorrupt({e})"),
            RepairError::OpenFresh(e) => write!(f, "RepairError::OpenFresh({e})"),
            RepairError::Export(e) => write!(f, "RepairError::Export({e})"),
            RepairError::Import(e) => write!(f, "RepairError::Import({e})"),
            RepairError::RepairIntegrityFailed => write!(f, "RepairError::RepairIntegrityFailed"),
            RepairError::Io(e) => write!(f, "RepairError::Io({e})"),
        }
    }
}

/// Open a SQLite connection at the given path.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let _ = ensure_sqlite_vec_registered();
    Connection::open(path)
}

pub fn sqlite_vec_status(conn: &Connection) -> SqliteVecStatus {
    if let Err(error) = ensure_sqlite_vec_registered() {
        return SqliteVecStatus {
            available: false,
            version: None,
            error: Some(error),
        };
    }

    match conn.query_row("SELECT vec_version()", [], |row| row.get::<_, String>(0)) {
        Ok(version) => SqliteVecStatus {
            available: true,
            version: Some(version),
            error: None,
        },
        Err(error) => SqliteVecStatus {
            available: false,
            version: None,
            error: Some(error.to_string()),
        },
    }
}

pub(crate) fn env_u64_clamped(name: &str, default: u64, min: u64, max: u64) -> u64 {
    let parsed = std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default);
    parsed.clamp(min, max)
}

/// Apply WAL mode, NORMAL synchronous writes, foreign-key enforcement, and
/// bounded SQLite lock waits.
///
/// NOTE: PRAGMA synchronous=NORMAL is safe with WAL mode. From SQLite docs:
/// - FULL: Extra safety at the cost of significant performance (OS crash protection)
/// - NORMAL: All changes are synced before passing control to caller at critical moments
///   (process crash protection). With WAL checkpoint every 10s, data loss is limited to <10s.
///   This is the recommended setting for WAL mode workloads.
pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    // Defaults tuned for mixed desktop + daemon workloads; both are overridable
    // to let operators adapt for RAM-constrained or high-throughput hosts.
    let mmap_size = env_u64_clamped(
        "CORTEX_DB_MMAP_SIZE_BYTES",
        268_435_456,
        64 * 1024 * 1024,
        4 * 1024 * 1024 * 1024,
    );
    let cache_size_kib = env_u64_clamped("CORTEX_DB_CACHE_SIZE_KIB", 12_000, 2_000, 131_072);
    let cache_size = -(cache_size_kib as i64);
    let busy_timeout_ms = SQLITE_BUSY_TIMEOUT_MS;
    let wal_autocheckpoint_pages = SQLITE_WAL_AUTOCHECKPOINT_PAGES;
    let pragmas = format!(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = {busy_timeout_ms};
        PRAGMA foreign_keys = ON;
        PRAGMA mmap_size = {mmap_size};
        PRAGMA cache_size = {cache_size};
        PRAGMA temp_store = MEMORY;
        PRAGMA wal_autocheckpoint = {wal_autocheckpoint_pages};
        "#
    );
    conn.execute_batch(&pragmas)?;
    Ok(())
}

pub(crate) type MigrationDef = (&'static str, &'static str);

