// SPDX-License-Identifier: MIT
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sidecar;

use rusqlite::Connection;
use serde::Serialize;
use sidecar::SidecarDaemon;
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, Runtime, State};

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "cortex-tray";
const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_HIDE_ID: &str = "tray_hide";
const TRAY_QUIT_ID: &str = "tray_quit";
const DEFAULT_DAEMON_PORT: u16 = 7437;
const DAEMON_REACHABILITY_TIMEOUT_MS: u64 = 400;
const DAEMON_CONNECT_TIMEOUT_MS: u64 = 1_200;
const DAEMON_READ_TIMEOUT_MS: u64 = 10_000;
const DAEMON_WRITE_TIMEOUT_MS: u64 = 3_000;
const DAEMON_START_WAIT_MS: u64 = 3_000;
const DAEMON_STOP_WAIT_MS: u64 = 3_000;
const DAEMON_WAIT_POLL_MS: u64 = 200;

struct DaemonState {
    daemon: Mutex<SidecarDaemon>,
}

impl DaemonState {
    fn new(exe_path: Option<PathBuf>) -> Self {
        let daemon = match exe_path {
            Some(path) => SidecarDaemon::with_exe_path(path),
            None => SidecarDaemon::default(),
        };
        Self {
            daemon: Mutex::new(daemon),
        }
    }

    fn status(&self) -> Result<(bool, Option<u32>), String> {
        let mut daemon = self
            .daemon
            .lock()
            .map_err(|_| "Failed to lock daemon state".to_string())?;
        let s = daemon.status();
        Ok((s.running, s.pid))
    }

    fn start(&self) -> Result<(bool, Option<u32>), String> {
        let mut daemon = self
            .daemon
            .lock()
            .map_err(|_| "Failed to lock daemon state".to_string())?;
        let s = daemon.start()?;
        Ok((s.running, s.pid))
    }

    fn stop(&self) -> Result<(), String> {
        let mut daemon = self
            .daemon
            .lock()
            .map_err(|_| "Failed to lock daemon state".to_string())?;
        daemon.stop()?;
        Ok(())
    }
}

struct LifecycleState {
    explicit_quit: AtomicBool,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            explicit_quit: AtomicBool::new(false),
        }
    }
}

impl LifecycleState {
    fn request_quit(&self) {
        self.explicit_quit.store(true, Ordering::SeqCst);
    }

    fn is_quit_requested(&self) -> bool {
        self.explicit_quit.load(Ordering::SeqCst)
    }
}

#[derive(Serialize)]
struct DaemonCommandResult {
    running: bool,
    reachable: bool,
    pid: Option<u32>,
    message: String,
}

fn cortex_home() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "Could not resolve USERPROFILE/HOME".to_string())
}

fn token_path() -> Result<PathBuf, String> {
    Ok(cortex_home()?.join(".cortex").join("cortex.token"))
}

fn cortex_db_path() -> Result<PathBuf, String> {
    Ok(cortex_home()?.join(".cortex").join("cortex.db"))
}

fn daemon_port() -> u16 {
    static CACHED_DAEMON_PORT: OnceLock<u16> = OnceLock::new();
    *CACHED_DAEMON_PORT.get_or_init(resolve_daemon_port)
}

fn is_cortex_reachable_with_port(port: u16, timeout_ms: u64) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(timeout_ms)).is_ok()
}

async fn wait_for_reachability(port: u16, target: bool, timeout: Duration) -> bool {
    tauri::async_runtime::spawn_blocking(move || {
        let started = std::time::Instant::now();
        loop {
            if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) == target {
                return true;
            }
            if started.elapsed() >= timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(DAEMON_WAIT_POLL_MS));
        }
    })
    .await
    .unwrap_or(false)
}

fn cortex_binary_name() -> &'static str {
    if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    }
}

fn parse_port_from_paths_json(output: &[u8]) -> Result<u16, String> {
    let json: serde_json::Value = serde_json::from_slice(output)
        .map_err(|err| format!("Invalid JSON from `cortex paths --json`: {err}"))?;
    let port = json
        .get("port")
        .and_then(|value| value.as_u64())
        .ok_or_else(|| "Missing `port` in `cortex paths --json` output".to_string())?;
    u16::try_from(port).map_err(|err| format!("Port value out of range ({port}): {err}"))
}

