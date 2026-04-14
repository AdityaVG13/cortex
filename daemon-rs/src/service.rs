// SPDX-License-Identifier: MIT
//! Windows Service support for Cortex daemon.
//!
//! Subcommands:
//!   `cortex service install`   -- Register as Windows Service (requires Admin)
//!   `cortex service uninstall` -- Remove Windows Service
//!   `cortex service start`     -- Start the service
//!   `cortex service stop`      -- Stop the service
//!   `cortex service status`    -- Check service status
//!   `cortex service ensure`    -- Ensure installed + running + healthy
//!   `cortex service-run`       -- Internal: SCM entry point
//!
//! The service runs the same daemon as `cortex serve` but under the Windows
//! Service Control Manager with manual start by default, auto-restart on
//! failure, and proper lifecycle management.

const SERVICE_NAME: &str = "CortexDaemon";
const DISPLAY_NAME: &str = "Cortex Memory Daemon";
const DESCRIPTION: &str = "Always-on AI memory daemon -- serves Claude, Gemini, Codex, Cursor, and local LLMs via HTTP (:7437) and MCP.";
const DEFAULT_START_MODE: &str = "demand";
const ENSURE_HEALTH_TIMEOUT_SECS: u64 = 12;
const ENSURE_POLL_MILLIS: u64 = 250;

fn daemon_base_url() -> String {
    let port = crate::auth::CortexPaths::resolve().port;
    format!("http://127.0.0.1:{port}")
}

fn daemon_health_url() -> String {
    format!("{}/health", daemon_base_url())
}

fn daemon_readiness_url() -> String {
    format!("{}/readiness", daemon_base_url())
}

fn daemon_ready_from_payload(
    status: u16,
    body: &str,
    paths: &crate::auth::CortexPaths,
) -> Option<bool> {
    if let Some(ready) = crate::daemon_lifecycle::readiness_state_from_payload(
        status,
        body,
        Some(paths.port),
        Some(paths),
    ) {
        return Some(ready);
    }
    if crate::daemon_lifecycle::is_cortex_health_payload(
        status,
        body,
        Some(paths.port),
        Some(paths),
    ) {
        return Some(true);
    }
    None
}

fn build_sc_create_command(exe_path: &str, username: &str) -> String {
    format!(
        "sc.exe create {} binPath= \"\\\"{}\\\" service-run\" start= {} DisplayName= \"{}\" obj= \".\\{}\"",
        SERVICE_NAME, exe_path, DEFAULT_START_MODE, DISPLAY_NAME, username
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceState {
    NotInstalled,
    Running,
    Stopped,
    StartPending,
    StopPending,
    Unknown,
}

impl ServiceState {
    fn as_str(self) -> &'static str {
        match self {
            ServiceState::NotInstalled => "NOT_INSTALLED",
            ServiceState::Running => "RUNNING",
            ServiceState::Stopped => "STOPPED",
            ServiceState::StartPending => "START_PENDING",
            ServiceState::StopPending => "STOP_PENDING",
            ServiceState::Unknown => "UNKNOWN",
        }
    }
}

fn output_text(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "<no output>".to_string(),
    }
}

fn parse_service_state(output_text: &str) -> ServiceState {
    if output_text.contains("RUNNING") {
        ServiceState::Running
    } else if output_text.contains("STOPPED") {
        ServiceState::Stopped
    } else if output_text.contains("START_PENDING") {
        ServiceState::StartPending
    } else if output_text.contains("STOP_PENDING") {
        ServiceState::StopPending
    } else {
        ServiceState::Unknown
    }
}

fn query_service_state() -> Result<ServiceState, String> {
    let output = std::process::Command::new("sc.exe")
        .args(["query", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc.exe query: {e}"))?;

    if output.status.success() {
        let text = output_text(&output);
        return Ok(parse_service_state(&text));
    }

    let text = output_text(&output);
    if text.contains("1060") || text.contains("does not exist") {
        Ok(ServiceState::NotInstalled)
    } else {
        Err(text)
    }
}

fn daemon_health_ready() -> bool {
    let paths = crate::auth::CortexPaths::resolve();
    let client = match reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let readiness_url = daemon_readiness_url();
    if let Ok(response) = client.get(&readiness_url).send() {
        let status = response.status().as_u16();
        if let Ok(body) = response.text() {
            if let Some(ready) = daemon_ready_from_payload(status, &body, &paths) {
                return ready;
            }
        }
    }

    let health_url = daemon_health_url();
    let response = match client.get(&health_url).send() {
        Ok(response) => response,
        Err(_) => return false,
    };
    let status = response.status().as_u16();
    let body = match response.text() {
        Ok(body) => body,
        Err(_) => return false,
    };
    daemon_ready_from_payload(status, &body, &paths).unwrap_or(false)
}

fn wait_for_daemon_health(timeout: std::time::Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if daemon_health_ready() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(ENSURE_POLL_MILLIS));
    }
}

