use crate::constants::*;
use crate::cortex_http::readiness::{
    is_cortex_reachable_with_port, read_auth_token_once, read_auth_token_with_retry_blocking,
    wait_for_reachability_blocking,
};
use crate::cortex_http::request::{send_cortex_request, FetchCortexResponse};
use crate::daemon::paths::{cortex_db_path, daemon_port};
use crate::daemon::state::DaemonState;
use rusqlite::Connection;
use std::time::Duration;
use tauri::{Manager, Runtime};

pub fn shutdown_daemon<R: Runtime>(app: &tauri::AppHandle<R>) {
    let daemon_state = app.state::<DaemonState>();
    let (managed, _) = daemon_state.status().unwrap_or((false, None));
    let port = daemon_port();
    if managed && is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        let _ = send_http_shutdown();
        let _ =
            wait_for_reachability_blocking(port, false, Duration::from_millis(DAEMON_STOP_WAIT_MS));
    }
    if managed {
        let _ = daemon_state.stop();
        let _ = flush_cortex_db_on_shutdown();
    }
}

fn flush_cortex_db_on_shutdown() -> Result<(), String> {
    let db_path = cortex_db_path()?;
    if !db_path.exists() {
        return Ok(());
    }

    let conn = Connection::open(&db_path).map_err(|err| {
        format!(
            "Failed to open DB for shutdown flush {}: {err}",
            db_path.display()
        )
    })?;
    configure_shutdown_flush_connection(&conn).map_err(|err| {
        format!(
            "Failed to flush WAL on shutdown {}: {err}",
            db_path.display()
        )
    })?;
    conn.close()
        .map_err(|(_, err)| format!("Failed to close DB after shutdown flush: {err}"))?;
    Ok(())
}

pub fn configure_shutdown_flush_connection(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        r#"
        PRAGMA busy_timeout = {SQLITE_BUSY_TIMEOUT_MS};
        PRAGMA journal_mode = WAL;
        PRAGMA wal_checkpoint(TRUNCATE);
        "#
    ))
}
/// Send POST /shutdown to the daemon's HTTP endpoint (works for any daemon,
/// regardless of who spawned it). Returns Ok(()) on success or if connection
/// fails (daemon already gone).
pub(crate) fn send_http_shutdown() -> Result<(), String> {
    let token = read_auth_token_once().unwrap_or_default();
    let initial = send_cortex_request("POST", "/shutdown", &token, Some("{}"), None);
    if matches!(
        initial,
        Ok(FetchCortexResponse {
            status: 401 | 403,
            ..
        })
    ) {
        if let Ok(refreshed_token) =
            read_auth_token_with_retry_blocking(Duration::from_millis(AUTH_TOKEN_WAIT_MS))
        {
            if !refreshed_token.is_empty() && refreshed_token != token {
                return interpret_shutdown_response(send_cortex_request(
                    "POST",
                    "/shutdown",
                    &refreshed_token,
                    Some("{}"),
                    None,
                ));
            }
        }
    }

    interpret_shutdown_response(initial)
}

pub fn interpret_shutdown_response(
    response: Result<FetchCortexResponse, String>,
) -> Result<(), String> {
    match response {
        Ok(resp) if (200..300).contains(&resp.status) => Ok(()),
        Ok(resp) if resp.status == 401 || resp.status == 403 => Err(
            "Shutdown rejected by daemon authentication. Refresh the token or restart the daemon from Control Center."
                .to_string(),
        ),
        Ok(resp) => {
            let detail = extract_error_detail(&resp.body)
                .map(|value| format!(" ({value})"))
                .unwrap_or_default();
            Err(format!("Daemon shutdown failed: HTTP {}{detail}", resp.status))
        }
        Err(err) if err.starts_with("Cannot connect to daemon") => Ok(()),
        Err(err) => Err(format!("Failed to send daemon shutdown: {err}")),
    }
}

pub fn extract_error_detail(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(error) = json.get("error").and_then(|value| value.as_str()) {
            return Some(error.to_string());
        }
    }
    Some(trimmed.chars().take(120).collect())
}
