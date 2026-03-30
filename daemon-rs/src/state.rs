use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{broadcast, oneshot, Mutex};

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
    pub data: Value,
}

// ─── Shared application state ─────────────────────────────────────────────────

/// Shared state threaded through every Axum handler via `axum::extract::State`.
///
/// All fields are cheaply `Clone`able — most are wrapped in `Arc`.
#[derive(Clone)]
pub struct RuntimeState {
    /// SQLite connection behind an async mutex so handlers can safely share it.
    pub db: Arc<Mutex<Connection>>,
    /// Auth token written to `~/.cortex/cortex.token` at startup.
    pub token: Arc<String>,
    /// Broadcast channel for SSE events; clone the sender to fan-out.
    pub events: broadcast::Sender<DaemonEvent>,
    /// Monotonic counter for MCP call IDs.
    pub mcp_calls: Arc<AtomicU64>,
    /// Active MCP sessions: session-id → last-heartbeat (Unix seconds).
    pub mcp_sessions: Arc<Mutex<HashMap<String, i64>>>,
    /// Per-agent recall history, capped at MAX_RECALL_HISTORY entries.
    pub recall_history: Arc<Mutex<HashMap<String, Vec<RecallHistoryEntry>>>>,
    /// Short-lived pre-warmed recall cache.
    pub pre_cache: Arc<Mutex<HashMap<String, PreCacheEntry>>>,
    /// Tracks which content IDs have already been served to each agent so
    /// duplicate entries are suppressed.
    pub served_content: Arc<Mutex<HashMap<String, HashSet<u32>>>>,
    /// Sending half of the graceful-shutdown oneshot.  The `/shutdown` endpoint
    /// takes this and fires it; the Axum server listens on the receiving half.
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// The user's home directory (used when constructing runtime paths).
    pub home: std::path::PathBuf,
    /// Absolute path of the SQLite database file.
    pub db_path: std::path::PathBuf,
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
    db_path: &std::path::Path,
) -> Result<(RuntimeState, oneshot::Receiver<()>), String> {
    // 1. Open and configure the database.
    let conn = crate::db::open(db_path)
        .map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;

    crate::db::configure(&conn)
        .map_err(|e| format!("Failed to configure database: {e}"))?;

    crate::db::initialize_schema(&conn)
        .map_err(|e| format!("Failed to initialise schema: {e}"))?;

    // 2. Integrity check — non-fatal; log a warning and continue.
    match crate::db::verify_integrity(&conn) {
        Ok(true) => {}
        Ok(false) => eprintln!("[cortex] WARNING: database integrity check failed"),
        Err(e) => eprintln!("[cortex] WARNING: integrity check error: {e}"),
    }

    // 3. Auth token.
    let token = crate::auth::generate_token();

    // 4. Channels.
    let (events_tx, _) = broadcast::channel::<DaemonEvent>(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // 5. Derive home directory from db_path parent (or env fallback).
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."));

    let state = RuntimeState {
        db: Arc::new(Mutex::new(conn)),
        token: Arc::new(token),
        events: events_tx,
        mcp_calls: Arc::new(AtomicU64::new(0)),
        mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
        recall_history: Arc::new(Mutex::new(HashMap::new())),
        pre_cache: Arc::new(Mutex::new(HashMap::new())),
        served_content: Arc::new(Mutex::new(HashMap::new())),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        home,
        db_path: db_path.to_path_buf(),
    };

    Ok((state, shutdown_rx))
}
