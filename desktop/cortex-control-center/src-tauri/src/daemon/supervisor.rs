use crate::constants::*;
use crate::cortex_http::readiness::probe_cortex_reachability_with_port;
use crate::daemon::paths::{daemon_port, log_startup_path, service_ensure_fallback_enabled};
use crate::daemon::spawn::{try_local_app_managed_ensure, try_service_ensure};
use crate::daemon::state::DaemonState;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{Manager, Runtime};

pub struct SupervisorControl {
    stop: AtomicBool,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl SupervisorControl {
    pub fn new() -> Self {
        Self { stop: AtomicBool::new(false), handle: Mutex::new(None) }
    }

    fn stop_flag(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub fn attach(&self, handle: JoinHandle<()>) {
        *self.handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
    }

    fn join(&self) {
        if let Some(handle) = self.handle.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }
}

pub fn request_supervisor_stop<R: Runtime>(app: &tauri::AppHandle<R>) {
    app.state::<SupervisorControl>().request_stop();
}

pub fn join_supervisor<R: Runtime>(app: &tauri::AppHandle<R>) {
    app.state::<SupervisorControl>().join();
}

fn supervisor_should_stop(app_handle: &tauri::AppHandle) -> bool {
    app_handle.state::<SupervisorControl>().stop_flag()
}

/// Sleep until the next tick, returning true when the app asked the thread to exit.
fn wait_for_supervisor_tick_or_stop(app_handle: &tauri::AppHandle) -> bool {
    const SLICE_MS: u64 = 200;
    let mut slept = 0;
    while slept < SUPERVISOR_TICK_MS {
        if supervisor_should_stop(app_handle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(SLICE_MS));
        slept += SLICE_MS;
    }
    supervisor_should_stop(app_handle)
}

pub fn run_supervisor_loop(app_handle: &tauri::AppHandle) {
    let consecutive_failures = AtomicU32::new(0);
    loop {
        if wait_for_supervisor_tick_or_stop(app_handle) {
            return;
        }
        supervisor_tick(app_handle, &consecutive_failures);
    }
}

/// Respawns an unreachable, unmanaged daemon unless the user stopped it.
/// Called on a fixed cadence from the startup watchdog.
pub fn supervisor_tick(app_handle: &tauri::AppHandle, consecutive_failures: &AtomicU32) {
    let daemon_state = app_handle.state::<DaemonState>();
    if daemon_state.supervisor_paused() {
        return;
    }

    let port = daemon_port();
    let probe = probe_cortex_reachability_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if probe.reachable || probe.starting {
        consecutive_failures.store(0, Ordering::SeqCst);
        return;
    }

    let (managed, _) = daemon_state.status().unwrap_or((false, None));
    if managed {
        return;
    }

    let attempt = consecutive_failures.fetch_add(1, Ordering::SeqCst);
    match try_local_app_managed_ensure(&daemon_state, port) {
        Ok(_) => {
            log_startup_path("supervisor", "respawn", "daemon was unreachable; supervisor respawned via app-managed local mode");
            consecutive_failures.store(0, Ordering::SeqCst);
        }
        Err(err) => {
            if attempt == 0 || attempt % 10 == 0 {
                eprintln!("[cortex-control-center] supervisor respawn attempt {attempt} failed: {err}");
                log_startup_path("supervisor", "respawn-failed", &err);
            }
        }
    }
}

pub fn bootstrap_daemon_on_startup(app_handle: &tauri::AppHandle) {
    let port = daemon_port();
    let probe = probe_cortex_reachability_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if probe.reachable {
        log_startup_path(
            "setup",
            "existing-daemon",
            if probe.identity_mismatch {
                "daemon already reachable at application startup (identity mismatch fallback)"
            } else {
                "daemon already reachable at application startup"
            },
        );
        return;
    }

    if probe.starting {
        log_startup_path(
            "setup",
            "existing-daemon",
            if probe.identity_mismatch {
                "daemon already starting at application startup (identity mismatch fallback)"
            } else {
                "daemon already starting at application startup"
            },
        );
        return;
    }

    let daemon_state = app_handle.state::<DaemonState>();
    match try_local_app_managed_ensure(&daemon_state, port) {
        Ok(local_probe) => {
            log_startup_path(
                "setup",
                "app-managed-spawn",
                if local_probe.reachable {
                    "daemon started via Control Center local mode"
                } else {
                    "daemon spawned via Control Center local mode and is still starting"
                },
            );
        }
        Err(local_err) => {
            if service_ensure_fallback_enabled() {
                match try_service_ensure(port) {
                    Ok(true) => {
                        log_startup_path("setup", "service-ensure", "daemon started or validated via service ensure after app-managed fallback");
                    }
                    Ok(false) => {
                        eprintln!("[cortex-control-center] app-managed local start failed at startup and Windows service ensure was unavailable: {local_err}");
                        log_startup_path("setup", "blocked", "app-managed spawn failed");
                    }
                    Err(service_err) => {
                        eprintln!(
                            "[cortex-control-center] app-managed local start failed at startup and service ensure also failed: {local_err}; {service_err}"
                        );
                        log_startup_path("setup", "blocked", "app-managed spawn failed");
                    }
                }
            } else {
                eprintln!("[cortex-control-center] app-managed local start failed at startup (service fallback disabled): {local_err}");
                log_startup_path("setup", "blocked", "app-managed spawn failed");
            }
        }
    }
}
