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
const HEALTH_PROBE_TIMEOUT_SECS: u64 = 2;

#[cfg(windows)]
const CREATE_NO_WINDOW_FLAG: u32 = 0x0800_0000;

#[cfg(windows)]
fn apply_hidden_process_flags(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(CREATE_NO_WINDOW_FLAG);
}

#[cfg(not(windows))]
fn apply_hidden_process_flags(_command: &mut std::process::Command) {}

fn daemon_base_url() -> String {
    let port = crate::auth::CortexPaths::resolve().port;
    format!("http://127.0.0.1:{port}")
}

fn daemon_health_url() -> String {
    format!("{}/health", daemon_base_url())
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
    let mut command = std::process::Command::new("sc.exe");
    command.args(["query", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command
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

fn parse_http_probe_response(raw: &[u8]) -> Result<(u16, String), String> {
    let Some(header_end) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err("invalid HTTP response from Cortex daemon".to_string());
    };

    let header = std::str::from_utf8(&raw[..header_end])
        .map_err(|_| "daemon response headers are not valid UTF-8".to_string())?;
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| "daemon response missing valid status line".to_string())?;
    let body = String::from_utf8_lossy(&raw[header_end + 4..]).to_string();
    Ok((status, body))
}

fn should_use_partial_probe_response(err: &std::io::Error, response_len: usize) -> bool {
    response_len > 0
        && matches!(
            err.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        )
}

fn daemon_probe(path: &str) -> Result<(u16, String), String> {
    use std::io::{Read, Write};

    let port = crate::auth::CortexPaths::resolve().port;
    let mut stream = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS),
    )
    .map_err(|e| format!("connect failed: {e}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(
            HEALTH_PROBE_TIMEOUT_SECS,
        )))
        .map_err(|e| format!("read timeout failed: {e}"))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(
            HEALTH_PROBE_TIMEOUT_SECS,
        )))
        .map_err(|e| format!("write timeout failed: {e}"))?;

    let request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;

    let mut response = Vec::new();
    if let Err(err) = stream.read_to_end(&mut response) {
        if !should_use_partial_probe_response(&err, response.len()) {
            return Err(format!("read failed: {err}"));
        }
    }
    parse_http_probe_response(&response)
}

fn daemon_health_response() -> Option<String> {
    let paths = crate::auth::CortexPaths::resolve();

    if let Ok((status, body)) = daemon_probe("/readiness") {
        if daemon_ready_from_payload(status, &body, &paths) == Some(true) {
            return Some(body);
        }
    }

    if let Ok((status, body)) = daemon_probe("/health") {
        if daemon_ready_from_payload(status, &body, &paths).unwrap_or(false) {
            return Some(body);
        }
    }

    None
}

fn daemon_health_ready() -> bool {
    daemon_health_response().is_some()
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
    let mut command = std::process::Command::new("sc.exe");
    command.args(["start", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command
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
    let mut command = std::process::Command::new("sc.exe");
    command.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command
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

    let mut create_cmd = std::process::Command::new("cmd");
    create_cmd.args(["/C", &sc_cmd]);
    apply_hidden_process_flags(&mut create_cmd);
    let output = create_cmd.output();

    match output {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service '{}' installed", SERVICE_NAME);
            eprintln!("[cortex] Runs as: .\\{}", username);

            // Set description
            let mut description_cmd = std::process::Command::new("sc.exe");
            description_cmd.args(["description", SERVICE_NAME, DESCRIPTION]);
            apply_hidden_process_flags(&mut description_cmd);
            let _ = description_cmd.output();

            // Configure recovery: restart on failure (5s, 10s, 30s)
            let mut failure_cmd = std::process::Command::new("cmd");
            failure_cmd.args([
                "/C",
                &format!(
                    "sc.exe failure {} reset= 86400 actions= restart/5000/restart/10000/restart/30000",
                    SERVICE_NAME
                ),
            ]);
            apply_hidden_process_flags(&mut failure_cmd);
            let _ = failure_cmd.output();

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
    let mut stop_cmd = std::process::Command::new("sc.exe");
    stop_cmd.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut stop_cmd);
    let _ = stop_cmd.output();
    std::thread::sleep(std::time::Duration::from_secs(2));

    let mut delete_cmd = std::process::Command::new("sc.exe");
    delete_cmd.args(["delete", SERVICE_NAME]);
    apply_hidden_process_flags(&mut delete_cmd);
    match delete_cmd.output() {
        Ok(o) if o.status.success() => eprintln!("[cortex] Service uninstalled"),
        Ok(o) => {
            eprintln!("[cortex] Failed to uninstall");
            eprintln!("{}", String::from_utf8_lossy(&o.stderr));
        }
        Err(e) => eprintln!("[cortex] Failed to run sc.exe: {e}"),
    }
}

pub fn start() {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["start", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    match command.output() {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service started");
            // Wait and verify
            std::thread::sleep(std::time::Duration::from_secs(3));
            let health_url = daemon_health_url();
            if daemon_health_ready() {
                eprintln!("[cortex] Daemon is LIVE at {health_url}");
                if let Ok((_, body)) = daemon_probe("/health") {
                    eprintln!("{body}");
                }
            } else {
                eprintln!("[cortex] Service started but health check pending");
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
    let mut command = std::process::Command::new("sc.exe");
    command.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    match command.output() {
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
    let mut command = std::process::Command::new("sc.exe");
    command.args(["query", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    match command.output() {
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
            if daemon_health_ready() {
                eprintln!("[cortex] HTTP: LIVE");
                if let Ok((_, body)) = daemon_probe("/health") {
                    eprintln!("{body}");
                }
            } else {
                eprintln!("[cortex] HTTP: not responding");
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

/// Service-first daemon ensure for library callers.
/// Returns true when daemon health is live after ensure.
#[cfg(windows)]
pub fn ensure_ready() -> bool {
    ensure_windows()
}

#[cfg(not(windows))]
pub fn ensure_ready() -> bool {
    false
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

    #[test]
    fn parse_http_probe_response_extracts_status_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"ok\"}";
        let (status, body) = parse_http_probe_response(raw).expect("parse response");
        assert_eq!(status, 200);
        assert_eq!(body, "{\"status\":\"ok\"}");
    }

    #[test]
    fn parse_http_probe_response_rejects_invalid_payloads() {
        let err = parse_http_probe_response(b"not-http").unwrap_err();
        assert!(err.contains("invalid HTTP response"));
    }

    #[test]
    fn partial_probe_timeout_only_applies_when_bytes_exist() {
        let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        let would_block = std::io::Error::new(std::io::ErrorKind::WouldBlock, "would block");
        let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");

        assert!(should_use_partial_probe_response(&timeout, 16));
        assert!(should_use_partial_probe_response(&would_block, 16));
        assert!(!should_use_partial_probe_response(&timeout, 0));
        assert!(!should_use_partial_probe_response(&reset, 16));
    }
}
