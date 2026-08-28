//! Shared in-process test databases and `RuntimeState` builders.
//!
//! Write and read connections for a state share one on-disk SQLite file so
//! store followed by recall observes the same rows. A lone `test_conn()` stays
//! in-memory for single-connection contracts.
use cortex_daemon::db;
use cortex_daemon::rate_limit::RateLimiter;
use cortex_daemon::rerank::{RerankConfig, Reranker};
use cortex_daemon::state::RuntimeState;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

pub fn open_file_db(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open sqlite");
    db::configure(&conn).expect("configure db");
    db::initialize_schema(&conn).expect("initialize schema");
    db::run_pending_migrations(&conn);
    conn
}

/// Write + read connections on one file, plus the temp home directory.
pub fn shared_file_pair() -> (Connection, Connection, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("cortex-tests-")
        .tempdir()
        .expect("tempdir")
        .keep();
    let path = dir.join("cortex.db");
    let write = open_file_db(&path);
    let read = Connection::open(&path).expect("open read sqlite");
    db::configure(&read).expect("configure read db");
    (write, read, dir)
}

pub fn solo_state() -> RuntimeState {
    let (write, read, home) = shared_file_pair();
    let db_path = home.join("cortex.db");
    let mut state = runtime_state(write, read, false, None, RerankConfig::off(), None);
    state.home = home;
    state.db_path = db_path;
    state
}

pub fn team_state(default_owner_id: i64) -> RuntimeState {
    let (write, read, home) = shared_file_pair();
    let db_path = home.join("cortex.db");
    let mut state = runtime_state(
        write,
        read,
        true,
        Some(default_owner_id),
        RerankConfig::off(),
        None,
    );
    state.home = home;
    state.db_path = db_path;
    state
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
