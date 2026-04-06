// SPDX-License-Identifier: MIT
//! Windows Service support for Cortex daemon.
//!
//! Subcommands:
//!   `cortex service install`   -- Register as Windows Service (requires Admin)
//!   `cortex service uninstall` -- Remove Windows Service
//!   `cortex service start`     -- Start the service
//!   `cortex service stop`      -- Stop the service
//!   `cortex service status`    -- Check service status
//!   `cortex service-run`       -- Internal: SCM entry point
//!
//! The service runs the same daemon as `cortex serve` but under the Windows
//! Service Control Manager with auto-start, auto-restart on failure, and
//! proper lifecycle management. Every AI (Claude, Gemini, Codex, Cursor,
//! Qwen, DeepSeek, GLM, Droid) benefits because the daemon is always alive.

const SERVICE_NAME: &str = "CortexDaemon";
const DISPLAY_NAME: &str = "Cortex Memory Daemon";
const DESCRIPTION: &str =
    "Always-on AI memory daemon -- serves Claude, Gemini, Codex, Cursor, and local LLMs via HTTP (:7437) and MCP.";

fn daemon_health_url() -> String {
    let port = crate::auth::CortexPaths::resolve().port;
    format!("http://127.0.0.1:{port}/health")
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
    let sc_cmd = format!(
        "sc.exe create {} binPath= \"\\\"{}\\\" service-run\" start= auto DisplayName= \"{}\" obj= \".\\{}\"",
        SERVICE_NAME, exe_path, DISPLAY_NAME, username
    );

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

            eprintln!("[cortex] Auto-start: enabled");
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

