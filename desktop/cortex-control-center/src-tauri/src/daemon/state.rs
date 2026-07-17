use crate::constants::{CONTROL_CENTER_LOCK_FILE, CONTROL_CENTER_OWNER_TAG, DEFAULT_DAEMON_PORT, LOCAL_DAEMON_LOCK_WAIT_SECS};
use crate::daemon::paths::{default_cortex_dir, is_disallowed_daemon_binary_path, resolved_cortex_paths};
use crate::daemon::process::apply_hidden_daemon_process_flags;
use fs2::FileExt;
use serde::Serialize;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct DaemonState {
    pub exe_path: Option<PathBuf>,
    pub child: Mutex<Option<Child>>,
    /// Set true by `stop()`, cleared by `ensure_local_daemon()`. Watchdog reads
    /// this to decide whether a missing daemon should be auto-respawned. Lets
    /// the user explicitly stop the daemon without immediate revival.
    pub intentional_stop: AtomicBool,
}

impl DaemonState {
    pub fn new(exe_path: Option<PathBuf>) -> Self {
        Self { exe_path, child: Mutex::new(None), intentional_stop: AtomicBool::new(false) }
    }

    pub fn supervisor_paused(&self) -> bool {
        self.intentional_stop.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> Result<(bool, Option<u32>), String> {
        let mut child = self.child.lock().map_err(|_| "Failed to lock managed daemon state.".to_string())?;
        let Some(managed_child) = child.as_mut() else {
            return Ok((false, None));
        };

        match managed_child.try_wait() {
            Ok(Some(_)) => {
                *child = None;
                Ok((false, None))
            }
            Ok(None) => Ok((true, Some(managed_child.id()))),
            Err(err) => {
                eprintln!("[cortex-control-center] failed to poll managed daemon process; clearing stale handle: {err}");
                *child = None;
                Ok((false, None))
            }
        }
    }

    pub fn ensure_local_daemon(&self) -> Result<Option<u32>, String> {
        let mut child = self.child.lock().map_err(|_| "Failed to lock managed daemon state.".to_string())?;
        if let Some(existing) = child.as_mut() {
            match existing.try_wait() {
                Ok(Some(_)) => {
                    *child = None;
                }
                Ok(None) => {
                    return Ok(Some(existing.id()));
                }
                Err(err) => {
                    eprintln!("[cortex-control-center] failed to poll existing managed daemon before spawn; clearing stale handle: {err}");
                    *child = None;
                }
            }
        }

        let exe_path = self.exe_path.clone().ok_or_else(|| "Could not resolve Cortex daemon binary for app-managed local mode.".to_string())?;
        if is_disallowed_daemon_binary_path(&exe_path) {
            return Err(format!("Refusing to launch app-managed daemon from disallowed path: {}", exe_path.display()));
        }

        let paths = resolved_cortex_paths();
        let home = paths.home.clone().ok_or_else(|| "Could not resolve Cortex home path for app-managed local mode.".to_string())?;
        let db = paths.db.clone().ok_or_else(|| "Could not resolve Cortex database path for app-managed local mode.".to_string())?;
        // App-managed mode is intentionally local-only. We always bind to loopback
        // so Control Center can own daemon lifecycle without exposing it on LAN.
        let bind = "127.0.0.1".to_string();
        let port = paths.port.unwrap_or(DEFAULT_DAEMON_PORT);

        let mut command = Command::new(&exe_path);
        command
            .arg("serve")
            .arg("--home")
            .arg(home.display().to_string())
            .arg("--db")
            .arg(db.display().to_string())
            .arg("--port")
            .arg(port.to_string())
            .arg("--bind")
            .arg(bind)
            .env("CORTEX_DAEMON_OWNER", CONTROL_CENTER_OWNER_TAG)
            .env("CORTEX_DAEMON_OWNER_SOURCE", "control-center-app")
            .env("CORTEX_DAEMON_OWNER_MODE", "app-managed-local")
            .env("CORTEX_WAIT_FOR_DAEMON_LOCK", "1")
            .env("CORTEX_DAEMON_LOCK_WAIT_SECS", LOCAL_DAEMON_LOCK_WAIT_SECS.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        apply_hidden_daemon_process_flags(&mut command);

        let spawned = command.spawn().map_err(|err| format!("Failed to spawn app-managed daemon from {}: {err}", exe_path.display()))?;
        let pid = spawned.id();
        *child = Some(spawned);
        // A successful spawn implicitly arms the supervisor: any later death
        // should trigger an auto-respawn until the user explicitly stops.
        self.intentional_stop.store(false, Ordering::SeqCst);
        Ok(Some(pid))
    }

    pub fn stop(&self) -> Result<(), String> {
        // Pause the supervisor BEFORE killing the child so the watchdog does
        // not race in and spawn a new instance during teardown.
        self.intentional_stop.store(true, Ordering::SeqCst);
        let mut child = self.child.lock().map_err(|_| "Failed to lock managed daemon state.".to_string())?;
        if let Some(managed_child) = child.as_mut() {
            match managed_child.try_wait() {
                Ok(Some(_)) => {
                    *child = None;
                }
                Ok(None) => {
                    if let Err(err) = managed_child.kill() {
                        *child = None;
                        return Err(format!("Failed to stop managed daemon process: {err}"));
                    }
                    let _ = managed_child.wait();
                    *child = None;
                }
                Err(err) => {
                    eprintln!("[cortex-control-center] failed to poll managed daemon process during stop; clearing stale handle: {err}");
                    *child = None;
                }
            }
        }
        Ok(())
    }
}

pub struct LifecycleState {
    explicit_quit: AtomicBool,
}

pub struct AppInstanceGuard {
    lock_file: File,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self { explicit_quit: AtomicBool::new(false) }
    }
}

impl LifecycleState {
    pub fn request_quit(&self) {
        self.explicit_quit.store(true, Ordering::SeqCst);
    }

