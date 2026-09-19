use crate::protocol::nonempty_opt;
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

/// Fail-closed `SELECT COUNT(*)` (or any single i64). Distinct from backup/debt
/// census helpers that substitute 0 or a pressure cap on read failure.
pub fn count_sql(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<i64, String> {
    conn.query_row(sql, params, |r| r.get(0))
        .map_err(|e| e.to_string())
}

/// Census COUNT that treats an unreadable query as empty. Distinct from
/// `count_sql`, which fails closed, and from debt pressure which substitutes
/// the hard job cap.
pub fn count_or_zero(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
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
        match nonempty_opt(raw).map(|v| v.to_ascii_lowercase()).as_deref() {
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
    // The busy timeout must apply before any lock-taking pragma runs:
    // `journal_mode = WAL` can busy under concurrent boot, and a SQL-set
    // timeout later in this same batch would come too late. Set it via API
    // first (no lock needed), then run the batch with bounded busy retries.
    let _ = conn.busy_timeout(std::time::Duration::from_millis(busy_timeout_ms));
    execute_batch_with_busy_retry(
        conn,
        &format!(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = {synchronous}; PRAGMA busy_timeout = {busy_timeout_ms}; PRAGMA foreign_keys = ON; PRAGMA mmap_size = {mmap_size}; PRAGMA cache_size = {cache_size}; PRAGMA temp_store = MEMORY; PRAGMA wal_autocheckpoint = {wal_autocheckpoint_pages};"
        ),
    )
}

/// `execute_batch` with the shared bounded busy policy, for setup-time SQL
/// (pragmas, schema) that can contend under concurrent boot.
pub(crate) fn execute_batch_with_busy_retry(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    let mut attempt = 0;
    loop {
        match conn.execute_batch(sql) {
            Err(err) if is_busy_error(&err) && attempt + 1 < SAVEPOINT_BUSY_RETRIES => {
                attempt += 1;
                sleep_busy_backoff(attempt);
            }
            other => return other,
        }
    }
}
pub type MigrationDef = (&'static str, &'static str);

fn rollback_savepoint(conn: &Connection, name: &str) {
    let _ = conn.execute_batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
}

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
        self.conn.execute_batch(&format!("RELEASE {}", self.name))?;
        self.released = true;
        Ok(())
    }
}

impl Drop for SqliteSavepoint<'_> {
    fn drop(&mut self) {
        if !self.released {
            rollback_savepoint(self.conn, self.name);
        }
    }
}

/// `SAVEPOINT` around a body that needs `&mut Connection` (so a live
/// [`SqliteSavepoint`] cannot coexist with the exclusive borrow). Rolls back
/// on `Err` and on unwind; `RELEASE` only on `Ok`.
///
/// Multi-process write safety: when autocommit, the savepoint opens inside
/// `BEGIN IMMEDIATE`, so lock contention surfaces as plain `BUSY` (which the
/// busy timeout retries) instead of `BUSY_SNAPSHOT` on deferred upgrade
/// (which it does not). `BEGIN` additionally retries on busy with bounded
/// backoff. Body errors (`E`) and `COMMIT` outcomes never retry: the former
/// are application decisions, the latter are commit-unknown.
pub fn with_savepoint_mut<T, E>(
    conn: &mut Connection,
    name: &'static str,
    f: impl FnOnce(&mut Connection) -> Result<T, E>,
    map_sql: impl Fn(rusqlite::Error) -> E,
) -> Result<T, E> {
    savepoint_ident(name);
    let opened = open_write_txn(conn, name, &map_sql)?;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(conn))) {
        Ok(Ok(value)) => {
            if let Err(err) = conn.execute_batch(&format!("RELEASE {name}")) {
                // RELEASE failed with the body already applied. Roll back so
                // this connection is not returned to the write mutex still
                // inside a transaction (the next BEGIN/SAVEPOINT would fail
                // or join leftover writes).
                abort_write_txn(conn, name, opened);
                return Err(map_sql(err));
            }
            if opened {
                // Commit-unknown: never retry (a retry could double-apply).
                if let Err(err) = conn.execute_batch("COMMIT") {
                    let _ = conn.execute_batch("ROLLBACK");
                    return Err(map_sql(err));
                }
            }
            Ok(value)
        }
        Ok(Err(err)) => {
            abort_write_txn(conn, name, opened);
            Err(err)
        }
        Err(payload) => {
            abort_write_txn(conn, name, opened);
            std::panic::resume_unwind(payload);
        }
    }
}

pub(crate) const SAVEPOINT_BUSY_RETRIES: usize = 8;

fn open_write_txn<E>(
    conn: &mut Connection,
    name: &'static str,
    map_sql: &impl Fn(rusqlite::Error) -> E,
) -> Result<bool, E> {
    if !conn.is_autocommit() {
        // Nested inside a caller's transaction: plain savepoint as before.
        conn.execute_batch(&format!("SAVEPOINT {name}"))
            .map_err(map_sql)?;
        return Ok(false);
    }
    let mut attempt = 0usize;
    loop {
        match conn.execute_batch("BEGIN IMMEDIATE") {
            Ok(()) => break,
            Err(err) if is_busy_error(&err) && attempt + 1 < SAVEPOINT_BUSY_RETRIES => {
                attempt += 1;
                // Bounded ~250ms on top of the connection busy timeout.
                sleep_busy_backoff(attempt);
            }
            Err(err) => return Err(map_sql(err)),
        }
    }
    // Inside our own IMMEDIATE txn the savepoint cannot hit busy: we hold
    // the write lock for the whole body.
    conn.execute_batch(&format!("SAVEPOINT {name}"))
        .map_err(map_sql)?;
    Ok(true)
}

fn abort_write_txn(conn: &mut Connection, name: &'static str, opened: bool) {
    if opened {
        let _ = conn.execute_batch("ROLLBACK");
    } else {
        rollback_savepoint(conn, name);
    }
}

pub(crate) fn is_busy_error(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == rusqlite::ErrorCode::DatabaseBusy
    )
}

/// Bounded busy backoff shared by write transactions and contention-sensitive
/// reads: 2, 4, 8 … ms with deterministic jitter. Attempt counts from 1.
pub(crate) fn sleep_busy_backoff(attempt: usize) {
    let backoff = 2u64
        .saturating_pow(attempt.min(6) as u32)
        .min(128)
        .saturating_add((attempt as u64 * 7) % 13);
    std::thread::sleep(std::time::Duration::from_millis(backoff));
}