fn resolve_port_with_binary(binary: impl AsRef<std::ffi::OsStr>) -> Result<Option<u16>, String> {
    let output = Command::new(binary)
        .args(["paths", "--json"])
        .output()
        .map_err(|err| format!("Failed to execute `cortex paths --json`: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Ok(None);
        }
        return Err(format!("`cortex paths --json` failed: {stderr}"));
    }
    parse_port_from_paths_json(&output.stdout).map(Some)
}

fn resolve_daemon_port() -> u16 {
    match resolve_port_with_binary("cortex") {
        Ok(Some(port)) => return port,
        Ok(None) => {}
        Err(err) => eprintln!("[cortex-control-center] {err}"),
    }

    if let Some(binary) = find_cortex_binary() {
        match resolve_port_with_binary(&binary) {
            Ok(Some(port)) => return port,
            Ok(None) => {}
            Err(err) => eprintln!("[cortex-control-center] {err}"),
        }
    }

    match env::var("CORTEX_PORT") {
        Ok(value) => match value.parse::<u16>() {
            Ok(port) => port,
            Err(err) => {
                eprintln!("[cortex-control-center] Invalid CORTEX_PORT '{value}': {err}");
                DEFAULT_DAEMON_PORT
            }
        },
        Err(env::VarError::NotPresent) => DEFAULT_DAEMON_PORT,
        Err(err) => {
            eprintln!("[cortex-control-center] Failed to read CORTEX_PORT: {err}");
            DEFAULT_DAEMON_PORT
        }
    }
}

fn find_cortex_binary() -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sidecar = dir.join(cortex_binary_name());
            if sidecar.exists() {
                return Some(sidecar);
            }
        }
    }

    if let Ok(home) = cortex_home() {
        let plugin_path = home.join(".cortex").join("bin").join(cortex_binary_name());
        if plugin_path.exists() {
            return Some(plugin_path);
        }

        // Prefer release binary (built by beforeBuildCommand)
        let release_path = home
            .join("cortex")
            .join("daemon-rs")
            .join("target")
            .join("release")
            .join(cortex_binary_name());
        if release_path.exists() {
            return Some(release_path);
        }

        // Fall back to debug binary (built by beforeDevCommand)
        let debug_path = home
            .join("cortex")
            .join("daemon-rs")
            .join("target")
            .join("debug")
            .join(cortex_binary_name());
        if debug_path.exists() {
            return Some(debug_path);
        }
    }

    None
}

fn show_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.minimize();
        let _ = window.hide();
    }
}

fn request_app_quit<R: Runtime>(app: &tauri::AppHandle<R>) {
    let lifecycle = app.state::<LifecycleState>();
    lifecycle.request_quit();
    app.exit(0);
}

fn shutdown_daemon<R: Runtime>(app: &tauri::AppHandle<R>) {
    let daemon_state = app.state::<DaemonState>();
    let _ = daemon_state.stop();
    let _ = flush_cortex_db_on_shutdown();
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
    conn.execute_batch(
        r#"
    PRAGMA journal_mode = WAL;
    PRAGMA wal_checkpoint(TRUNCATE);
    "#,
    )
    .map_err(|err| {
        format!(
            "Failed to flush WAL on shutdown {}: {err}",
            db_path.display()
        )
    })?;
    conn.close()
        .map_err(|(_, err)| format!("Failed to close DB after shutdown flush: {err}"))?;
    Ok(())
}

fn setup_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
    let tray_menu = MenuBuilder::new(app)
        .text(TRAY_SHOW_ID, "Show")
        .text(TRAY_HIDE_ID, "Hide / Minimize")
        .separator()
        .text(TRAY_QUIT_ID, "Quit")
        .build()?;

    let mut tray_builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .tooltip("Cortex Control Center")
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_ID => show_main_window(app),
            TRAY_HIDE_ID => hide_main_window(app),
            TRAY_QUIT_ID => request_app_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon);
    }

    let _tray = tray_builder.build(app)?;
    Ok(())
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle, daemon_state: State<DaemonState>) -> Result<(), String> {
    // Kill our own sidecar child if we spawned it
    let _ = daemon_state.stop();
    // Don't HTTP shutdown external daemons on quit -- Claude or other tools
    // may still be using it. Only the Stop button does that explicitly.
    let _ = flush_cortex_db_on_shutdown();
    let lifecycle = app.state::<LifecycleState>();
    lifecycle.request_quit();
    app.exit(0);
    Ok(())
}

