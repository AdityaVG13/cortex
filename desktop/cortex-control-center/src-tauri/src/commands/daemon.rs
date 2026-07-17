use crate::constants::{DAEMON_REACHABILITY_TIMEOUT_MS, DAEMON_STOP_WAIT_MS};
use crate::cortex_http::{
    auth_token_ready, is_cortex_reachable_with_port, probe_cortex_reachability_with_port, read_auth_token_with_retry, wait_for_reachability,
};
use crate::daemon::paths::{daemon_port, log_startup_path, service_ensure_fallback_enabled};
use crate::daemon::shutdown::send_http_shutdown;
use crate::daemon::spawn::{try_local_app_managed_ensure, try_service_ensure};
use crate::daemon::state::{describe_daemon_state, DaemonCommandResult, DaemonState};
use std::time::Duration;
use tauri::State;

#[tauri::command]
pub async fn daemon_status(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let (managed, pid) = state.status()?;
    let port = daemon_port();
    let probe = tauri::async_runtime::spawn_blocking(move || probe_cortex_reachability_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS))
        .await
        .map_err(|err| format!("daemon_status reachability task failed: {err}"))?;
    let reachable = probe.reachable;
    let starting = probe.starting;
    let auth_token_ready = if reachable {
        tauri::async_runtime::spawn_blocking(auth_token_ready).await.map_err(|err| format!("daemon_status token task failed: {err}"))?
    } else {
        false
    };
    let mut message = describe_daemon_state(managed, reachable, starting, auth_token_ready, pid, port);
    if probe.identity_mismatch {
        message.push_str(" Runtime identity metadata mismatch detected; using loose local daemon probe.");
    }

    Ok(DaemonCommandResult { running: managed || reachable || starting, reachable, managed, auth_token_ready, pid, message })
}

#[tauri::command]
pub async fn start_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (managed, pid) = state.status()?;
    let probe = probe_cortex_reachability_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if probe.reachable {
        log_startup_path(
            "start_daemon",
            "existing-daemon",
            if probe.identity_mismatch {
                "daemon already reachable before start command (identity mismatch fallback)"
            } else {
                "daemon already reachable before start command"
            },
        );
        let auth_token_ready = auth_token_ready();
        let mut message = describe_daemon_state(managed, true, false, auth_token_ready, pid, port);
        if probe.identity_mismatch {
            message.push_str(" Runtime identity metadata mismatch detected; using loose local daemon probe.");
        }
        return Ok(DaemonCommandResult { running: true, reachable: true, managed, auth_token_ready, pid, message });
    }

    if probe.starting {
        log_startup_path(
            "start_daemon",
            "existing-daemon",
            if probe.identity_mismatch {
                "daemon already starting before start command (identity mismatch fallback)"
            } else {
                "daemon already starting before start command"
            },
        );
        let auth_token_ready = auth_token_ready();
        let mut message = describe_daemon_state(managed, false, true, auth_token_ready, pid, port);
        if probe.identity_mismatch {
            message.push_str(" Runtime identity metadata mismatch detected; using loose local daemon probe.");
        }
        return Ok(DaemonCommandResult { running: true, reachable: false, managed, auth_token_ready, pid, message });
    }

    match try_local_app_managed_ensure(&state, port) {
        Ok(local_probe) => {
            log_startup_path(
                "start_daemon",
                "app-managed-spawn",
                if local_probe.reachable {
                    "daemon started via Control Center local mode"
                } else {
                    "daemon spawned via Control Center local mode and is still starting"
                },
            );
            let (managed, pid) = state.status()?;
            let auth_token_ready = if local_probe.reachable { auth_token_ready() } else { false };
            let mut message = describe_daemon_state(managed, local_probe.reachable, local_probe.starting, auth_token_ready, pid, port);
            if local_probe.identity_mismatch {
                message.push_str(" Runtime identity metadata mismatch detected; using loose local daemon probe.");
            }
            Ok(DaemonCommandResult {
                running: managed || local_probe.reachable || local_probe.starting,
                reachable: local_probe.reachable,
                managed,
                auth_token_ready,
                pid,
                message,
            })
        }
        Err(local_err) => {
            if service_ensure_fallback_enabled() {
                match try_service_ensure(port) {
                    Ok(true) => {
                        log_startup_path("start_daemon", "service-ensure", "daemon started or validated via service ensure after app-managed fallback");
                        let auth_token_ready = auth_token_ready();
                        Ok(DaemonCommandResult {
                            running: true,
                            reachable: true,
                            managed: false,
                            auth_token_ready,
                            pid: None,
                            message: "Daemon started via Windows service.".to_string(),
                        })
                    }
                    Ok(false) => {
                        log_startup_path("start_daemon", "blocked", "app-managed spawn failed");
                        Err(format!("App-managed local start failed and Windows service ensure was unavailable: {local_err}"))
                    }
                    Err(service_err) => {
                        log_startup_path("start_daemon", "blocked", "app-managed spawn failed");
                        Err(format!("App-managed local start failed: {local_err}. Windows service ensure failed: {service_err}"))
                    }
                }
            } else {
                log_startup_path("start_daemon", "blocked", "app-managed spawn failed (service fallback disabled)");
                Err(format!("App-managed local start failed: {local_err}"))
            }
        }
    }
}

#[tauri::command]
pub async fn stop_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (was_running, _) = state.status()?;
    let still_reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    let mut shutdown_error = None;
    if was_running && still_reachable {
        if let Err(err) = send_http_shutdown() {
            shutdown_error = Some(err);
        }
        let _ = wait_for_reachability(port, false, Duration::from_millis(DAEMON_STOP_WAIT_MS)).await;
    }

    let managed_stop_error = if was_running { state.stop().err() } else { None };
    let reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if reachable {
        if let Some(err) = managed_stop_error.or(shutdown_error) {
            return Err(err);
        }
    }

    let message = if was_running && !reachable {
        "Stopped app-managed Cortex daemon.".to_string()
    } else if was_running && reachable {
        "Shutdown signal sent, daemon still shutting down...".to_string()
    } else if !was_running && still_reachable {
        "Unmanaged daemon is running; Control Center did not stop it.".to_string()
    } else {
        "Daemon is already stopped.".to_string()
    };

    Ok(DaemonCommandResult { running: reachable, reachable, managed: false, auth_token_ready: reachable && auth_token_ready(), pid: None, message })
}

#[tauri::command]
pub async fn read_auth_token() -> Result<String, String> {
    read_auth_token_with_retry().await
}
