use rusqlite::Connection;
use std::path::Path;
use std::sync::atomic::AtomicI64;
pub const BEST_EFFORT_CHECKPOINT_MIN_INTERVAL_MS: i64 = 5_000;
pub const BEST_EFFORT_TRUNCATE_INTERVAL_MS: i64 = 5 * 60 * 1_000;
pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;
pub const SQLITE_WAL_AUTOCHECKPOINT_PAGES: u64 = 1_000;
pub static LAST_BEST_EFFORT_CHECKPOINT_MS: AtomicI64 = AtomicI64::new(0);
pub static LAST_BEST_EFFORT_TRUNCATE_MS: AtomicI64 = AtomicI64::new(0);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteVecStatus {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}
pub fn ensure_sqlite_vec_registered() -> Result<(), String> {
    Err("sqlite-vec disabled; clock-quorum recall does not load vec0".into())
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
/// Version string of the SQLite library actually linked into this process.
/// The Cargo dependency name (`rusqlite` + `bundled`) does not identify it.
pub fn sqlite_version() -> String {
    rusqlite::version().to_string()
}
/// Whether a SQLite release contains the WAL-reset race fix documented at
/// https://sqlite.org/wal.html: fixed in 3.51.3 and backported to 3.44.6 and
/// 3.50.7. Anything else in 3.7.0..=3.51.2 is affected; unparsable input is
/// treated as unfixed. This gates concurrent-local (multi-connection) claims.
pub fn sqlite_wal_reset_fixed(version: &str) -> bool {
    let mut parts = version.trim().split('.').map(|p| p.parse::<u32>().ok());
    let (Some(Some(major)), Some(Some(minor))) = (parts.next(), parts.next()) else {
        return false;
    };
    let patch = parts.next().flatten().unwrap_or(0);
    if major != 3 {
        return major > 3;
    }
    match minor {
        44 => patch >= 6,
        50 => patch >= 7,
        51 => patch >= 3,
        m => m > 51,
    }
}
pub fn sqlite_vec_status(_conn: &Connection) -> SqliteVecStatus {
    SqliteVecStatus {
        available: false,
        version: None,
        error: Some("sqlite-vec disabled".into()),
    }
}
pub fn env_u64_clamped(name: &str, default: u64, min: u64, max: u64) -> u64 {
    let parsed = std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default);
    parsed.clamp(min, max)
}
/// Durability profile for the authoritative connection.
/// `durable` (default) = `synchronous=FULL`: commit returns after the WAL is
/// fsynced, so the acknowledgement covers power loss on honest hardware.
/// `fast` = `synchronous=NORMAL`: survives process death only; every Receipt
/// then reports `ack_profile=process_crash`. Selected by `CORTEX_DURABILITY`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityProfile {
    Durable,
    Fast,
}
impl DurabilityProfile {
    pub fn from_env() -> Self {
        Self::parse(std::env::var("CORTEX_DURABILITY").ok().as_deref())
    }
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("fast") | Some("process_crash") | Some("normal") => Self::Fast,
            _ => Self::Durable,
        }
    }
    pub fn synchronous(self) -> &'static str {
        match self {
            Self::Durable => "FULL",
            Self::Fast => "NORMAL",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Fast => "fast",
        }
    }
}
pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    configure_with_profile(conn, DurabilityProfile::from_env())
}
pub fn configure_with_profile(
    conn: &Connection,
    profile: DurabilityProfile,
) -> rusqlite::Result<()> {
    let synchronous = profile.synchronous();
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
        PRAGMA synchronous = {synchronous};
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
pub type MigrationDef = (&'static str, &'static str);

fn savepoint_ident(name: &'static str) {
    assert!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
        "savepoint name must be a SQLite identifier"
    );
}

/// `BEGIN IMMEDIATE` that rolls back on drop unless [`ImmediateWrite::commit`]
/// succeeds. Covers error `return` and unwind; a bare `execute_batch("BEGIN
/// IMMEDIATE")` without this (or rusqlite `Transaction`) leaves the connection
/// in a write transaction.
#[must_use]
pub struct ImmediateWrite<'a> {
    conn: &'a Connection,
    finished: bool,
}

impl<'a> ImmediateWrite<'a> {
    pub fn begin(conn: &'a Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self {
            conn,
            finished: false,
        })
    }

    pub fn commit(mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for ImmediateWrite<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.conn.execute_batch("ROLLBACK");
            // Same as SqliteTx::abort: a failed ROLLBACK with a live
            // statement must not skip retry, or the write mutex is returned
            // still inside the transaction.
            if !self.conn.is_autocommit() {
                let _ = self.conn.execute_batch("ROLLBACK");
            }
        }
    }
}

/// Named `SAVEPOINT` that rolls back on drop unless [`SqliteSavepoint::release`]
/// succeeds. Use when the body only needs `&Connection` so the guard can stay
/// live across `?` / panic / future cancel.
#[must_use]
pub struct SqliteSavepoint<'a> {
    conn: &'a Connection,
    name: &'static str,
    released: bool,
}

impl<'a> SqliteSavepoint<'a> {
    pub fn enter(conn: &'a Connection, name: &'static str) -> rusqlite::Result<Self> {
        savepoint_ident(name);
        conn.execute_batch(&format!("SAVEPOINT {name}"))?;
        Ok(Self {
            conn,
            name,
            released: false,
        })
    }

    pub fn release(mut self) -> rusqlite::Result<()> {
        self.conn
            .execute_batch(&format!("RELEASE {}", self.name))?;
        self.released = true;
        Ok(())
    }
}

impl Drop for SqliteSavepoint<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.conn.execute_batch(&format!(
                "ROLLBACK TO {}; RELEASE {}",
                self.name, self.name
            ));
        }
    }
}

/// `SAVEPOINT` around a body that needs `&mut Connection` (so a live
/// [`SqliteSavepoint`] cannot coexist with the exclusive borrow). Rolls back
/// on `Err` and on unwind; `RELEASE` only on `Ok`.
pub fn with_savepoint_mut<T, E>(
    conn: &mut Connection,
    name: &'static str,
    f: impl FnOnce(&mut Connection) -> Result<T, E>,
    map_sql: impl Fn(rusqlite::Error) -> E,
) -> Result<T, E> {
    savepoint_ident(name);
    conn.execute_batch(&format!("SAVEPOINT {name}"))
        .map_err(&map_sql)?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(conn))) {
        Ok(Ok(value)) => {
            if let Err(err) = conn.execute_batch(&format!("RELEASE {name}")) {
                // RELEASE failed with the body already applied. Roll the
                // named savepoint back so this connection is not returned to
                // the write mutex still inside a transaction (the next
                // BEGIN/SAVEPOINT would fail or join leftover writes).
                let _ = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
                return Err(map_sql(err));
            }
            Ok(value)
        }
        Ok(Err(err)) => {
            let _ = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
            Err(err)
        }
        Err(payload) => {
            let _ = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
            std::panic::resume_unwind(payload);
        }
    }
}
