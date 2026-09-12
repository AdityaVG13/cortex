use crate::{CortexRuntime, auth, db};
use asupersync::Cx;
use futures_util::future::{Either, select};
use std::time::Duration;

struct OwnPidFile(auth::CortexPaths);
impl Drop for OwnPidFile {
    fn drop(&mut self) {
        auth::remove_own_pid_file(&self.0);
    }
}

/// Optional headless worker. Cross-process writes are observed through SQLite's
/// data version; idle ticks do not run maintenance passes. Each slice is capped.
pub async fn run_daemon(cx: &Cx, paths: auth::CortexPaths, shutdown: impl std::future::Future<Output = ()>) -> Result<(), String> {
    std::fs::create_dir_all(&paths.home).map_err(|err| err.to_string())?;
    let _lock = auth::acquire_daemon_lock(&paths)?;
    let runtime = CortexRuntime::open(&paths).map_err(|err| err.to_string())?;
    let _pid = match auth::write_pid_file(&paths) {
        Ok(()) => Some(OwnPidFile(paths.clone())),
        Err(err) => {
            eprintln!("[cortex] WARNING: {err}");
            None
        }
    };
    let mut shutdown = Box::pin(shutdown);
    let mut data_version = None;
    let mut pending = true;
    loop {
        cx.checkpoint().map_err(|err| err.to_string())?;
        let conn = runtime.state().db.lock(cx).await.map_err(|err| err.to_string())?;
        let version: i64 = conn.query_row("PRAGMA data_version", [], |row| row.get(0)).map_err(|err| err.to_string())?;
        if pending || data_version != Some(version) {
            db::outbox::maintain_slice(&conn, "supervisor", 32).map_err(|err| err.to_string())?;
            pending = db::outbox::debt(&conn).pending_jobs > 0;
            data_version = Some(version);
            db::checkpoint_wal_best_effort(&conn);
        }
        drop(conn);
        let tick = Box::pin(asupersync::time::sleep(cx.now(), Duration::from_secs(1)));
        match select(shutdown, tick).await {
            Either::Left(_) => break,
            Either::Right(((), remaining_shutdown)) => shutdown = remaining_shutdown,
        }
    }
    let conn = runtime.state().db.lock(cx).await.map_err(|err| err.to_string())?;
    conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);").map_err(|err| err.to_string())?;
    Ok(())
}
