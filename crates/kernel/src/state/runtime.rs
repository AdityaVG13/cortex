use super::read_pool::ReadConnectionProvider;
use super::types::{BrainFiringEvent, DaemonEvent, SqliteVecCanaryConfig};
use asupersync::{
    Cx,
    channel::{broadcast, oneshot},
    sync::{LockError, Mutex},
};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// Read-path side effects (retrieval bumps, recall events) that could not
    /// take the write connection within the bounded wait. Drained by the next
    /// writer under its own lock, so a background pass never stalls a read.
    pub deferred_side_effects: Arc<std::sync::Mutex<Vec<DeferredSideEffect>>>,
    pub shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub home: std::path::PathBuf,
    #[allow(dead_code)]
    pub db_path: std::path::PathBuf,
    pub token_path: std::path::PathBuf,
    pub pid_path: std::path::PathBuf,
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
/// Bounded wait a read path spends on the write connection before deferring.
pub const SIDE_EFFECT_LOCK_WAIT_MS: u64 = 25;
/// Cap on deferred read-side effects kept in memory; beyond it the oldest
/// bumps are dropped (they are operational telemetry, not durable memory).
pub const DEFERRED_SIDE_EFFECT_CAP: usize = 4096;

#[derive(Clone, Debug)]
pub enum DeferredSideEffect {
    Retrievals {
        sources: Vec<String>,
    },
    Event {
        event_type: String,
        payload: Value,
        agent: String,
    },
}

impl RuntimeState {
    /// Run `f` on the write connection if it can be acquired within the
    /// bounded wait; otherwise hand the work to `defer`. Returns whether the
    /// effect ran now.
    pub async fn with_write_or_defer<F>(
        &self,
        cx: &Cx,
        f: F,
        defer: DeferredSideEffect,
    ) -> Result<bool, LockError>
    where
        F: FnOnce(&Connection) + Send,
    {
        match self
            .db
            .lock_until(
                cx,
                cx.now() + std::time::Duration::from_millis(SIDE_EFFECT_LOCK_WAIT_MS),
            )
            .await
        {
            Ok(conn) => {
                self.drain_deferred(&conn);
                f(&conn);
                Ok(true)
            }
            Err(LockError::TimedOut(_)) => {
                let mut queue = self
                    .deferred_side_effects
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if queue.len() >= DEFERRED_SIDE_EFFECT_CAP {
                    queue.remove(0);
                }
                queue.push(defer);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }
    /// Apply queued read-side effects on an already-held write connection.
    pub fn drain_deferred(&self, conn: &Connection) -> usize {
        let items: Vec<DeferredSideEffect> = {
            let mut queue = self
                .deferred_side_effects
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *queue)
        };
        let count = items.len();
        for item in items {
            match item {
                DeferredSideEffect::Retrievals { sources } => {
                    crate::handlers::recall::bump_retrievals_sources(conn, &sources)
                }
                DeferredSideEffect::Event {
                    event_type,
                    payload,
                    agent,
                } => {
                    let _ = crate::handlers::log_event(conn, &event_type, payload, &agent);
                }
            }
        }
        count
    }
    pub fn emit(
        &self,
        cx: &Cx,
        event_type: &str,
        data: Value,
    ) -> Result<usize, broadcast::SendError<DaemonEvent>> {
        self.events.send(
            cx,
            DaemonEvent {
                event_type: event_type.to_string(),
                data,
            },
        )
    }
    pub fn next_mcp_call(&self) -> u64 {
        use std::sync::atomic::Ordering;
        self.mcp_calls.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn mark_activity_now(&self) {
        self.last_activity_unix_secs
            .store(current_unix_secs(), Ordering::SeqCst);
    }
    pub fn idle_for_secs(&self) -> u64 {
        let last = self.last_activity_unix_secs.load(Ordering::SeqCst);
        current_unix_secs().saturating_sub(last)
    }
}
pub fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