    pub fn is_quit_requested(&self) -> bool {
        self.explicit_quit.load(Ordering::SeqCst)
    }
}

impl AppInstanceGuard {
    pub fn acquire() -> Result<Option<Self>, String> {
        let lock_path = control_center_lock_path()?;
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
        }
        let mut lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|err| format!("Failed to open {}: {err}", lock_path.display()))?;
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                let _ = lock_file.set_len(0);
                let _ = writeln!(lock_file, "pid={}", std::process::id());
                Ok(Some(Self { lock_file }))
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(format!("Failed to lock {}: {err}", lock_path.display())),
        }
    }
}

impl Drop for AppInstanceGuard {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
    }
}

fn control_center_lock_path() -> Result<PathBuf, String> {
    Ok(default_cortex_dir()?.join("runtime").join(CONTROL_CENTER_LOCK_FILE))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonCommandResult {
    pub running: bool,
    pub reachable: bool,
    pub managed: bool,
    pub auth_token_ready: bool,
    pub pid: Option<u32>,
    pub message: String,
}
pub fn describe_daemon_state(managed: bool, reachable: bool, starting: bool, auth_token_ready: bool, pid: Option<u32>, port: u16) -> String {
    if managed && reachable && auth_token_ready {
        format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
    } else if managed && reachable {
        format!("Cortex daemon running (pid {}) and reachable, waiting for auth token.", pid.unwrap_or_default())
    } else if managed && starting {
        format!("Cortex daemon running (pid {}) and still starting on :{}.", pid.unwrap_or_default(), port)
    } else if managed {
        format!("Cortex daemon running (pid {}) but not reachable on :{} yet.", pid.unwrap_or_default(), port)
    } else if reachable && auth_token_ready {
        "Cortex daemon reachable (external process).".to_string()
    } else if reachable {
        "Cortex daemon reachable (external process), waiting for auth token.".to_string()
    } else if starting {
        format!("Cortex daemon is responding on :{} and still starting.", port)
    } else {
        "Cortex daemon is offline.".to_string()
    }
}