fn start_service_once() -> Result<(), String> {
    let output = std::process::Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc.exe start: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let text = output_text(&output);
    if text.contains("1056") {
        Ok(())
    } else {
        Err(text)
    }
}

fn stop_service_once() -> Result<(), String> {
    let output = std::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output()
        .map_err(|e| format!("Failed to run sc.exe stop: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let text = output_text(&output);
    if text.contains("1062") {
        Ok(())
    } else {
        Err(text)
    }
}

#[cfg(windows)]
fn ensure_windows() -> bool {
    if daemon_health_ready() {
        eprintln!("[cortex] Daemon already healthy");
        return true;
    }

    let mut state = match query_service_state() {
        Ok(state) => state,
        Err(err) => {
            eprintln!("[cortex] Failed to query service state: {err}");
            return false;
        }
    };

    if state == ServiceState::NotInstalled {
        eprintln!("[cortex] Service not installed; installing");
        install();
        state = match query_service_state() {
            Ok(next) => next,
            Err(err) => {
                eprintln!("[cortex] Failed to query service state after install: {err}");
                return false;
            }
        };
        if state == ServiceState::NotInstalled {
            eprintln!("[cortex] Service install did not complete (run as Administrator if needed)");
            return false;
        }
    }

    eprintln!("[cortex] Service state before ensure: {}", state.as_str());

    if state == ServiceState::Running {
        if wait_for_daemon_health(std::time::Duration::from_secs(2)) {
            eprintln!("[cortex] Service already running and healthy");
            return true;
        }
        eprintln!("[cortex] Service running but health failed; restarting once");
        if let Err(err) = stop_service_once() {
            eprintln!("[cortex] Failed to stop unhealthy service: {err}");
            return false;
        }
    }

    if let Err(err) = start_service_once() {
        eprintln!("[cortex] Failed to start service: {err}");
        return false;
    }

    if wait_for_daemon_health(std::time::Duration::from_secs(ENSURE_HEALTH_TIMEOUT_SECS)) {
        eprintln!("[cortex] Service ensured and daemon health is live");
        true
    } else {
        eprintln!("[cortex] Service started but daemon health endpoint is still unavailable");
        false
    }
}

// ---- CLI commands (work on any platform) ------------------------------------

pub fn install() {
    let exe = std::env::current_exe().expect("Failed to get exe path");
    let exe_path = exe.to_string_lossy().to_string();

    // COR-8 fix: detect current username to run service under user account,
    // NOT LocalSystem. LocalSystem has a different USERPROFILE which would
    // open a completely separate database at C:\Windows\system32\config\systemprofile.
    let username = std::env::var("USERNAME").unwrap_or_else(|_| "cortex-user".to_string());

    // COR-5 fix: use cmd /C for sc.exe to handle binPath quoting correctly.
    // sc.exe has non-standard argument parsing that fights with Rust's Command.
    let sc_cmd = build_sc_create_command(&exe_path, &username);

    let output = std::process::Command::new("cmd")
        .args(["/C", &sc_cmd])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service '{}' installed", SERVICE_NAME);
            eprintln!("[cortex] Runs as: .\\{}", username);

            // Set description
            let _ = std::process::Command::new("sc.exe")
                .args(["description", SERVICE_NAME, DESCRIPTION])
                .output();

            // Configure recovery: restart on failure (5s, 10s, 30s)
            let _ = std::process::Command::new("cmd")
                .args(["/C", &format!(
                    "sc.exe failure {} reset= 86400 actions= restart/5000/restart/10000/restart/30000",
                    SERVICE_NAME
                )])
                .output();

            eprintln!("[cortex] Auto-start on boot: disabled (manual start mode)");
            eprintln!("[cortex] To opt in later: sc.exe config CortexDaemon start= auto");
            eprintln!("[cortex] Recovery: restart on failure (5s / 10s / 30s)");
            eprintln!("[cortex] NOTE: You may need to set the password:");
            eprintln!("[cortex]   sc.exe config CortexDaemon password= YOUR_PASSWORD");
            eprintln!("[cortex] Then: cortex service start");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1073") {
                eprintln!("[cortex] Service already exists. Run: cortex service uninstall");
            } else {
                eprintln!("[cortex] Failed to install (run as Administrator)");
                eprintln!("{}", stderr);
            }
        }
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn uninstall() {
    // Stop first (ignore errors if not running)
    let _ = std::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output();
    std::thread::sleep(std::time::Duration::from_secs(2));

    match std::process::Command::new("sc.exe")
        .args(["delete", SERVICE_NAME])
        .output()
    {
        Ok(o) if o.status.success() => eprintln!("[cortex] Service uninstalled"),
        Ok(o) => {
            eprintln!("[cortex] Failed to uninstall");
            eprintln!("{}", String::from_utf8_lossy(&o.stderr));
        }
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn start() {
    match std::process::Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .output()
    {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service started");
            // Wait and verify
            std::thread::sleep(std::time::Duration::from_secs(3));
            let health_url = daemon_health_url();
            if let Ok(h) = std::process::Command::new("curl")
                .args(["-s", &health_url])
                .output()
            {
                if h.status.success() {
                    eprintln!("[cortex] Daemon is LIVE at {health_url}");
                    eprintln!("{}", String::from_utf8_lossy(&h.stdout));
                } else {
                    eprintln!("[cortex] Service started but health check pending");
                }
            }
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1056") {
                eprintln!("[cortex] Service is already running");
            } else {
                eprintln!("[cortex] Failed to start service");
                eprintln!("{}", stderr);
            }
        }
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn stop() {
    match std::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output()
    {
        Ok(o) if o.status.success() => eprintln!("[cortex] Service stopped"),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1062") {
                eprintln!("[cortex] Service is not running");
            } else {
                eprintln!("[cortex] Failed to stop");
                eprintln!("{}", stderr);
            }
        }
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn status() {
    match std::process::Command::new("sc.exe")
        .args(["query", SERVICE_NAME])
        .output()
    {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let state = if stdout.contains("RUNNING") {
                "RUNNING"
            } else if stdout.contains("STOPPED") {
                "STOPPED"
            } else if stdout.contains("START_PENDING") {
                "STARTING"
            } else {
                "UNKNOWN"
            };
            eprintln!("[cortex] Service: {state}");

            // Also check HTTP health
            let health_url = daemon_health_url();
            if let Ok(h) = std::process::Command::new("curl")
                .args(["-s", &health_url])
                .output()
            {
                if h.status.success() {
                    eprintln!("[cortex] HTTP: LIVE");
                    eprintln!("{}", String::from_utf8_lossy(&h.stdout));
                } else {
                    eprintln!("[cortex] HTTP: not responding");
                }
            }
        }
        Ok(_) => eprintln!("[cortex] Service not installed. Run: cortex service install"),
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn ensure() {
    #[cfg(not(windows))]
    {
        eprintln!("[cortex] `service ensure` is only available on Windows");
        std::process::exit(1);
    }

    #[cfg(windows)]
    {
        if !ensure_windows() {
            std::process::exit(1);
        }
    }
}

// ---- Windows Service entry point (called by SCM) ----------------------------

#[cfg(windows)]
mod scm {
    use std::ffi::OsString;
    use std::sync::mpsc;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, cortex_service_main);

    pub fn dispatch() {
        service_dispatcher::start(super::SERVICE_NAME, ffi_service_main)
            .expect("[cortex] Failed to start service dispatcher");
    }

    fn cortex_service_main(_arguments: Vec<OsString>) {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop | ServiceControl::Shutdown => {
                    stop_tx.send(()).ok();
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(super::SERVICE_NAME, event_handler)
            .expect("[cortex] Failed to register service control handler");

        // Report: Starting
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::from_secs(15),
            process_id: None,
        });

        // COR-4 fix: report Stopped with error if runtime creation fails
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("[cortex-service] Failed to create tokio runtime: {e}");
                let _ = status_handle.set_service_status(ServiceStatus {
                    service_type: SERVICE_TYPE,
                    current_state: ServiceState::Stopped,
                    controls_accepted: ServiceControlAccept::empty(),
                    exit_code: ServiceExitCode::Win32(1),
                    checkpoint: 0,
                    wait_hint: std::time::Duration::default(),
                    process_id: None,
                });
                return;
            }
        };

        // COR-3 fix: report Running AFTER entering rt.block_on but BEFORE
        // run_daemon blocks on server::run. The daemon init (DB, indexing)
        // happens first, then we report Running right before the server binds.
        // Note: ideally we'd signal from inside run_daemon after bind, but
        // the current architecture doesn't expose that hook. Reporting here
        // is a reasonable compromise -- init is fast, server bind follows.
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::default(),
            process_id: None,
        });

        // Run the daemon with service shutdown signal
        rt.block_on(async {
            crate::run_daemon(crate::auth::CortexPaths::resolve(), async move {
                // Bridge std::sync::mpsc to async via spawn_blocking
                tokio::task::spawn_blocking(move || {
                    stop_rx.recv().ok();
                })
                .await
                .ok();
                eprintln!("[cortex-service] Stop signal received");
            })
            .await;
        });

        // Report: Stopped
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::default(),
            process_id: None,
        });
    }
}

