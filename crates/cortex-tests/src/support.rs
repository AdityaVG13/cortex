//! Promoted from `cortex-daemon/src/test_support.rs`.
//!
//! In-memory database and `RuntimeState` builders used across the extracted
//! daemon test suite. All items here previously lived behind
//! `#[cfg(test)]` in the daemon crate; they are now part of the public
//! test-support surface of `cortex-tests`.
use cortex_daemon::db;
use cortex_daemon::rate_limit::RateLimiter;
use cortex_daemon::rerank::{RerankConfig, Reranker};
use cortex_daemon::state::ReadConnectionProvider;
use cortex_daemon::state::RuntimeState;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, Mutex};

pub fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory db");
    db::configure(&conn).expect("configure db");
    db::initialize_schema(&conn).expect("initialize schema");
    db::run_pending_migrations(&conn);
    conn
}

pub fn solo_state() -> RuntimeState {
    runtime_state(
        test_conn(),
        test_conn(),
        false,
        None,
        RerankConfig::off(),
        None,
    )
}

pub fn team_state(default_owner_id: i64) -> RuntimeState {
    runtime_state(
        test_conn(),
        test_conn(),
        true,
        Some(default_owner_id),
        RerankConfig::off(),
        None,
    )
}

pub fn runtime_state(
    write_conn: Connection,
    read_conn: Connection,
    team_mode: bool,
    default_owner_id: Option<i64>,
    rerank_config: RerankConfig,
    reranker: Option<Arc<dyn Reranker>>,
) -> RuntimeState {
    let (events, _) = broadcast::channel(8);
    let (brain_firing, _) = broadcast::channel(8);
    RuntimeState {
        db: Arc::new(Mutex::new(write_conn)),
        db_read: Arc::new(Mutex::new(read_conn)),
        token: Arc::new("test-token".to_string()),
        events,
        brain_firing,
        mcp_calls: Arc::new(AtomicU64::new(0)),
        mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
        served_content: Arc::new(Mutex::new(HashMap::new())),
        shutdown_tx: Arc::new(Mutex::new(None)),
        home: PathBuf::from("."),
        db_path: PathBuf::from(":memory:"),
        token_path: PathBuf::from("cortex.token"),
        pid_path: PathBuf::from("cortex.pid"),
        port: 7437,
        embedding_engine: None,
        rate_limiter: RateLimiter::new(),
        team_mode,
        default_owner_id,
        team_api_key_hashes: Arc::new(RwLock::new(Vec::new())),
        degraded_mode: Arc::new(AtomicBool::new(false)),
        db_corrupted: Arc::new(AtomicBool::new(false)),
        readiness: Arc::new(AtomicBool::new(true)),
        last_activity_unix_secs: Arc::new(AtomicU64::new(0)),
        write_buffer_path: PathBuf::from("write_buffer.jsonl"),
        sqlite_vec_canary: cortex_daemon::state::SqliteVecCanaryConfig {
            trial_percent: 0,
            force_off: false,
            route_mode: cortex_daemon::state::SqliteVecRouteMode::Trial,
        },
        rerank_config,
        reranker,
    }
}
