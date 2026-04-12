// SPDX-License-Identifier: MIT
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{Mutex, broadcast, oneshot};

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

// ─── Shared application state ─────────────────────────────────────────────────

/// Shared state threaded through every Axum handler via `axum::extract::State`.
///
/// All fields are cheaply `Clone`able — most are wrapped in `Arc`.
#[derive(Clone)]
pub struct RuntimeState {
    /// SQLite write connection -- used by store, forget, resolve, diary, indexer.
    pub db: Arc<Mutex<Connection>>,
    /// SQLite read connection -- used by recall, peek, health, digest, boot.
    /// Separate from `db` so reads never block on writes (WAL mode).
    pub db_read: Arc<Mutex<Connection>>,
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
    /// Path for buffering writes when daemon is unreachable in proxy mode.
    /// Used by mcp_proxy via cortex_dir() directly; kept here for discoverability.
    #[allow(dead_code)]
    pub write_buffer_path: std::path::PathBuf,
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

    // Open a separate read-only connection for concurrent reads.
    let read_conn =
        crate::db::open(&paths.db).map_err(|e| format!("Failed to open read connection: {e}"))?;
    crate::db::configure(&read_conn)
        .map_err(|e| format!("Failed to configure read connection: {e}"))?;
    read_conn
        .execute_batch("PRAGMA query_only = ON;")
        .map_err(|e| e.to_string())?;
    eprintln!("[cortex] Read connection opened (WAL concurrent reads enabled)");

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

    if embedding_engine.is_some() {
        eprintln!(
            "[cortex] Embedding engine loaded ({}-dim, in-process ONNX)",
            crate::embeddings::DIMENSION
        );
    } else {
        eprintln!(
            "[cortex] Embedding engine not available -- keyword search only until model downloaded"
        );
    }

    let write_buffer_path = paths.write_buffer.clone();

    let state = RuntimeState {
        db: Arc::new(Mutex::new(conn)),
        db_read: Arc::new(Mutex::new(read_conn)),
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
        write_buffer_path,
    };

    Ok((state, shutdown_rx))
}
