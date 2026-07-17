const SERVICE_NAME: &str = "CortexDaemon";
const DISPLAY_NAME: &str = "Cortex Memory Daemon";
const DESCRIPTION: &str = "Always-on AI memory daemon -- serves Claude, Gemini, Codex, Cursor, and local LLMs via HTTP (:7437) and MCP.";
const DEFAULT_START_MODE: &str = "demand";
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
fn daemon_ready_from_payload(status: u16, body: &str, paths: &crate::auth::CortexPaths) -> Option<bool> {
    if let Some(ready) = crate::daemon_lifecycle::readiness_state_from_payload(status, body, Some(paths.port), Some(paths)) {
        return Some(ready);
    }
    if crate::daemon_lifecycle::is_cortex_health_payload(status, body, Some(paths.port), Some(paths)) {
        return Some(true);
    }
    None
}
fn escape_cmd_quoted_fragment(value: &str) -> String {
    value.replace('^', "^^").replace('%', "^%")
}
fn build_sc_create_command(exe_path: &str, username: &str) -> String {
    let exe_path = escape_cmd_quoted_fragment(exe_path);
    let username = escape_cmd_quoted_fragment(username);
    format!(
        "sc.exe create {} binPath= \"\\\"{}\\\" service-run\" start= {} DisplayName= \"{}\" obj= \".\\{}\"",
        SERVICE_NAME, exe_path, DEFAULT_START_MODE, DISPLAY_NAME, username
    )
}
fn username_is_safe_for_cmd_fragment(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ' ' | '.' | '_' | '-'))
}
fn resolve_service_username_from_env() -> String {
    match std::env::var("USERNAME") {
        Ok(raw) => {
            let trimmed = raw.trim();
            if username_is_safe_for_cmd_fragment(trimmed) {
                trimmed.to_string()
            } else {
                "cortex-user".to_string()
            }
        }
        Err(_) => "cortex-user".to_string(),
    }
}
fn service_exe_path_from_result(result: std::io::Result<std::path::PathBuf>) -> Result<String, String> {
    result
        .map(|exe| exe.to_string_lossy().to_string())
        .map_err(|err| format!("Failed to get exe path: {err}"))
}
fn service_exe_path() -> Result<String, String> {
    service_exe_path_from_result(std::env::current_exe())
}
#[cfg(windows)]
const ENSURE_HEALTH_TIMEOUT_SECS: u64 = 12;
#[cfg(windows)]
const ENSURE_POLL_MILLIS: u64 = 250;
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceState {
    NotInstalled,
    Running,
    Stopped,
    StartPending,
    StopPending,
    Unknown,
}
#[cfg(windows)]
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
#[cfg(windows)]
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
#[cfg(windows)]
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
#[cfg(windows)]
fn query_service_state() -> Result<ServiceState, String> {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["query", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command.output().map_err(|e| format!("Failed to run sc.exe query: {e}"))?;
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
    let (status, body) = crate::transport::parse_http_response_bytes(raw, "Cortex daemon")?;
    Ok((status.as_u16(), body))
}
fn should_use_partial_probe_response(err: &std::io::Error, response_len: usize) -> bool {
    response_len > 0 && matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
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
        .set_read_timeout(Some(std::time::Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS)))
        .map_err(|e| format!("read timeout failed: {e}"))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS)))
        .map_err(|e| format!("write timeout failed: {e}"))?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Cortex-Request: true\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).map_err(|e| format!("write failed: {e}"))?;
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
#[cfg(windows)]
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
#[cfg(windows)]
fn start_service_once() -> Result<(), String> {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["start", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command.output().map_err(|e| format!("Failed to run sc.exe start: {e}"))?;
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
#[cfg(windows)]
fn stop_service_once() -> Result<(), String> {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    let output = command.output().map_err(|e| format!("Failed to run sc.exe stop: {e}"))?;
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
pub fn install() -> bool {
    let exe_path = match service_exe_path() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("[cortex] {err}");
            return false;
        }
    };
    let username_env = std::env::var("USERNAME").ok();
    let username = resolve_service_username_from_env();
    if let Some(raw) = username_env {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && trimmed != username {
            eprintln!("[cortex] Warning: USERNAME contains unsupported characters; falling back to '{}'", username);
        }
    }
    let sc_cmd = build_sc_create_command(&exe_path, &username);
    let mut create_cmd = std::process::Command::new("cmd");
    create_cmd.args(["/V:OFF", "/C", &sc_cmd]);
    apply_hidden_process_flags(&mut create_cmd);
    let output = create_cmd.output();
    match output {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service '{}' installed", SERVICE_NAME);
            eprintln!("[cortex] Runs as: .\\{}", username);
            let mut description_cmd = std::process::Command::new("sc.exe");
            description_cmd.args(["description", SERVICE_NAME, DESCRIPTION]);
            apply_hidden_process_flags(&mut description_cmd);
            let _ = description_cmd.output();
            let mut failure_cmd = std::process::Command::new("cmd");
            failure_cmd.args([
                "/C",
                &format!("sc.exe failure {} reset= 86400 actions= restart/5000/restart/10000/restart/30000", SERVICE_NAME),
            ]);
            apply_hidden_process_flags(&mut failure_cmd);
            let _ = failure_cmd.output();
            eprintln!("[cortex] Auto-start on boot: disabled (manual start mode)");
            eprintln!("[cortex] To opt in later: sc.exe config CortexDaemon start= auto");
            eprintln!("[cortex] Recovery: restart on failure (5s / 10s / 30s)");
            eprintln!("[cortex] NOTE: You may need to set the password:");
            eprintln!("[cortex]   sc.exe config CortexDaemon password= YOUR_PASSWORD");
            eprintln!("[cortex] Then: cortex service start");
            true
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1073") {
                eprintln!("[cortex] Service already exists. Run: cortex service uninstall");
            } else {
                eprintln!("[cortex] Failed to install (run as Administrator)");
                eprintln!("{}", stderr);
            }
            false
        }
        Err(e) => {
            eprintln!("[cortex] Failed to run sc.exe: {e}");
            false
        }
    }
}
pub fn uninstall() -> bool {
    let mut stop_cmd = std::process::Command::new("sc.exe");
    stop_cmd.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut stop_cmd);
    let _ = stop_cmd.output();
    std::thread::sleep(std::time::Duration::from_secs(2));
    let mut delete_cmd = std::process::Command::new("sc.exe");
    delete_cmd.args(["delete", SERVICE_NAME]);
    apply_hidden_process_flags(&mut delete_cmd);
    match delete_cmd.output() {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service uninstalled");
            true
        }
        Ok(o) => {
            eprintln!("[cortex] Failed to uninstall");
            eprintln!("{}", String::from_utf8_lossy(&o.stderr));
            false
        }
        Err(e) => {
            eprintln!("[cortex] Failed to run sc.exe: {e}");
            false
        }
    }
}
pub fn start() -> bool {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["start", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    match command.output() {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service started");
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
            true
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1056") {
                eprintln!("[cortex] Service is already running");
                true
            } else {
                eprintln!("[cortex] Failed to start service");
                eprintln!("{}", stderr);
                false
            }
        }
        Err(e) => {
            eprintln!("[cortex] Failed to run sc.exe: {e}");
            false
        }
    }
}
pub fn stop() -> bool {
    let mut command = std::process::Command::new("sc.exe");
    command.args(["stop", SERVICE_NAME]);
    apply_hidden_process_flags(&mut command);
    match command.output() {
        Ok(o) if o.status.success() => {
            eprintln!("[cortex] Service stopped");
            true
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("1062") {
                eprintln!("[cortex] Service is not running");
                true
            } else {
                eprintln!("[cortex] Failed to stop");
                eprintln!("{}", stderr);
                false
            }
        }
        Err(e) => {
            eprintln!("[cortex] Failed to run sc.exe: {e}");
            false
        }
    }
}
pub fn status() -> bool {
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
            if daemon_health_ready() {
                eprintln!("[cortex] HTTP: LIVE");
                if let Ok((_, body)) = daemon_probe("/health") {
                    eprintln!("{body}");
                }
            } else {
                eprintln!("[cortex] HTTP: not responding");
            }
            true
        }
        Ok(_) => {
            eprintln!("[cortex] Service not installed. Run: cortex service install");
            false
        }
        Err(e) => {
            eprintln!("[cortex] Failed to run sc.exe: {e}");
            false
        }
    }
}
pub fn ensure() -> bool {
    #[cfg(not(windows))]
    {
        eprintln!("[cortex] `service ensure` is only available on Windows");
        false
    }
    #[cfg(windows)]
    {
        ensure_windows()
    }
}
#[cfg(windows)]
pub fn ensure_ready() -> bool {
    ensure_windows()
}
#[cfg(not(windows))]
#[allow(dead_code)]
pub fn ensure_ready() -> bool {
    false
}
#[cfg(windows)]
mod scm {
    use std::ffi::OsString;
    use std::sync::mpsc;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};
    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;
    define_windows_service!(ffi_service_main, cortex_service_main);
    pub fn dispatch() {
        if let Err(err) = service_dispatcher::start(super::SERVICE_NAME, ffi_service_main) {
            eprintln!("[cortex] Failed to start service dispatcher: {err}");
            std::process::exit(1);
        }
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
        let status_handle = match service_control_handler::register(super::SERVICE_NAME, event_handler) {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("[cortex-service] Failed to register service control handler: {err}");
                return;
            }
        };
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::from_secs(15),
            process_id: None,
        });
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
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: std::time::Duration::default(),
            process_id: None,
        });
        rt.block_on(async {
            crate::run_daemon(crate::auth::CortexPaths::resolve(), async move {
                tokio::task::spawn_blocking(move || {
                    stop_rx.recv().ok();
                })
                .await
                .ok();
                eprintln!("[cortex-service] Stop signal received");
            })
            .await;
        });
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
mod tests;
