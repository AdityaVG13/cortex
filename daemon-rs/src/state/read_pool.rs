// SPDX-License-Identifier: MIT
use rusqlite::Connection;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;
const READ_POOL_SIZE_ENV: &str = "CORTEX_DB_READ_POOL_SIZE";
const READ_POOL_DEFAULT_MIN: usize = 4;
const READ_POOL_DEFAULT_MAX: usize = 16;
const READ_POOL_HARD_MAX: usize = 32;
const READ_POOL_HARD_MIN: usize = 2;
pub type ReadConnLockFuture<'a> = Pin<Box<dyn Future<Output = tokio::sync::MutexGuard<'a, Connection>> + Send + 'a>>;
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
pub(crate) struct ReadConnectionPool {
    connections: Vec<Mutex<Connection>>,
    next_index: AtomicUsize,
}
impl ReadConnectionPool {
    pub(crate) fn new(connections: Vec<Connection>) -> Self {
        assert!(!connections.is_empty(), "read connection pool requires at least one connection");
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
pub(crate) fn derive_read_pool_size(configured: Option<usize>, cpu_hint: Option<usize>) -> usize {
    let default = cpu_hint.unwrap_or(READ_POOL_DEFAULT_MIN).clamp(READ_POOL_DEFAULT_MIN, READ_POOL_DEFAULT_MAX);
    configured.unwrap_or(default).clamp(READ_POOL_HARD_MIN, READ_POOL_HARD_MAX)
}
pub(crate) fn read_pool_size_from_env() -> usize {
    let configured = std::env::var(READ_POOL_SIZE_ENV).ok().and_then(|raw| raw.trim().parse::<usize>().ok());
    let cpu_hint = std::thread::available_parallelism().ok().map(|cpus| cpus.get());
    derive_read_pool_size(configured, cpu_hint)
}
pub(crate) fn open_query_only_connection(db_path: &Path) -> Result<Connection, String> {
    let read_conn = crate::db::open(db_path).map_err(|e| format!("Failed to open read connection: {e}"))?;
    crate::db::configure(&read_conn).map_err(|e| format!("Failed to configure read connection: {e}"))?;
    read_conn.execute_batch("PRAGMA query_only = ON;").map_err(|e| e.to_string())?;
    Ok(read_conn)
}
