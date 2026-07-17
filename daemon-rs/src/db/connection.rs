use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::AtomicI64;
use std::sync::OnceLock;
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
                    db: *mut rusqlite::ffi::sqlite3, err_msg: *mut *mut std::os::raw::c_char,
                    api: *const rusqlite::ffi::sqlite3_api_routines,
                ) -> std::os::raw::c_int;
            }
            let _sqlite_vec_symbol: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
            let init: SqliteVecEntryPoint = sqlite3_vec_init_auto_extension;
            let rc = unsafe { rusqlite::ffi::sqlite3_auto_extension(Some(init)) };
            if rc == 0 {
                Ok(())
            } else {
                Err(format!("sqlite3_auto_extension returned {rc}"))
            }
        })
        .clone()
}
#[derive(Debug)]
pub struct RepairResult {
    pub memories_recovered: usize,
    pub decisions_recovered: usize,
    pub corrupt_db_path: std::path::PathBuf,
}
pub enum RepairError {
    OpenCorrupt(rusqlite::Error),
    OpenFresh(rusqlite::Error),
    Export(rusqlite::Error),
    Import(rusqlite::Error),
    RepairIntegrityFailed,
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
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let _ = ensure_sqlite_vec_registered();
    Connection::open(path)
}
pub fn sqlite_vec_status(conn: &Connection) -> SqliteVecStatus {
    if let Err(error) = ensure_sqlite_vec_registered() {
        return SqliteVecStatus { available: false, version: None, error: Some(error) };
    }
    match conn.query_row("SELECT vec_version()", [], |row| row.get::<_, String>(0)) {
        Ok(version) => SqliteVecStatus { available: true, version: Some(version), error: None },
        Err(error) => SqliteVecStatus { available: false, version: None, error: Some(error.to_string()) },
    }
}
pub(crate) fn env_u64_clamped(name: &str, default: u64, min: u64, max: u64) -> u64 {
    let parsed = std::env::var(name).ok().and_then(|raw| raw.trim().parse::<u64>().ok()).unwrap_or(default);
    parsed.clamp(min, max)
}
pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    let mmap_size = env_u64_clamped("CORTEX_DB_MMAP_SIZE_BYTES", 268_435_456, 64 * 1024 * 1024, 4 * 1024 * 1024 * 1024);
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
