// SPDX-License-Identifier: MIT
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::auth::CortexPaths;

// ─── Supporting types (ported from embedded_daemon.rs) ───────────────────────

/// A single entry in the per-session recall history, recording what was queried
/// and when (Unix milliseconds).
#[derive(Clone, Debug)]
pub struct RecallHistoryEntry {
    pub query: String,
    pub timestamp: i64,
}

/// A cached recall result set.  `expires_at` is a Unix-millisecond deadline
/// after which the entry should be discarded.
#[derive(Clone, Debug)]
pub struct PreCacheEntry {
    pub query: String,
    /// Serialised recall results — stored as `Value` so this module does not
    /// need to know about the full recall pipeline types.
    pub results: Value,
    pub expires_at: i64,
}

/// A typed event broadcast to all SSE subscribers.
#[derive(Clone, Debug)]
pub struct DaemonEvent {
    pub event_type: String,
    // Event payloads are retained on the bus for future internal consumers,
    // but the public SSE stream currently redacts them before emission.
    #[allow(dead_code)]
    pub data: Value,
}

#[derive(Clone, Debug)]
pub enum SqliteVecRouteMode {
    Baseline,
    Trial,
    Primary,
}

impl SqliteVecRouteMode {
    fn from_env() -> Self {
        match std::env::var("CORTEX_SQLITE_VEC_ROUTE") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "baseline" | "off" | "disabled" => Self::Baseline,
                "trial" | "canary" | "sampled" => Self::Trial,
                "primary" | "vec0" | "production" | "on" => Self::Primary,
                unknown => {
                    eprintln!(
                        "[cortex] WARNING: invalid CORTEX_SQLITE_VEC_ROUTE={unknown:?}; using primary"
                    );
                    Self::Primary
                }
            },
            Err(_) => Self::Primary,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Trial => "trial",
            Self::Primary => "primary",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SqliteVecCanaryConfig {
    pub trial_percent: u8,
    pub force_off: bool,
    pub route_mode: SqliteVecRouteMode,
}

impl SqliteVecCanaryConfig {
    fn from_env() -> Self {
        let route_mode = SqliteVecRouteMode::from_env();
        let trial_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT")
            .ok()
            .and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return None;
                }
                match trimmed.parse::<u8>() {
                    Ok(percent) => Some(percent.min(100)),
                    Err(_) => {
                        eprintln!(
                            "[cortex] WARNING: invalid CORTEX_SQLITE_VEC_TRIAL_PERCENT={trimmed:?}; using 0"
                        );
                        Some(0)
                    }
                }
            })
            .unwrap_or(0);
        let force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF")
            .ok()
            .is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            });
        Self {
            trial_percent,
            force_off,
            route_mode,
        }
    }

    pub fn effective_route_mode(&self) -> SqliteVecRouteMode {
        if self.force_off {
            SqliteVecRouteMode::Baseline
        } else {
            self.route_mode.clone()
        }
    }
}

const READ_POOL_SIZE_ENV: &str = "CORTEX_DB_READ_POOL_SIZE";
const READ_POOL_DEFAULT_MIN: usize = 4;
const READ_POOL_DEFAULT_MAX: usize = 16;
const READ_POOL_HARD_MAX: usize = 32;
const READ_POOL_HARD_MIN: usize = 2;

pub type ReadConnLockFuture<'a> =
    Pin<Box<dyn Future<Output = tokio::sync::MutexGuard<'a, Connection>> + Send + 'a>>;

/// Shared read handle abstraction so runtime can use a pooled implementation
/// while tests and fixtures can continue to inject a single Mutex connection.
pub trait ReadConnectionProvider: Send + Sync {
    fn lock<'a>(&'a self) -> ReadConnLockFuture<'a>;

    fn pool_size(&self) -> usize {
        1
    }
}

impl ReadConnectionProvider for Mutex<Connection> {
    fn lock<'a>(&'a self) -> ReadConnLockFuture<'a> {
        Box::pin(async move { tokio::sync::Mutex::lock(self).await })
    }
}

struct ReadConnectionPool {
    connections: Vec<Mutex<Connection>>,
    next_index: AtomicUsize,
}

impl ReadConnectionPool {
    fn new(connections: Vec<Connection>) -> Self {
        assert!(
            !connections.is_empty(),
            "read connection pool requires at least one connection"
        );
        Self {
            connections: connections.into_iter().map(Mutex::new).collect(),
            next_index: AtomicUsize::new(0),
        }
    }
}