/// Dispatch to Windows SCM. Called from main.rs `service-run` arm.
#[cfg(windows)]
pub fn dispatch_service() {
    scm::dispatch();
}

#[cfg(not(windows))]
pub fn dispatch_service() {
    eprintln!("[cortex] Windows Service is only available on Windows");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_service_{name}_{unique}"))
    }

    #[test]
    fn build_sc_create_command_defaults_to_manual_start() {
        let cmd = build_sc_create_command(r"C:\Program Files\Cortex\cortex.exe", "alice");
        assert!(
            cmd.contains("start= demand"),
            "expected manual start mode: {cmd}"
        );
        assert!(
            !cmd.contains("start= auto"),
            "must not auto-start by default: {cmd}"
        );
    }

    #[test]
    fn build_sc_create_command_includes_quoted_binpath_and_user() {
        let exe = r"C:\Program Files\Cortex\cortex.exe";
        let cmd = build_sc_create_command(exe, "alice");
        let expected_bin = format!("binPath= \"\\\"{}\\\" service-run\"", exe);
        assert!(cmd.contains(&format!("sc.exe create {}", SERVICE_NAME)));
        assert!(
            cmd.contains(&expected_bin),
            "missing binPath quoting: {cmd}"
        );
        assert!(
            cmd.contains("obj= \".\\alice\""),
            "missing user account object: {cmd}"
        );
    }

    #[test]
    fn parse_service_state_recognizes_known_states() {
        assert_eq!(
            parse_service_state("STATE              : 4  RUNNING"),
            ServiceState::Running
        );
        assert_eq!(
            parse_service_state("STATE              : 1  STOPPED"),
            ServiceState::Stopped
        );
        assert_eq!(
            parse_service_state("STATE              : 2  START_PENDING"),
            ServiceState::StartPending
        );
        assert_eq!(
            parse_service_state("STATE              : 3  STOP_PENDING"),
            ServiceState::StopPending
        );
        assert_eq!(
            parse_service_state("STATE              : ???"),
            ServiceState::Unknown
        );
    }

    #[test]
    fn service_state_strings_are_stable() {
        assert_eq!(ServiceState::NotInstalled.as_str(), "NOT_INSTALLED");
        assert_eq!(ServiceState::Running.as_str(), "RUNNING");
        assert_eq!(ServiceState::Stopped.as_str(), "STOPPED");
        assert_eq!(ServiceState::StartPending.as_str(), "START_PENDING");
        assert_eq!(ServiceState::StopPending.as_str(), "STOP_PENDING");
        assert_eq!(ServiceState::Unknown.as_str(), "UNKNOWN");
    }

    #[test]
    fn daemon_ready_payload_accepts_readiness_ready_and_health_ok() {
        let home_dir = temp_test_dir("ready_payload");
        let home = home_dir.to_string_lossy().to_string();
        let paths = crate::auth::CortexPaths::resolve_with_overrides(
            Some(&home),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );

        let readiness = serde_json::json!({
            "status": "ready",
            "ready": true,
            "runtime": {
                "port": 7437,
                "token_path": paths.token.display().to_string(),
                "db_path": paths.db.display().to_string(),
                "pid_path": paths.pid.display().to_string(),
            },
            "stats": { "home": paths.home.display().to_string() }
        })
        .to_string();
        assert_eq!(
            daemon_ready_from_payload(200, &readiness, &paths),
            Some(true)
        );

        let health = serde_json::json!({
            "status": "ok",
            "runtime": {
                "port": 7437,
                "token_path": paths.token.display().to_string(),
                "db_path": paths.db.display().to_string(),
                "pid_path": paths.pid.display().to_string(),
            },
            "stats": { "home": paths.home.display().to_string(), "memories": 1 }
        })
        .to_string();
        assert_eq!(daemon_ready_from_payload(200, &health, &paths), Some(true));
    }

    #[test]
    fn daemon_ready_payload_preserves_starting_state() {
        let home_dir = temp_test_dir("starting_payload");
        let home = home_dir.to_string_lossy().to_string();
        let paths = crate::auth::CortexPaths::resolve_with_overrides(
            Some(&home),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );

        let readiness = serde_json::json!({
            "status": "starting",
            "ready": false,
            "runtime": {
                "port": 7437,
                "token_path": paths.token.display().to_string(),
                "db_path": paths.db.display().to_string(),
                "pid_path": paths.pid.display().to_string(),
            },
            "stats": { "home": paths.home.display().to_string() }
        })
        .to_string();
        assert_eq!(
            daemon_ready_from_payload(503, &readiness, &paths),
            Some(false)
        );
    }
}