#[tauri::command]
fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    hide_main_window(&app);
    Ok(())
}

#[tauri::command]
fn daemon_status(state: State<DaemonState>) -> Result<DaemonCommandResult, String> {
    let (running, pid) = state.status()?;
    let port = daemon_port();
    let reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    let message = if running && reachable {
        format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
    } else if running {
        format!(
            "Cortex daemon running (pid {}) but not reachable on :{} yet.",
            pid.unwrap_or_default(),
            port
        )
    } else if reachable {
        "Cortex daemon reachable (external process).".to_string()
    } else {
        "Cortex daemon is offline.".to_string()
    };

    Ok(DaemonCommandResult {
        running,
        reachable,
        pid,
        message,
    })
}

#[tauri::command]
async fn start_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (running, pid) = state.start()?;
    let reachable = if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        true
    } else {
        wait_for_reachability(port, true, Duration::from_millis(DAEMON_START_WAIT_MS)).await
    };
    let message = if reachable {
        format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
    } else {
        format!(
            "Cortex daemon started (pid {}) but :{} is not reachable yet.",
            pid.unwrap_or_default(),
            port
        )
    };

    Ok(DaemonCommandResult {
        running,
        reachable,
        pid,
        message,
    })
}

/// Send POST /shutdown to the daemon's HTTP endpoint (works for any daemon,
/// regardless of who spawned it). Returns Ok(()) on success or if connection
/// fails (daemon already gone).
fn send_http_shutdown() -> Result<(), String> {
    use std::io::Write;

    let port = daemon_port();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(2000)) {
        Ok(s) => s,
        Err(_) => return Ok(()), // daemon already unreachable
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(2000)));

    let token = token_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    let body = "{}";
    let mut request =
        format!("POST /shutdown HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Cortex-Request: true\r\n");
    if !token.is_empty() {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    request.push_str("Content-Type: application/json\r\n");
    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    request.push_str("Connection: close\r\n\r\n");
    request.push_str(body);

    let _ = stream.write_all(request.as_bytes());
    let _ = stream.flush();
    Ok(())
}

#[tauri::command]
async fn stop_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (was_running, _) = state.status()?;
    let _ = state.stop();

    // If daemon is still reachable (started externally by Claude, CLI, etc.),
    // send a graceful HTTP shutdown signal
    let still_reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if still_reachable {
        let _ = send_http_shutdown();
        let _ =
            wait_for_reachability(port, false, Duration::from_millis(DAEMON_STOP_WAIT_MS)).await;
    }

    let reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    let message = if was_running && !reachable {
        "Stopped Cortex daemon.".to_string()
    } else if !was_running && still_reachable && !reachable {
        "Sent shutdown to external daemon.".to_string()
    } else if reachable {
        "Shutdown signal sent, daemon still shutting down...".to_string()
    } else {
        "Daemon is already stopped.".to_string()
    };

    Ok(DaemonCommandResult {
        running: false,
        reachable,
        pid: None,
        message,
    })
}

#[tauri::command]
fn read_auth_token() -> Result<String, String> {
    let path = token_path()?;
    let token = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read token at {}: {err}", path.display()))?;
    Ok(token.trim().to_string())
}

// ─── HTTP Proxy (bypasses WebView2 mixed-content restrictions) ──────────────

#[tauri::command]
fn fetch_cortex(path: String, auth_token: String) -> Result<FetchCortexResponse, String> {
    send_cortex_request("GET", &path, &auth_token, None)
}

#[tauri::command]
fn post_cortex(
    path: String,
    auth_token: String,
    body: String,
) -> Result<FetchCortexResponse, String> {
    send_cortex_request("POST", &path, &auth_token, Some(&body))
}