impl ReadConnectionProvider for ReadConnectionPool {
    fn lock<'a>(&'a self) -> ReadConnLockFuture<'a> {
        let idx = self.next_index.fetch_add(1, Ordering::Relaxed) % self.connections.len();
        Box::pin(async move { self.connections[idx].lock().await })
    }

    fn pool_size(&self) -> usize {
        self.connections.len()
    }
}

fn derive_read_pool_size(configured: Option<usize>, cpu_hint: Option<usize>) -> usize {
    let default = cpu_hint
        .unwrap_or(READ_POOL_DEFAULT_MIN)
        .clamp(READ_POOL_DEFAULT_MIN, READ_POOL_DEFAULT_MAX);
    configured
        .unwrap_or(default)
        .clamp(READ_POOL_HARD_MIN, READ_POOL_HARD_MAX)
}

fn read_pool_size_from_env() -> usize {
    let configured = std::env::var(READ_POOL_SIZE_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok());
    let cpu_hint = std::thread::available_parallelism()
        .ok()
        .map(|cpus| cpus.get());
    derive_read_pool_size(configured, cpu_hint)
}

fn open_query_only_connection(db_path: &Path) -> Result<Connection, String> {
    let read_conn =
        crate::db::open(db_path).map_err(|e| format!("Failed to open read connection: {e}"))?;
    crate::db::configure(&read_conn)
        .map_err(|e| format!("Failed to configure read connection: {e}"))?;
    read_conn
        .execute_batch("PRAGMA query_only = ON;")
        .map_err(|e| e.to_string())?;
    Ok(read_conn)
}

// ─── Shared application state ─────────────────────────────────────────────────

/// Shared state threaded through every Axum handler via `axum::extract::State`.
///
/// All fields are cheaply `Clone`able — most are wrapped in `Arc`.
#[derive(Clone)]
pub struct RuntimeState {
    /// SQLite write connection -- used by store, forget, resolve, diary, indexer.
    pub db: Arc<Mutex<Connection>>,
    /// SQLite read connection provider -- used by recall, peek, health, digest, boot.
    /// Runtime uses a small pool of query-only connections so concurrent reads do
    /// not serialize on one async mutex.
    pub db_read: Arc<dyn ReadConnectionProvider>,
    /// Auth token loaded from or written to the resolved runtime token path.
    pub token: Arc<String>,
    /// Broadcast channel for SSE events; clone the sender to fan-out.
    pub events: broadcast::Sender<DaemonEvent>,
    /// Monotonic counter for MCP call IDs.
    pub mcp_calls: Arc<AtomicU64>,
    /// Active MCP sessions: session-id → last-heartbeat (Unix seconds).
    #[allow(dead_code)]
    pub mcp_sessions: Arc<Mutex<HashMap<String, i64>>>,
    /// Per-agent recall history, capped at MAX_RECALL_HISTORY entries.
    pub recall_history: Arc<Mutex<HashMap<String, Vec<RecallHistoryEntry>>>>,
    /// Short-lived pre-warmed recall cache.
    pub pre_cache: Arc<Mutex<HashMap<String, PreCacheEntry>>>,
    /// Tracks which content hashes have been served to each agent recently.
    /// Maps hash → Unix-ms timestamp. Entries older than SERVED_TTL_MS are
    /// evicted, so content can be re-served after the cooldown.
    pub served_content: Arc<Mutex<HashMap<String, HashMap<u32, i64>>>>,
    /// Sending half of the graceful-shutdown oneshot.  The `/shutdown` endpoint
    /// takes this and fires it; the Axum server listens on the receiving half.
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// The user's home directory (used when constructing runtime paths).
    pub home: std::path::PathBuf,
    /// Absolute path of the SQLite database file.
    #[allow(dead_code)]
    pub db_path: std::path::PathBuf,
    /// Absolute path of the runtime auth token file.
    pub token_path: std::path::PathBuf,
    /// Absolute path of the runtime PID file.
    pub pid_path: std::path::PathBuf,
    /// Active HTTP port for this daemon instance.
    pub port: u16,
    /// In-process ONNX embedding engine (None if model not downloaded yet).
    pub embedding_engine: Option<Arc<crate::embeddings::EmbeddingEngine>>,
    /// Per-IP sliding-window rate limiter.
    pub rate_limiter: crate::rate_limit::RateLimiter,
    /// True when running with team-mode schema enabled.
    pub team_mode: bool,
    /// Default owner used for owner-scoped conductor rows.
    pub default_owner_id: Option<i64>,
    /// Team-mode API key hashes loaded from `users` for Argon2 verification.
    /// Wrapped in RwLock so admin endpoints can add/remove keys at runtime.
    pub team_api_key_hashes: Arc<std::sync::RwLock<Vec<(i64, String)>>>,
    /// Set to true when ONNX embedding fails at runtime (graceful degradation).
    pub degraded_mode: Arc<AtomicBool>,
    /// Set to true when a runtime `quick_check` detects B-tree corruption.
    /// Exposed on the `/health` endpoint as `db_corrupted`.
    pub db_corrupted: Arc<AtomicBool>,
    /// Readiness gate for daemon startup sequencing.
    /// `/readiness` reports this directly while `/health` remains diagnostic.
    pub readiness: Arc<AtomicBool>,
    /// Path for buffering writes when daemon is unreachable in proxy mode.
    /// Used by mcp_proxy via cortex_dir() directly; kept here for discoverability.
    #[allow(dead_code)]
    pub write_buffer_path: std::path::PathBuf,
    /// Guarded sqlite-vec semantic trial routing controls.
    pub sqlite_vec_canary: SqliteVecCanaryConfig,
}

