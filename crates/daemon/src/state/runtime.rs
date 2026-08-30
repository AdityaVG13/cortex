use super::read_pool::ReadConnectionProvider;
use super::types::{BrainFiringEvent, DaemonEvent, SqliteVecCanaryConfig};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, oneshot, Mutex};
#[derive(Clone)]
pub struct RuntimeState {
    pub db: Arc<Mutex<Connection>>,
    pub db_read: Arc<dyn ReadConnectionProvider>,
    pub token: Arc<String>,
    pub events: broadcast::Sender<DaemonEvent>,
    pub brain_firing: broadcast::Sender<BrainFiringEvent>,
    pub mcp_calls: Arc<AtomicU64>,
    #[allow(dead_code)]
    pub mcp_sessions: Arc<Mutex<HashMap<String, i64>>>,
    pub served_content: Arc<Mutex<HashMap<String, HashMap<u32, i64>>>>,
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub home: std::path::PathBuf,
    #[allow(dead_code)]
    pub db_path: std::path::PathBuf,
    pub token_path: std::path::PathBuf,
    pub pid_path: std::path::PathBuf,
    pub port: u16,
    pub rate_limiter: crate::rate_limit::RateLimiter,
    pub team_mode: bool,
    pub default_owner_id: Option<i64>,
    pub team_api_key_hashes: Arc<std::sync::RwLock<Vec<(i64, String)>>>,
    pub degraded_mode: Arc<AtomicBool>,
    pub db_corrupted: Arc<AtomicBool>,
    pub readiness: Arc<AtomicBool>,
    pub last_activity_unix_secs: Arc<AtomicU64>,
    #[allow(dead_code)]
    pub write_buffer_path: std::path::PathBuf,
    pub sqlite_vec_canary: SqliteVecCanaryConfig,
}
impl RuntimeState {
    pub fn emit(&self, event_type: &str, data: Value) {
        let _ = self.events.send(DaemonEvent { event_type: event_type.to_string(), data });
    }
    pub fn next_mcp_call(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.mcp_calls.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn mark_activity_now(&self) {
        self.last_activity_unix_secs.store(current_unix_secs(), Ordering::SeqCst);
    }
    pub fn idle_for_secs(&self) -> u64 {
        let last = self.last_activity_unix_secs.load(Ordering::SeqCst);
        current_unix_secs().saturating_sub(last)
    }
}
pub(crate) fn current_unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}