fn send_cortex_request(
    method: &str,
    path: &str,
    auth_token: &str,
    body: Option<&str>,
) -> Result<FetchCortexResponse, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    if path.contains('\r') || path.contains('\n') {
        return Err("Invalid request path".to_string());
    }

    let port = daemon_port();
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(DAEMON_CONNECT_TIMEOUT_MS),
    )
    .map_err(|e| format!("Cannot connect to daemon: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(DAEMON_READ_TIMEOUT_MS)))
        .map_err(|e| format!("Cannot set read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(Duration::from_millis(DAEMON_WRITE_TIMEOUT_MS)))
        .map_err(|e| format!("Cannot set write timeout: {e}"))?;

    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Cortex-Request: true\r\n");
    if !auth_token.is_empty() {
        request.push_str(&format!("Authorization: Bearer {auth_token}\r\n"));
    }
    if let Some(payload) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    }
    request.push_str("Connection: close\r\n\r\n");
    if let Some(payload) = body {
        request.push_str(payload);
    }

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|e| format!("Read failed: {e}"))?;

    // Split headers from body
    if let Some(pos) = find_bytes(&response, b"\r\n\r\n") {
        let headers = &response[..pos];
        let body = &response[pos + 4..];
        let headers_text = String::from_utf8_lossy(headers);
        let status = parse_status_code(&headers_text)?;
        let chunked = headers_text.lines().any(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("transfer-encoding:") && lower.contains("chunked")
        });

        // Check for chunked transfer encoding
        let body_bytes = if chunked {
            decode_chunked_bytes(body)?
        } else {
            body.to_vec()
        };
        let body_text = String::from_utf8(body_bytes)
            .map_err(|e| format!("Response body is not valid UTF-8: {e}"))?;
        Ok(FetchCortexResponse {
            status,
            body: body_text,
        })
    } else {
        Err("Invalid HTTP response".to_string())
    }
}

#[derive(Serialize)]
struct FetchCortexResponse {
    status: u16,
    body: String,
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_status_code(headers: &str) -> Result<u16, String> {
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| "Missing HTTP status line".to_string())?;
    let code = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("Invalid HTTP status line: {status_line}"))?;
    code.parse::<u16>()
        .map_err(|e| format!("Invalid HTTP status code '{code}': {e}"))
}

