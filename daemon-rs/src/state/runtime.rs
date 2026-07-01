// SPDX-License-Identifier: MIT
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::{broadcast, oneshot, Mutex};

use super::read_pool::ReadConnectionProvider;
use super::types::{
    BrainFiringEvent, DaemonEvent, PreCacheEntry, RecallHistoryEntry, SqliteVecCanaryConfig,
};

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
    /// Broadcast channel for Brain-tab firing telemetry. Subscribed only by
    /// `/brain/firing`; full payloads, owner-scoped at the handler.
    pub brain_firing: broadcast::Sender<BrainFiringEvent>,
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
    /// Last observed request activity timestamp (Unix seconds).
    pub last_activity_unix_secs: Arc<AtomicU64>,
    /// Path for buffering writes when daemon is unreachable in proxy mode.
    /// Used by mcp_proxy via cortex_dir() directly; kept here for discoverability.
    #[allow(dead_code)]
    pub write_buffer_path: std::path::PathBuf,
    /// Guarded sqlite-vec semantic trial routing controls.
    pub sqlite_vec_canary: SqliteVecCanaryConfig,
    /// Cross-encoder reranker config. Default is off; shadow/primary are opt-in.
    pub rerank_config: crate::rerank::RerankConfig,
    /// Optional cross-encoder reranker loaded from local assets.
    pub reranker: Option<Arc<dyn crate::rerank::Reranker>>,
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

    /// Mark daemon activity to support idle-shutdown economics.
    pub fn mark_activity_now(&self) {
        self.last_activity_unix_secs
            .store(current_unix_secs(), Ordering::SeqCst);
    }

    /// Seconds since the last observed request activity.
    pub fn idle_for_secs(&self) -> u64 {
        let last = self.last_activity_unix_secs.load(Ordering::SeqCst);
        current_unix_secs().saturating_sub(last)
    }
}

pub(crate) fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