impl RuntimeState {
    /// Broadcast an event to all current SSE subscribers.  Silently drops the
    /// result — a send error just means there are no active subscribers.
    pub fn emit(&self, event_type: &str, data: Value) {
        let _ = self.events.send(DaemonEvent {
            event_type: event_type.to_string(),
            data,
        });
    }

    /// Increment the MCP call counter and return the new value.
    pub fn next_mcp_call(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.mcp_calls.fetch_add(1, Ordering::SeqCst) + 1
    }
}

// ─── Initialisation ───────────────────────────────────────────────────────────

/// Open the database, apply schema migrations, generate an auth token, and
/// assemble the shared `RuntimeState`.
///
/// Returns `(state, shutdown_rx)`.  Pass `shutdown_rx` to Axum's
/// `with_graceful_shutdown` so the server exits cleanly when the `/shutdown`
/// endpoint fires.
pub fn initialize(
    paths: &CortexPaths,
    allow_token_rotation: bool,
) -> Result<(RuntimeState, oneshot::Receiver<()>), String> {
    let db_path = &paths.db;
    // 1. Open and configure the database.
    let conn = crate::db::open(db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;

    crate::db::configure(&conn).map_err(|e| format!("Failed to configure database: {e}"))?;

    crate::db::initialize_schema(&conn).map_err(|e| format!("Failed to initialise schema: {e}"))?;

    // 2. Startup integrity gate.
    // Use fast quick_check on boot for low-latency restarts. Only escalate to
    // full integrity_check + auto-repair when quick_check fails.
    if crate::db::quick_check(&conn) {
        eprintln!("[cortex] DB quick_check: OK");
    } else {
        eprintln!(
            "[cortex] WARNING: PRAGMA quick_check FAILED on {} -- running full integrity_check",
            db_path.display()
        );

        let integrity_ok = crate::db::verify_integrity(&conn).unwrap_or(false);
        if !integrity_ok {
            eprintln!(
                "[cortex] WARNING: PRAGMA integrity_check FAILED on {} -- attempting auto-repair",
                db_path.display()
            );

            // Drop the write connection before auto_repair renames the file.
            drop(conn);

            let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
            match crate::db::auto_repair(db_path, &timestamp) {
                Ok(result) => {
                    eprintln!(
                        "[cortex] Auto-repair succeeded: {} memories, {} decisions recovered. \
                         Corrupted DB preserved at {}",
                        result.memories_recovered,
                        result.decisions_recovered,
                        result.corrupt_db_path.display()
                    );
                    // Reopen the repaired DB and continue normal startup.
                    let conn = crate::db::open(db_path)
                        .map_err(|e| format!("Failed to open repaired DB: {e}"))?;
                    crate::db::configure(&conn)
                        .map_err(|e| format!("Failed to configure repaired DB: {e}"))?;
                    return initialize_with_conn(conn, paths, allow_token_rotation);
                }
                Err(e) => {
                    eprintln!(
                        "[cortex] Auto-repair failed ({e:?}). \
                         Starting in degraded mode -- reads may return incomplete data. \
                         DB path: {}",
                        db_path.display()
                    );
                    // Reopen whatever DB exists (may be the corrupted original if
                    // auto_repair failed before the rename step).
                    let conn = crate::db::open(db_path).map_err(|open_err| {
                        format!(
                            "Database corrupt and could not be reopened after failed repair: {open_err}"
                        )
                    })?;
                    crate::db::configure(&conn).ok();
                    crate::db::initialize_schema(&conn).ok();
                    let (state, rx) = initialize_with_conn(conn, paths, allow_token_rotation)?;
                    // Signal degraded mode so /health reflects corruption.
                    state
                        .db_corrupted
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    return Ok((state, rx));
                }
            }
        } else {
            eprintln!("[cortex] DB integrity: OK (after quick_check failure)");
        }
    }

    initialize_with_conn(conn, paths, allow_token_rotation)
}

fn initialize_with_conn(
    conn: Connection,
    paths: &CortexPaths,
    allow_token_rotation: bool,
) -> Result<(RuntimeState, oneshot::Receiver<()>), String> {
    // Rebuild FTS indexes only when they appear empty for non-empty source data.
    match crate::db::rebuild_fts_if_needed(&conn) {
        Ok(true) => eprintln!("[cortex] FTS baseline rebuilt"),
        Ok(false) => {}
        Err(e) => eprintln!("[cortex] WARNING: FTS rebuild check failed: {e}"),
    }

    // Open a small query-only read pool so bursty read load does not queue on a
    // single async mutex.
    let read_pool_size = read_pool_size_from_env();
    let mut read_connections = Vec::with_capacity(read_pool_size);
    for _ in 0..read_pool_size {
        read_connections.push(open_query_only_connection(&paths.db)?);
    }
    let db_read: Arc<dyn ReadConnectionProvider> =
        Arc::new(ReadConnectionPool::new(read_connections));
    eprintln!(
        "[cortex] Read pool opened with {} query-only connection{} (WAL concurrent reads enabled)",
        db_read.pool_size(),
        if db_read.pool_size() == 1 { "" } else { "s" }
    );

    let mode = crate::db::current_mode(&conn);
    let team_mode = mode == "team";
    let default_owner_id = if team_mode {
        let from_config = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse::<i64>().ok());
        from_config.or_else(|| {
            conn.query_row(
                "SELECT id FROM users ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, id ASC LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok()
        })
    } else {
        None
    };
    let team_api_key_hashes = if team_mode {
        let mut hashes: Vec<(i64, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, api_key_hash FROM users") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    hashes.push(row);
                }
            }
        }
        Arc::new(std::sync::RwLock::new(hashes))
    } else {
        Arc::new(std::sync::RwLock::new(Vec::new()))
    };

    // Auth token.
    let token = if team_mode {
        crate::auth::read_token_from(paths).unwrap_or_else(crate::auth::generate_ephemeral_token)
    } else if allow_token_rotation {
        crate::auth::generate_token_for(paths)
    } else {
        crate::auth::read_token_from(paths).unwrap_or_else(crate::auth::generate_ephemeral_token)
    };

    // Channels.
    let (events_tx, _) = broadcast::channel::<DaemonEvent>(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let home = paths.home.clone();
    let models_dir = paths.models.clone();
    let embedding_engine = crate::embeddings::EmbeddingEngine::load(&models_dir).map(Arc::new);

    if let Some(engine) = embedding_engine.as_ref() {
        eprintln!(
            "[cortex] Embedding engine loaded (model={}, {}-dim, in-process ONNX)",
            engine.model_key(),
            engine.dimension()
        );
    } else {
        eprintln!(
            "[cortex] Embedding engine not available -- keyword search only until model downloaded"
        );
    }

    let write_buffer_path = paths.write_buffer.clone();
    let sqlite_vec_canary = SqliteVecCanaryConfig::from_env();
    if sqlite_vec_canary.force_off {
        eprintln!(
            "[cortex] sqlite-vec routing force-off (configured mode={}, effective mode=baseline)",
            sqlite_vec_canary.route_mode.as_str()
        );
    } else {
        match sqlite_vec_canary.route_mode {
            SqliteVecRouteMode::Baseline => {
                eprintln!("[cortex] sqlite-vec routing mode=baseline (shadow diagnostics only)");
            }
            SqliteVecRouteMode::Trial => {
                if sqlite_vec_canary.trial_percent > 0 {
                    eprintln!(
                        "[cortex] sqlite-vec routing mode=trial ({}% sampled)",
                        sqlite_vec_canary.trial_percent
                    );
                } else {
                    eprintln!(
                        "[cortex] sqlite-vec routing mode=trial but trial percent is 0 (baseline-only)"
                    );
                }
            }
            SqliteVecRouteMode::Primary => {
                eprintln!(
                    "[cortex] sqlite-vec routing mode=primary (guarded vec0 routing enabled)"
                );
            }
        }
    }

    let state = RuntimeState {
        db: Arc::new(Mutex::new(conn)),
        db_read,
        token: Arc::new(token),
        events: events_tx,
        mcp_calls: Arc::new(AtomicU64::new(0)),
        mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
        recall_history: Arc::new(Mutex::new(HashMap::new())),
        pre_cache: Arc::new(Mutex::new(HashMap::new())),
        served_content: Arc::new(Mutex::new(HashMap::<String, HashMap<u32, i64>>::new())),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        home,
        db_path: paths.db.clone(),
        token_path: paths.token.clone(),
        pid_path: paths.pid.clone(),
        port: paths.port,
        embedding_engine,
        rate_limiter: crate::rate_limit::RateLimiter::new(),
        team_mode,
        default_owner_id,
        team_api_key_hashes,
        degraded_mode: Arc::new(AtomicBool::new(false)),
        db_corrupted: Arc::new(AtomicBool::new(false)),
        readiness: Arc::new(AtomicBool::new(false)),
        write_buffer_path,
        sqlite_vec_canary,
    };

    Ok((state, shutdown_rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_guard() -> MutexGuard<'static, ()> {
        match TEST_ENV_LOCK.get_or_init(|| Mutex::new(())).lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    const V041_SCHEMA_SQL: &str = r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS memories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          text TEXT NOT NULL,
          source TEXT,
          type TEXT DEFAULT 'memory',
          tags TEXT,
          source_agent TEXT DEFAULT 'unknown',
          confidence REAL DEFAULT 0.8,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS decisions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          decision TEXT NOT NULL,
          context TEXT,
          type TEXT DEFAULT 'decision',
          source_agent TEXT DEFAULT 'unknown',
          confidence REAL DEFAULT 0.8,
          surprise REAL DEFAULT 1.0,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          parent_id INTEGER,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS embeddings (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          target_type TEXT NOT NULL,
          target_id INTEGER NOT NULL,
          vector BLOB NOT NULL,
          model TEXT DEFAULT 'nomic-embed-text',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          type TEXT NOT NULL,
          data TEXT,
          source_agent TEXT,
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS co_occurrence (
          source_a TEXT NOT NULL,
          source_b TEXT NOT NULL,
          count INTEGER DEFAULT 1,
          last_seen TEXT DEFAULT (datetime('now')),
          PRIMARY KEY (source_a, source_b)
        );

        CREATE TABLE IF NOT EXISTS locks (
          id TEXT PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          agent TEXT NOT NULL,
          locked_at TEXT NOT NULL,
          expires_at TEXT
        );

        CREATE TABLE IF NOT EXISTS activities (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          description TEXT NOT NULL,
          files_json TEXT NOT NULL DEFAULT '[]',
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          sender TEXT NOT NULL,
          recipient TEXT NOT NULL,
          message TEXT NOT NULL,
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
          agent TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          description TEXT,
          started_at TEXT NOT NULL,
          last_heartbeat TEXT NOT NULL,
          expires_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks (
          task_id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          description TEXT,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          priority TEXT NOT NULL DEFAULT 'medium',
          required_capability TEXT NOT NULL DEFAULT 'any',
          status TEXT NOT NULL DEFAULT 'pending',
          claimed_by TEXT,
          created_at TEXT NOT NULL,
          claimed_at TEXT,
          completed_at TEXT,
          summary TEXT
        );

        CREATE TABLE IF NOT EXISTS feed (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          kind TEXT NOT NULL,
          summary TEXT NOT NULL,
          content TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          task_id TEXT,
          trace_id TEXT,
          priority TEXT NOT NULL DEFAULT 'normal',
          timestamp TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS feed_acks (
          agent TEXT PRIMARY KEY,
          last_seen_id TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a);
        CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b);
        CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status);
        CREATE INDEX IF NOT EXISTS idx_memories_source_status ON memories(source, status);
        CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status);
        CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_events_type_created ON events(type, created_at);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);

        CREATE TABLE IF NOT EXISTS context_cache (
          cache_key TEXT PRIMARY KEY,
          content_hash TEXT NOT NULL,
          compressed TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0,
          created_at TEXT DEFAULT (datetime('now')),
          hits INTEGER DEFAULT 0
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
          text, source, tags,
          content=memories,
          content_rowid=id,
          tokenize='trigram'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
          decision, context,
          content=decisions,
          content_rowid=id,
          tokenize='trigram'
        );

        CREATE TABLE IF NOT EXISTS recall_feedback (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          query_text TEXT NOT NULL,
          query_embedding BLOB,
          result_source TEXT NOT NULL,
          result_type TEXT NOT NULL DEFAULT 'unknown',
          result_id INTEGER,
          signal REAL NOT NULL DEFAULT 1.0,
          agent TEXT NOT NULL DEFAULT 'unknown',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_feedback_result ON recall_feedback(result_source);
        CREATE INDEX IF NOT EXISTS idx_feedback_created ON recall_feedback(created_at);

        CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_ai AFTER INSERT ON decisions BEGIN
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_ad AFTER DELETE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_au AFTER UPDATE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;
    "#;

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_state_{name}_{unique}"))
    }

    fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
        let pragma = format!("PRAGMA table_info({table})");
        let mut stmt = match conn.prepare(&pragma) {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows,
            Err(_) => return false,
        };
        for name in rows.flatten() {
            if name == column {
                return true;
            }
        }
        false
    }

    fn seed_v041_fixture(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let conn = crate::db::open(db_path).unwrap();
        crate::db::configure(&conn).unwrap();
        conn.execute_batch(V041_SCHEMA_SQL).unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, tags, source_agent, confidence, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
            params![
                "legacy memory payload",
                "legacy::memory",
                "memory",
                "alpha,beta",
                "legacy-agent",
                0.72_f64
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, confidence, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
            params![
                "legacy decision payload",
                "legacy context payload",
                "decision",
                "legacy-agent",
                0.91_f64
            ],
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }

    #[test]
    fn initialize_and_migrate_v041_fixture_to_latest_schema() {
        let home_dir = temp_test_dir("upgrade_v041");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54968),
            Some("127.0.0.1"),
        );
        seed_v041_fixture(&paths.db);

        let (state, shutdown_rx) =
            initialize(&paths, true).expect("current daemon boot should accept a v0.4.1 database");
        assert!(
            paths.token.exists(),
            "boot should materialize a shared auth token"
        );
        drop(shutdown_rx);
        drop(state);

        let conn = crate::db::open(&paths.db).unwrap();
        crate::db::configure(&conn).unwrap();

        let applied = crate::db::run_pending_migrations(&conn);
        assert_eq!(
            applied,
            crate::db::migration_definitions().len(),
            "legacy fixtures should apply every tracked migration exactly once"
        );
        assert!(crate::db::verify_integrity(&conn).unwrap());
        assert_eq!(
            crate::db::current_schema_user_version(&conn).unwrap(),
            crate::db::latest_schema_user_version()
        );

        let memory_text: String = conn
            .query_row(
                "SELECT text FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(memory_text, "legacy memory payload");

        let decision_text: String = conn
            .query_row(
                "SELECT decision FROM decisions WHERE context = 'legacy context payload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decision_text, "legacy decision payload");

        let merged_count: i64 = conn
            .query_row(
                "SELECT merged_count FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(merged_count, 0);

        let quality: i64 = conn
            .query_row(
                "SELECT quality FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quality, 50);

        let source_client: String = conn
            .query_row(
                "SELECT source_client FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_client, "unknown");

        let reasoning_depth: String = conn
            .query_row(
                "SELECT reasoning_depth FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reasoning_depth, "single-shot");

        let trust_score: f64 = conn
            .query_row(
                "SELECT trust_score FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trust_score, 0.8_f64);

        assert!(crate::db::table_exists(&conn, "schema_migrations"));
        assert!(crate::db::table_exists(&conn, "focus_sessions"));
        assert!(crate::db::table_exists(&conn, "memory_clusters"));
        assert!(crate::db::table_exists(&conn, "cluster_members"));
        assert!(crate::db::table_exists(&conn, "client_permissions"));
        assert!(crate::db::table_exists(&conn, "decision_conflicts"));
        assert!(crate::db::table_exists(&conn, "agent_feedback"));

        assert!(table_has_column(&conn, "memories", "merged_count"));
        assert!(table_has_column(&conn, "memories", "quality"));
        assert!(table_has_column(&conn, "memories", "expires_at"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(table_has_column(&conn, "decisions", "merged_count"));
        assert!(table_has_column(&conn, "decisions", "quality"));
        assert!(table_has_column(&conn, "decisions", "expires_at"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));

        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            migration_count as usize,
            crate::db::migration_definitions().len() + 1,
            "boot should add the FTS seed marker before tracked migrations run"
        );
        let tokenizer_migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = '012'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tokenizer_migration_count, 1);

        let fts_seeded_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 'fts_seeded_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_seeded_count, 1);

        let memories_fts_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            memories_fts_sql
                .to_ascii_lowercase()
                .contains("tokenize='porter unicode61'"),
            "expected porter/unicode61 tokenizer, got: {memories_fts_sql}"
        );
        let decisions_fts_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'decisions_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            decisions_fts_sql
                .to_ascii_lowercase()
                .contains("tokenize='porter unicode61'"),
            "expected porter/unicode61 tokenizer, got: {decisions_fts_sql}"
        );

        let memory_fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
            .unwrap();
        let decision_fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(memory_fts_rows, 1);
        assert_eq!(decision_fts_rows, 1);

        drop(conn);
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn sqlite_vec_canary_config_defaults_to_primary_route() {
        let _guard = env_guard();
        let prev_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT").ok();
        let prev_force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF").ok();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");

        let config = SqliteVecCanaryConfig::from_env();
        assert_eq!(config.trial_percent, 0);
        assert!(!config.force_off);
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert!(matches!(
            config.effective_route_mode(),
            SqliteVecRouteMode::Primary
        ));

        if let Some(value) = prev_percent {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        }
        if let Some(value) = prev_force_off {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        }
        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn sqlite_vec_canary_config_parses_and_clamps_env_values() {
        let _guard = env_guard();
        let prev_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT").ok();
        let prev_force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF").ok();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", "145");
        std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", "yes");
        std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", "primary");

        let config = SqliteVecCanaryConfig::from_env();
        assert_eq!(config.trial_percent, 100);
        assert!(config.force_off);
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert!(matches!(
            config.effective_route_mode(),
            SqliteVecRouteMode::Baseline
        ));

        if let Some(value) = prev_percent {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        }
        if let Some(value) = prev_force_off {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        }
        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn sqlite_vec_canary_config_route_mode_aliases_are_supported() {
        let _guard = env_guard();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", "vec0");

        let config = SqliteVecCanaryConfig::from_env();
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert_eq!(config.route_mode.as_str(), "primary");

        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn derive_read_pool_size_uses_cpu_hint_when_env_unset() {
        assert_eq!(derive_read_pool_size(None, Some(2)), READ_POOL_DEFAULT_MIN);
        assert_eq!(derive_read_pool_size(None, Some(64)), READ_POOL_DEFAULT_MAX);
    }

    #[test]
    fn derive_read_pool_size_clamps_configured_values() {
        assert_eq!(derive_read_pool_size(Some(0), Some(8)), READ_POOL_HARD_MIN);
        assert_eq!(derive_read_pool_size(Some(1), Some(8)), READ_POOL_HARD_MIN);
        assert_eq!(derive_read_pool_size(Some(7), Some(8)), 7);
        assert_eq!(
            derive_read_pool_size(Some(READ_POOL_HARD_MAX + 100), Some(8)),
            READ_POOL_HARD_MAX
        );
    }

    #[test]
    fn read_connection_pool_rotates_connections() {
        let conn_a = Connection::open_in_memory().unwrap();
        conn_a.execute_batch("PRAGMA user_version = 101;").unwrap();
        let conn_b = Connection::open_in_memory().unwrap();
        conn_b.execute_batch("PRAGMA user_version = 202;").unwrap();
        let pool = ReadConnectionPool::new(vec![conn_a, conn_b]);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let seen_versions = rt.block_on(async {
            let mut versions = Vec::new();
            for _ in 0..4 {
                let conn = pool.lock().await;
                let version: i64 = conn
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap();
                versions.push(version);
            }
            versions
        });

        assert_eq!(seen_versions, vec![101, 202, 101, 202]);
    }
}