fn decode_chunked_bytes(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut remaining = body;

    loop {
        let line_end = find_bytes(remaining, b"\r\n").ok_or_else(|| {
            "Invalid chunked encoding: missing chunk size line ending".to_string()
        })?;
        let size_line = std::str::from_utf8(&remaining[..line_end])
            .map_err(|e| format!("Invalid chunk size line UTF-8: {e}"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|e| format!("Invalid chunk size '{size_hex}': {e}"))?;

        let data_start = line_end + 2;
        if data_start > remaining.len() {
            return Err("Invalid chunked encoding: malformed chunk header".to_string());
        }
        remaining = &remaining[data_start..];

        if size == 0 {
            break;
        }

        if remaining.len() < size + 2 {
            return Err("Invalid chunked encoding: chunk truncated".to_string());
        }

        result.extend_from_slice(&remaining[..size]);
        remaining = &remaining[size..];

        if !remaining.starts_with(b"\r\n") {
            return Err("Invalid chunked encoding: missing CRLF after chunk".to_string());
        }
        remaining = &remaining[2..];
    }

    Ok(result)
}

// ─── MCP Auto-Registration ──────────────────────────────────────────────────

#[derive(Serialize)]
struct EditorDetection {
    name: String,
    detected: bool,
    registered: bool,
    message: String,
}

fn cortex_exe_path() -> Option<PathBuf> {
    find_cortex_binary()
}

fn register_cursor_mcp(cortex_exe: &str) -> Result<EditorDetection, String> {
    let home = cortex_home().map_err(|e| e.to_string())?;
    let cursor_dir = home.join(".cursor");
    if !cursor_dir.exists() {
        return Ok(EditorDetection {
            name: "Cursor".into(),
            detected: false,
            registered: false,
            message: "Cursor not detected (~/.cursor/ not found)".into(),
        });
    }

    let mcp_path = cursor_dir.join("mcp.json");
    let mut config: serde_json::Value = if mcp_path.exists() {
        let content = fs::read_to_string(&mcp_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let servers = config
        .as_object_mut()
        .ok_or("Invalid mcp.json format")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if servers.get("cortex").is_some() {
        return Ok(EditorDetection {
            name: "Cursor".into(),
            detected: true,
            registered: true,
            message: "Already registered".into(),
        });
    }

    servers
        .as_object_mut()
        .ok_or("Invalid mcpServers format")?
        .insert(
            "cortex".into(),
            serde_json::json!({
              "command": cortex_exe,
              "args": ["mcp"]
            }),
        );

    let out = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&mcp_path, out).map_err(|e| e.to_string())?;

    Ok(EditorDetection {
        name: "Cursor".into(),
        detected: true,
        registered: true,
        message: format!("Registered in {}", mcp_path.display()),
    })
}

fn register_claude_code_mcp(cortex_exe: &str) -> Result<EditorDetection, String> {
    let home = cortex_home().map_err(|e| e.to_string())?;
    let claude_dir = home.join(".claude");
    if !claude_dir.exists() {
        return Ok(EditorDetection {
            name: "Claude Code".into(),
            detected: false,
            registered: false,
            message: "Claude Code not detected (~/.claude/ not found)".into(),
        });
    }

    let settings_path = claude_dir.join("settings.json");
    let mut config: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let servers = config
        .as_object_mut()
        .ok_or("Invalid settings.json format")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    if servers.get("cortex").is_some() {
        return Ok(EditorDetection {
            name: "Claude Code".into(),
            detected: true,
            registered: true,
            message: "Already registered".into(),
        });
    }

    servers
        .as_object_mut()
        .ok_or("Invalid mcpServers format")?
        .insert(
            "cortex".into(),
            serde_json::json!({
              "command": cortex_exe,
              "args": ["mcp"]
            }),
        );

    let out = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&settings_path, out).map_err(|e| e.to_string())?;

    Ok(EditorDetection {
        name: "Claude Code".into(),
        detected: true,
        registered: true,
        message: format!("Registered in {}", settings_path.display()),
    })
}

#[tauri::command]
fn setup_editors() -> Result<Vec<EditorDetection>, String> {
    let cortex_exe = cortex_exe_path().ok_or(
    "Could not find cortex binary in sidecar directory, ~/.cortex/bin/, or ~/cortex/daemon-rs/target/release/",
  )?;
    let exe_str = cortex_exe.to_string_lossy().to_string();

    let mut results = vec![];
    match register_claude_code_mcp(&exe_str) {
        Ok(r) => results.push(r),
        Err(e) => results.push(EditorDetection {
            name: "Claude Code".into(),
            detected: true,
            registered: false,
            message: format!("Registration failed: {e}"),
        }),
    }
    match register_cursor_mcp(&exe_str) {
        Ok(r) => results.push(r),
        Err(e) => results.push(EditorDetection {
            name: "Cursor".into(),
            detected: true,
            registered: false,
            message: format!("Registration failed: {e}"),
        }),
    }
    Ok(results)
}

#[tauri::command]
fn detect_editors() -> Result<Vec<EditorDetection>, String> {
    let home = cortex_home()?;
    let has_exe = cortex_exe_path().is_some();
    let mut results = vec![];

    let claude_detected = home.join(".claude").exists();
    results.push(EditorDetection {
        name: "Claude Code".into(),
        detected: claude_detected,
        registered: false,
        message: if claude_detected {
            "Detected".into()
        } else {
            "Not installed".into()
        },
    });

    let cursor_detected = home.join(".cursor").exists();
    results.push(EditorDetection {
        name: "Cursor".into(),
        detected: cursor_detected,
        registered: false,
        message: if cursor_detected {
            "Detected".into()
        } else {
            "Not installed".into()
        },
    });

    if !has_exe {
        for r in &mut results {
            r.message = "cortex.exe not found -- build daemon first".into();
        }
    }

    Ok(results)
}

fn main() {
    let exe_path = find_cortex_binary();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DaemonState::new(exe_path))
        .manage(LifecycleState::default())
        .setup(|app| {
            setup_tray(app)?;

            let daemon_state = app.handle().state::<DaemonState>();
            let _ = daemon_state.start();

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.app_handle().state::<LifecycleState>();
                if !lifecycle.is_quit_requested() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            start_daemon,
            stop_daemon,
            quit_app,
            hide_to_tray,
            read_auth_token,
            fetch_cortex,
            post_cortex,
            setup_editors,
            detect_editors
        ])
        .build(tauri::generate_context!())
        .expect("error while building cortex control center");

    app.run(|app_handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            let lifecycle = app_handle.state::<LifecycleState>();
            if !lifecycle.is_quit_requested() {
                api.prevent_exit();
                hide_main_window(app_handle);
            } else {
                shutdown_daemon(app_handle);
            }
        }
        tauri::RunEvent::Exit => {
            shutdown_daemon(app_handle);
        }
        _ => {}
    });
}
