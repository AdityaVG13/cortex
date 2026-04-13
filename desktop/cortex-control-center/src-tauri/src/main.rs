// SPDX-License-Identifier: MIT
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sidecar;

use fs2::FileExt;
use rusqlite::Connection;
use serde::Serialize;
use sidecar::SidecarDaemon;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, Runtime, State};

const MAIN_WINDOW_LABEL: &str = "main";
const TRAY_ID: &str = "cortex-tray";
const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_HIDE_ID: &str = "tray_hide";
const TRAY_QUIT_ID: &str = "tray_quit";
const DEV_DAEMON_TARGET_DIR: &str = "target-control-center-dev";
const RELEASE_DAEMON_TARGET_DIR: &str = "target-control-center-release";
const DEFAULT_DAEMON_PORT: u16 = 7437;
const DAEMON_REACHABILITY_TIMEOUT_MS: u64 = 400;
const DAEMON_CONNECT_TIMEOUT_MS: u64 = 1_200;
const DAEMON_READ_TIMEOUT_MS: u64 = 10_000;
const DAEMON_WRITE_TIMEOUT_MS: u64 = 3_000;
const DAEMON_START_WAIT_MS: u64 = 3_000;
const DAEMON_STOP_WAIT_MS: u64 = 3_000;
const DAEMON_WAIT_POLL_MS: u64 = 200;
const AUTH_TOKEN_WAIT_MS: u64 = 1_500;
const AUTH_TOKEN_POLL_MS: u64 = 100;
const CONTROL_CENTER_LOCK_FILE: &str = "control-center.lock";

struct DaemonState {
    daemon: Mutex<SidecarDaemon>,
}

impl DaemonState {
    fn new(exe_path: Option<PathBuf>) -> Self {
        let runtime_copy_dir = if cfg!(debug_assertions) {
            default_cortex_dir()
                .ok()
                .map(|dir| runtime_copy_dir_for_session(&dir))
        } else {
            None
        };
        let daemon = match exe_path {
            Some(path) => SidecarDaemon::with_exe_path(path, runtime_copy_dir),
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

struct AppInstanceGuard {
    lock_file: File,
}

#[derive(Clone, Copy)]
struct RequestTimeouts {
    connect: Duration,
    read: Duration,
    write: Duration,
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

impl AppInstanceGuard {
    fn acquire() -> Result<Option<Self>, String> {
        let lock_path = control_center_lock_path()?;
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
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
    Ok(default_cortex_dir()?
        .join("runtime")
        .join(CONTROL_CENTER_LOCK_FILE))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DaemonCommandResult {
    running: bool,
    reachable: bool,
    managed: bool,
    auth_token_ready: bool,
    pid: Option<u32>,
    message: String,
}

fn cortex_home() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "Could not resolve USERPROFILE/HOME".to_string())
}

#[derive(Clone, Debug, Default)]
struct ResolvedCortexPaths {
    token: Option<PathBuf>,
    db: Option<PathBuf>,
    port: Option<u16>,
}

fn default_cortex_dir() -> Result<PathBuf, String> {
    Ok(cortex_home()?.join(".cortex"))
}

fn runtime_copy_dir_for_session(cortex_dir: &Path) -> PathBuf {
    cortex_dir
        .join("runtime")
        .join("control-center-dev")
        .join(format!("session-{}", std::process::id()))
}

fn token_path() -> Result<PathBuf, String> {
    resolved_cortex_paths()
        .token
        .ok_or_else(|| "Could not resolve Cortex token path".to_string())
}

fn cortex_db_path() -> Result<PathBuf, String> {
    resolved_cortex_paths()
        .db
        .ok_or_else(|| "Could not resolve Cortex database path".to_string())
}

fn daemon_port() -> u16 {
    resolve_daemon_port()
}

fn is_cortex_reachable_with_port(port: u16, timeout_ms: u64) -> bool {
    let response = send_cortex_request_with_port(
        port,
        "GET",
        "/health",
        "",
        None,
        RequestTimeouts {
            connect: Duration::from_millis(timeout_ms),
            read: Duration::from_millis(timeout_ms),
            write: Duration::from_millis(timeout_ms),
        },
    );

    matches!(
        response,
        Ok(resp) if is_cortex_health_response(resp.status, &resp.body)
    )
}

async fn wait_for_reachability(port: u16, target: bool, timeout: Duration) -> bool {
    tauri::async_runtime::spawn_blocking(move || {
        wait_for_reachability_blocking(port, target, timeout)
    })
    .await
    .unwrap_or(false)
}

fn wait_for_reachability_blocking(port: u16, target: bool, timeout: Duration) -> bool {
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
}

fn read_auth_token_once() -> Result<String, String> {
    let path = token_path()?;
    let token = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read token at {}: {err}", path.display()))?;
    Ok(token.trim().to_string())
}

fn auth_token_ready() -> bool {
    matches!(read_auth_token_once(), Ok(token) if !token.is_empty())
}

fn read_auth_token_with_retry_blocking(timeout: Duration) -> Result<String, String> {
    let path = token_path()?;
    if !is_cortex_reachable_with_port(daemon_port(), DAEMON_REACHABILITY_TIMEOUT_MS) {
        return read_auth_token_once();
    }

    let started = std::time::Instant::now();
    let mut last_error = format!("Auth token not ready at {}", path.display());

    loop {
        match fs::read_to_string(&path) {
            Ok(token) => {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
                last_error = format!("Auth token file is empty at {}", path.display());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                last_error = format!("Auth token file not found at {}", path.display());
            }
            Err(err) => {
                last_error = format!("Failed to read token at {}: {err}", path.display());
            }
        }

        if started.elapsed() >= timeout {
            return Err(last_error);
        }

        std::thread::sleep(Duration::from_millis(AUTH_TOKEN_POLL_MS));
    }
}

async fn read_auth_token_with_retry() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        read_auth_token_with_retry_blocking(Duration::from_millis(AUTH_TOKEN_WAIT_MS))
    })
    .await
    .map_err(|err| format!("Auth token wait task failed: {err}"))?
}

fn describe_daemon_state(
    managed: bool,
    reachable: bool,
    auth_token_ready: bool,
    pid: Option<u32>,
    port: u16,
) -> String {
    if managed && reachable && auth_token_ready {
        format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
    } else if managed && reachable {
        format!(
            "Cortex daemon running (pid {}) and reachable, waiting for auth token.",
            pid.unwrap_or_default()
        )
    } else if managed {
        format!(
            "Cortex daemon running (pid {}) but not reachable on :{} yet.",
            pid.unwrap_or_default(),
            port
        )
    } else if reachable && auth_token_ready {
        "Cortex daemon reachable (external process).".to_string()
    } else if reachable {
        "Cortex daemon reachable (external process), waiting for auth token.".to_string()
    } else {
        "Cortex daemon is offline.".to_string()
    }
}

fn cortex_binary_name() -> &'static str {
    if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    }
}

fn workspace_binary_candidates(home: &Path, prefer_debug: bool) -> Vec<PathBuf> {
    let daemon_root = home.join("cortex").join("daemon-rs");
    let release_path = daemon_root
        .join("target")
        .join("release")
        .join(cortex_binary_name());
    let isolated_release_path = daemon_root
        .join(RELEASE_DAEMON_TARGET_DIR)
        .join("release")
        .join(cortex_binary_name());
    let debug_path = daemon_root
        .join("target")
        .join("debug")
        .join(cortex_binary_name());
    let isolated_debug_path = daemon_root
        .join(DEV_DAEMON_TARGET_DIR)
        .join("debug")
        .join(cortex_binary_name());

    if prefer_debug {
        vec![
            isolated_debug_path,
            debug_path,
            isolated_release_path,
            release_path,
        ]
    } else {
        vec![
            isolated_release_path,
            release_path,
            isolated_debug_path,
            debug_path,
        ]
    }
}

fn resolve_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let locator = if cfg!(windows) { "where.exe" } else { "which" };
    let output = Command::new(locator).arg(binary_name).output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
}

fn parse_paths_json(output: &[u8]) -> Result<ResolvedCortexPaths, String> {
    let json: serde_json::Value = serde_json::from_slice(output)
        .map_err(|err| format!("Invalid JSON from `cortex paths --json`: {err}"))?;
    let port = json
        .get("port")
        .and_then(|value| value.as_u64())
        .map(|value| {
            u16::try_from(value).map_err(|err| format!("Port value out of range ({value}): {err}"))
        })
        .transpose()?;

    Ok(ResolvedCortexPaths {
        token: json
            .get("token")
            .and_then(|value| value.as_str())
            .map(PathBuf::from),
        db: json
            .get("db")
            .and_then(|value| value.as_str())
            .map(PathBuf::from),
        port,
    })
}

fn resolve_paths_with_binary(
    binary: impl AsRef<std::ffi::OsStr>,
) -> Result<Option<ResolvedCortexPaths>, String> {
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
    parse_paths_json(&output.stdout).map(Some)
}

fn fallback_cortex_paths() -> ResolvedCortexPaths {
    let cortex_dir = env::var("CORTEX_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| default_cortex_dir().ok());

    let port = match env::var("CORTEX_PORT") {
        Ok(value) => match value.parse::<u16>() {
            Ok(port) => Some(port),
            Err(err) => {
                eprintln!("[cortex-control-center] Invalid CORTEX_PORT '{value}': {err}");
                Some(DEFAULT_DAEMON_PORT)
            }
        },
        Err(env::VarError::NotPresent) => Some(DEFAULT_DAEMON_PORT),
        Err(err) => {
            eprintln!("[cortex-control-center] Failed to read CORTEX_PORT: {err}");
            Some(DEFAULT_DAEMON_PORT)
        }
    };

    ResolvedCortexPaths {
        token: cortex_dir.as_ref().map(|dir| dir.join("cortex.token")),
        db: cortex_dir.as_ref().map(|dir| dir.join("cortex.db")),
        port,
    }
}

fn resolved_cortex_paths() -> ResolvedCortexPaths {
    if let Some(binary) = find_cortex_binary() {
        match resolve_paths_with_binary(&binary) {
            Ok(Some(paths)) => return paths,
            Ok(None) => {}
            Err(err) => eprintln!("[cortex-control-center] {err}"),
        }
    }

    if let Some(binary) = resolve_binary_on_path("cortex") {
        match resolve_paths_with_binary(&binary) {
            Ok(Some(paths)) => return paths,
            Ok(None) => {}
            Err(err) => eprintln!("[cortex-control-center] {err}"),
        }
    }

    fallback_cortex_paths()
}

fn resolve_daemon_port() -> u16 {
    resolved_cortex_paths().port.unwrap_or(DEFAULT_DAEMON_PORT)
}

fn newest_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;

    for candidate in candidates {
        let Ok(metadata) = fs::metadata(candidate) else {
            continue;
        };
        let modified = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        match &newest {
            Some((current, _)) if &modified <= current => {}
            _ => newest = Some((modified, candidate.clone())),
        }
    }

    newest.map(|(_, path)| path)
}

fn installed_plugin_binary_path(home: &Path) -> PathBuf {
    home.join(".cortex").join("bin").join(cortex_binary_name())
}

fn copy_if_changed(src: &Path, dest: &Path) -> Result<(), String> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => {
            existing != fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(format!("read {}: {err}", dest.display())),
    };

    if needs_copy {
        fs::copy(src, dest)
            .map_err(|e| format!("copy {} -> {}: {e}", src.display(), dest.display()))?;
    }

    Ok(())
}

fn ensure_editor_binary_path() -> Result<PathBuf, String> {
    let home = cortex_home()?;
    let source = find_cortex_binary().ok_or_else(|| {
        "Could not find cortex binary in sidecar directory, ~/.cortex/bin/, or ~/cortex/daemon-rs/{target-control-center-dev,target-control-center-release,target}/{debug,release}/".to_string()
    })?;
    let installed = installed_plugin_binary_path(&home);

    if source != installed {
        if let Some(parent) = installed.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        copy_if_changed(&source, &installed)?;
    }

    Ok(installed)
}

fn find_cortex_binary() -> Option<PathBuf> {
    let sidecar_candidate = env::current_exe().ok().and_then(|exe| {
        exe.parent()
            .map(|dir| dir.join(cortex_binary_name()))
            .filter(|path| path.exists())
    });

    if let Ok(home) = cortex_home() {
        let plugin_path = home.join(".cortex").join("bin").join(cortex_binary_name());
        let mut candidates = Vec::new();
        if cfg!(debug_assertions) {
            // In dev builds prefer workspace binaries so the app does not
            // silently launch an older installed or copied sidecar daemon.
            let workspace_candidates = workspace_binary_candidates(&home, true);
            if let Some(candidate) = newest_existing_path(&workspace_candidates) {
                return Some(candidate);
            }
            candidates.push(plugin_path);
            if let Some(sidecar) = sidecar_candidate.clone() {
                candidates.push(sidecar);
            }
        } else {
            if let Some(sidecar) = sidecar_candidate.clone() {
                candidates.push(sidecar);
            }
            candidates.push(plugin_path);
            candidates.extend(workspace_binary_candidates(&home, false));
        }

        for candidate in candidates {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    if let Some(sidecar) = sidecar_candidate {
        return Some(sidecar);
    }

    resolve_binary_on_path(cortex_binary_name())
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

fn hide_to_tray_on_close() -> bool {
    !cfg!(debug_assertions)
}

fn request_app_quit<R: Runtime>(app: &tauri::AppHandle<R>) {
    let lifecycle = app.state::<LifecycleState>();
    lifecycle.request_quit();
    app.exit(0);
}

fn shutdown_daemon<R: Runtime>(app: &tauri::AppHandle<R>) {
    let daemon_state = app.state::<DaemonState>();
    let port = daemon_port();
    let _ = daemon_state.stop();
    if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        let _ = send_http_shutdown();
        let _ =
            wait_for_reachability_blocking(port, false, Duration::from_millis(DAEMON_STOP_WAIT_MS));
    }
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
fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
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
    let (managed, pid) = state.status()?;
    let port = daemon_port();
    let reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    let auth_token_ready = reachable && auth_token_ready();
    let message = describe_daemon_state(managed, reachable, auth_token_ready, pid, port);

    Ok(DaemonCommandResult {
        running: managed || reachable,
        reachable,
        managed,
        auth_token_ready,
        pid,
        message,
    })
}

#[tauri::command]
async fn start_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (already_managed, current_pid) = state.status()?;
    if !already_managed && is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        let auth_token_ready = auth_token_ready();
        return Ok(DaemonCommandResult {
            running: true,
            reachable: true,
            managed: false,
            auth_token_ready,
            pid: None,
            message: describe_daemon_state(false, true, auth_token_ready, None, port),
        });
    }

    let (managed, pid) = if already_managed {
        (already_managed, current_pid)
    } else {
        state.start()?
    };
    let reachable = if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        true
    } else {
        wait_for_reachability(port, true, Duration::from_millis(DAEMON_START_WAIT_MS)).await
    };
    let auth_token_ready = reachable && auth_token_ready();
    let message = describe_daemon_state(managed, reachable, auth_token_ready, pid, port);

    Ok(DaemonCommandResult {
        running: managed,
        reachable,
        managed,
        auth_token_ready,
        pid,
        message,
    })
}

/// Send POST /shutdown to the daemon's HTTP endpoint (works for any daemon,
/// regardless of who spawned it). Returns Ok(()) on success or if connection
/// fails (daemon already gone).
fn send_http_shutdown() -> Result<(), String> {
    let token = read_auth_token_once().unwrap_or_default();
    let initial = send_cortex_request("POST", "/shutdown", &token, Some("{}"));
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
                ));
            }
        }
    }

    interpret_shutdown_response(initial)
}

fn interpret_shutdown_response(
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

fn extract_error_detail(body: &str) -> Option<String> {
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

#[tauri::command]
async fn stop_daemon(state: State<'_, DaemonState>) -> Result<DaemonCommandResult, String> {
    let port = daemon_port();
    let (was_running, _) = state.status()?;
    let managed_stop_error = state.stop().err();

    // If daemon is still reachable (started externally by Claude, CLI, etc.),
    // send a graceful HTTP shutdown signal
    let still_reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    let mut shutdown_error = None;
    if still_reachable {
        if let Err(err) = send_http_shutdown() {
            shutdown_error = Some(err);
        }
        let _ =
            wait_for_reachability(port, false, Duration::from_millis(DAEMON_STOP_WAIT_MS)).await;
    }

    let reachable = is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if reachable {
        if let Some(err) = managed_stop_error.or(shutdown_error) {
            return Err(err);
        }
    }

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
        managed: false,
        auth_token_ready: reachable && auth_token_ready(),
        pid: None,
        message,
    })
}

#[tauri::command]
async fn read_auth_token() -> Result<String, String> {
    read_auth_token_with_retry().await
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
    send_cortex_request_with_port(
        daemon_port(),
        method,
        path,
        auth_token,
        body,
        RequestTimeouts {
            connect: Duration::from_millis(DAEMON_CONNECT_TIMEOUT_MS),
            read: Duration::from_millis(DAEMON_READ_TIMEOUT_MS),
            write: Duration::from_millis(DAEMON_WRITE_TIMEOUT_MS),
        },
    )
}

fn send_cortex_request_with_port(
    port: u16,
    method: &str,
    path: &str,
    auth_token: &str,
    body: Option<&str>,
    timeouts: RequestTimeouts,
) -> Result<FetchCortexResponse, String> {
    use std::io::{Read, Write};

    if path.contains('\r') || path.contains('\n') {
        return Err("Invalid request path".to_string());
    }

    let mut stream =
        TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeouts.connect)
            .map_err(|e| format!("Cannot connect to daemon: {e}"))?;
    stream
        .set_read_timeout(Some(timeouts.read))
        .map_err(|e| format!("Cannot set read timeout: {e}"))?;
    stream
        .set_write_timeout(Some(timeouts.write))
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

fn is_cortex_health_response(status: u16, body: &str) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return false;
    };

    let health_status = json.get("status").and_then(|value| value.as_str());
    let runtime = json.get("runtime").and_then(|value| value.as_object());
    let stats = json.get("stats").and_then(|value| value.as_object());

    matches!(health_status, Some("ok" | "degraded")) && runtime.is_some() && stats.is_some()
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
#[serde(rename_all = "camelCase")]
struct EditorDetection {
    id: String,
    name: String,
    detected: bool,
    registered: bool,
    config_path: Option<String>,
    command_path: Option<String>,
    message: String,
}

#[derive(Clone, Copy)]
enum EditorConfigKind {
    Json,
    Toml,
}

#[derive(Clone)]
struct EditorTarget {
    id: &'static str,
    name: &'static str,
    agent_name: &'static str,
    config_kind: EditorConfigKind,
    config_path: PathBuf,
    fallback_config_paths: Vec<PathBuf>,
}

fn cortex_exe_path() -> Option<PathBuf> {
    find_cortex_binary()
}

fn editor_args(target: &EditorTarget) -> [&'static str; 3] {
    ["mcp", "--agent", target.agent_name]
}

fn editor_path_detected(path: &Path) -> bool {
    path.exists() || path.parent().map(|parent| parent.exists()).unwrap_or(false)
}

fn editor_config_path(target: &EditorTarget) -> PathBuf {
    if target.config_path.exists() {
        return target.config_path.clone();
    }
    for path in &target.fallback_config_paths {
        if path.exists() {
            return path.clone();
        }
    }
    target.config_path.clone()
}

fn cortex_mcp_registration(target: &EditorTarget, cortex_exe: &str) -> serde_json::Value {
    serde_json::json!({
      "command": cortex_exe,
      "args": editor_args(target)
    })
}
fn editor_targets(home: &Path) -> Vec<EditorTarget> {
    vec![
        EditorTarget {
            id: "claude-code",
            name: "Claude Code",
            agent_name: "claude",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".claude").join("settings.json"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "claude-desktop",
            name: "Claude Desktop",
            agent_name: "claude",
            config_kind: EditorConfigKind::Json,
            config_path: home
                .join("AppData")
                .join("Roaming")
                .join("Claude")
                .join("claude_desktop_config.json"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "cursor",
            name: "Cursor",
            agent_name: "cursor",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".cursor").join("mcp.json"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "codex",
            name: "Codex",
            agent_name: "codex",
            config_kind: EditorConfigKind::Toml,
            config_path: home.join(".codex").join("config.toml"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "gemini",
            name: "Gemini CLI",
            agent_name: "gemini",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".gemini").join("settings").join("mcp.json"),
            fallback_config_paths: vec![home.join(".gemini").join("settings.json")],
        },
        EditorTarget {
            id: "droid",
            name: "Droid",
            agent_name: "droid",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".factory").join("mcp.json"),
            fallback_config_paths: Vec::new(),
        },
    ]
}

fn editor_detected(target: &EditorTarget) -> bool {
    editor_path_detected(&target.config_path)
        || target
            .fallback_config_paths
            .iter()
            .any(|path| editor_path_detected(path))
}

fn editor_command_path(cortex_exe: Option<&str>) -> Option<String> {
    cortex_exe.map(|path| path.to_string())
}

fn editor_detection(
    target: &EditorTarget,
    detected: bool,
    registered: bool,
    cortex_exe: Option<&str>,
    message: String,
) -> EditorDetection {
    let config_path = editor_config_path(target);
    EditorDetection {
        id: target.id.into(),
        name: target.name.into(),
        detected,
        registered,
        config_path: Some(config_path.display().to_string()),
        command_path: editor_command_path(cortex_exe),
        message,
    }
}

fn json_registration_for(target: &EditorTarget, cortex_exe: &str) -> serde_json::Value {
    let mut registration = cortex_mcp_registration(target, cortex_exe);
    if let Some(object) = registration.as_object_mut() {
        match target.id {
            "gemini" => {
                object.insert("trust".into(), serde_json::Value::Bool(true));
            }
            "droid" => {
                object.insert("disabled".into(), serde_json::Value::Bool(false));
            }
            _ => {}
        }
    }
    registration
}

fn read_json_config(config_path: &Path) -> Result<serde_json::Value, String> {
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        Ok(serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({})))
    } else {
        Ok(serde_json::json!({}))
    }
}

fn read_toml_config(config_path: &Path) -> Result<toml::Value, String> {
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        Ok(toml::from_str(&content).unwrap_or_else(|_| toml::Value::Table(Default::default())))
    } else {
        Ok(toml::Value::Table(Default::default()))
    }
}

fn json_args_match(config: &serde_json::Value, expected_args: &[&str]) -> bool {
    config
        .get("args")
        .and_then(|value| value.as_array())
        .map(|args| {
            args.len() == expected_args.len()
                && args
                    .iter()
                    .zip(expected_args.iter())
                    .all(|(value, expected)| value.as_str() == Some(*expected))
        })
        .unwrap_or(false)
}

fn toml_args_match(config: &toml::Value, expected_args: &[&str]) -> bool {
    config
        .get("args")
        .and_then(|value| value.as_array())
        .map(|args| {
            args.len() == expected_args.len()
                && args
                    .iter()
                    .zip(expected_args.iter())
                    .all(|(value, expected)| value.as_str() == Some(*expected))
        })
        .unwrap_or(false)
}

fn is_editor_registered_at_path(
    target: &EditorTarget,
    cortex_exe: &str,
    config_path: &Path,
) -> Result<bool, String> {
    if !config_path.exists() {
        return Ok(false);
    }
    let expected_args = editor_args(target);

    match target.config_kind {
        EditorConfigKind::Json => {
            let config = read_json_config(config_path)?;
            Ok(config
                .get("mcpServers")
                .and_then(|value| value.get("cortex"))
                .map(|value| {
                    value
                        .get("command")
                        .and_then(|command| command.as_str())
                        .map(|command| command == cortex_exe)
                        .unwrap_or(false)
                        && json_args_match(value, &expected_args)
                })
                .unwrap_or(false))
        }
        EditorConfigKind::Toml => {
            let config = read_toml_config(config_path)?;
            Ok(config
                .get("mcp_servers")
                .and_then(|value| value.get("cortex"))
                .map(|value| {
                    value
                        .get("command")
                        .and_then(|command| command.as_str())
                        .map(|command| command == cortex_exe)
                        .unwrap_or(false)
                        && toml_args_match(value, &expected_args)
                })
                .unwrap_or(false))
        }
    }
}

fn is_editor_registered(target: &EditorTarget, cortex_exe: &str) -> Result<bool, String> {
    let config_path = editor_config_path(target);
    is_editor_registered_at_path(target, cortex_exe, &config_path)
}

fn register_json_editor(
    target: &EditorTarget,
    cortex_exe: &str,
) -> Result<EditorDetection, String> {
    let config_path = editor_config_path(target);
    if !editor_detected(target) {
        return Ok(editor_detection(
            target,
            false,
            false,
            Some(cortex_exe),
            format!("{} not detected ({})", target.name, config_path.display()),
        ));
    }

    let mut config = read_json_config(&config_path)?;
    let servers = config
        .as_object_mut()
        .ok_or_else(|| format!("Invalid JSON config format in {}", config_path.display()))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let action = if is_editor_registered_at_path(target, cortex_exe, &config_path)? {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };

    servers
        .as_object_mut()
        .ok_or_else(|| format!("Invalid mcpServers format in {}", config_path.display()))?
        .insert("cortex".into(), json_registration_for(target, cortex_exe));

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&config_path, out).map_err(|e| e.to_string())?;

    Ok(editor_detection(
        target,
        true,
        true,
        Some(cortex_exe),
        format!("{action} in {}", config_path.display()),
    ))
}

fn register_toml_editor(
    target: &EditorTarget,
    cortex_exe: &str,
) -> Result<EditorDetection, String> {
    let config_path = editor_config_path(target);
    if !editor_detected(target) {
        return Ok(editor_detection(
            target,
            false,
            false,
            Some(cortex_exe),
            format!("{} not detected ({})", target.name, config_path.display()),
        ));
    }

    let mut config = read_toml_config(&config_path)?;
    let root = config
        .as_table_mut()
        .ok_or_else(|| format!("Invalid TOML config format in {}", config_path.display()))?;
    let servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| format!("Invalid [mcp_servers] format in {}", config_path.display()))?;

    let action = if is_editor_registered_at_path(target, cortex_exe, &config_path)? {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };
    let args = editor_args(target);

    let mut server = toml::map::Map::new();
    server.insert(
        "command".into(),
        toml::Value::String(cortex_exe.to_string()),
    );
    server.insert(
        "args".into(),
        toml::Value::Array(
            args.into_iter()
                .map(|value| toml::Value::String(value.to_string()))
                .collect(),
        ),
    );
    servers.insert("cortex".into(), toml::Value::Table(server));

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&config_path, out).map_err(|e| e.to_string())?;

    Ok(editor_detection(
        target,
        true,
        true,
        Some(cortex_exe),
        format!("{action} in {}", config_path.display()),
    ))
}

fn register_editor(target: &EditorTarget, cortex_exe: &str) -> Result<EditorDetection, String> {
    match target.config_kind {
        EditorConfigKind::Json => register_json_editor(target, cortex_exe),
        EditorConfigKind::Toml => register_toml_editor(target, cortex_exe),
    }
}

#[tauri::command]
fn setup_editors(editor_ids: Option<Vec<String>>) -> Result<Vec<EditorDetection>, String> {
    let cortex_exe = ensure_editor_binary_path()?;
    let exe_str = cortex_exe.to_string_lossy().to_string();
    let home = cortex_home()?;
    let targets = editor_targets(&home);
    let requested_ids = editor_ids
        .unwrap_or_default()
        .into_iter()
        .collect::<HashSet<_>>();
    let use_selection = !requested_ids.is_empty();
    let mut results = Vec::new();

    for target in targets {
        let detected = editor_detected(&target);
        if use_selection && !requested_ids.contains(target.id) {
            continue;
        }
        if !use_selection && !detected {
            continue;
        }

        match register_editor(&target, &exe_str) {
            Ok(result) => results.push(result),
            Err(err) => results.push(editor_detection(
                &target,
                detected,
                false,
                Some(&exe_str),
                format!("Configuration failed: {err}"),
            )),
        }
    }

    Ok(results)
}

#[tauri::command]
fn detect_editors() -> Result<Vec<EditorDetection>, String> {
    let home = cortex_home()?;
    let cortex_exe = cortex_exe_path();
    let cortex_exe_string = cortex_exe
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let mut results = Vec::new();

    for target in editor_targets(&home) {
        let detected = editor_detected(&target);
        let registered = if let Some(ref exe) = cortex_exe_string {
            is_editor_registered(&target, exe).unwrap_or(false)
        } else {
            false
        };
        let message = if cortex_exe_string.is_none() {
            "cortex.exe not found -- build daemon first".into()
        } else if registered {
            format!("Configured in {}", target.config_path.display())
        } else if detected {
            format!("Detected at {}", target.config_path.display())
        } else {
            format!("Not detected ({})", target.config_path.display())
        };

        results.push(editor_detection(
            &target,
            detected,
            registered,
            cortex_exe_string.as_deref(),
            message,
        ));
    }

    Ok(results)
}

fn main() {
    let _instance_guard = match AppInstanceGuard::acquire() {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            eprintln!("Cortex Control Center is already running.");
            return;
        }
        Err(err) => {
            eprintln!("Failed to initialize Cortex Control Center: {err}");
            return;
        }
    };
    let exe_path = find_cortex_binary();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(DaemonState::new(exe_path))
        .manage(LifecycleState::default())
        .setup(|app| {
            setup_tray(app)?;

            let daemon_state = app.handle().state::<DaemonState>();
            if !is_cortex_reachable_with_port(daemon_port(), DAEMON_REACHABILITY_TIMEOUT_MS) {
                let _ = daemon_state.start();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.app_handle().state::<LifecycleState>();
                // In `tauri dev`, let the window close normally so the dev runner can
                // restart without force-killing the WebView2 host.
                if hide_to_tray_on_close() && !lifecycle.is_quit_requested() {
                    api.prevent_close();
                    hide_main_window(window.app_handle());
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
            if hide_to_tray_on_close() && !lifecycle.is_quit_requested() {
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

#[cfg(test)]
mod tests {
    use super::{
        default_cortex_dir, editor_args, editor_config_path, editor_targets, extract_error_detail,
        interpret_shutdown_response, is_cortex_health_response, runtime_copy_dir_for_session,
        workspace_binary_candidates, FetchCortexResponse,
    };
    use std::fs;
    use std::path::Path;

    #[test]
    fn workspace_binary_candidates_prefers_debug_for_dev_builds() {
        let candidates = workspace_binary_candidates(Path::new("C:/Users/aditya"), true);
        assert!(candidates[0]
            .to_string_lossy()
            .contains("target-control-center-dev\\debug"));
        assert!(candidates[1].to_string_lossy().contains("target\\debug"));
        assert!(candidates[2]
            .to_string_lossy()
            .contains("target-control-center-release\\release"));
        assert!(candidates[3].to_string_lossy().contains("target\\release"));
    }

    #[test]
    fn workspace_binary_candidates_prefers_release_for_packaged_builds() {
        let candidates = workspace_binary_candidates(Path::new("C:/Users/aditya"), false);
        assert!(candidates[0]
            .to_string_lossy()
            .contains("target-control-center-release\\release"));
        assert!(candidates[1].to_string_lossy().contains("target\\release"));
        assert!(candidates[2]
            .to_string_lossy()
            .contains("target-control-center-dev\\debug"));
        assert!(candidates[3].to_string_lossy().contains("target\\debug"));
    }

    #[test]
    fn runtime_copy_dir_is_scoped_to_the_control_center_session() {
        let cortex_dir = default_cortex_dir().expect("cortex dir");
        let path = runtime_copy_dir_for_session(&cortex_dir);
        let pid = std::process::id().to_string();

        assert!(path.starts_with(cortex_dir.join("runtime").join("control-center-dev")));
        assert!(path.to_string_lossy().contains(&format!("session-{pid}")));
    }

    #[test]
    fn interpret_shutdown_response_surfaces_auth_rejection() {
        let err = interpret_shutdown_response(Ok(FetchCortexResponse {
            status: 401,
            body: "{\"error\":\"Unauthorized\"}".to_string(),
        }))
        .unwrap_err();

        assert!(err.contains("Refresh the token"));
    }

    #[test]
    fn extract_error_detail_prefers_json_error_field() {
        let detail = extract_error_detail("{\"error\":\"Unauthorized\"}").unwrap();
        assert_eq!(detail, "Unauthorized");
    }

    #[test]
    fn cortex_health_probe_accepts_healthy_response_shape() {
        assert!(is_cortex_health_response(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#
        ));
        assert!(is_cortex_health_response(
            200,
            r#"{"status":"degraded","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#
        ));
    }

    #[test]
    fn cortex_health_probe_rejects_non_cortex_responses() {
        assert!(!is_cortex_health_response(200, "<html>ok</html>"));
        assert!(!is_cortex_health_response(200, r#"{"status":"ok"}"#));
        assert!(!is_cortex_health_response(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0"}}"#
        ));
        assert!(!is_cortex_health_response(
            503,
            r#"{"status":"ok","runtime":{}}"#
        ));
    }

    #[test]
    fn editor_registration_uses_explicit_agent_args() {
        let home = Path::new("C:/Users/aditya");
        let targets = editor_targets(home);
        let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
        let claude = targets
            .iter()
            .find(|target| target.id == "claude-code")
            .unwrap();

        assert_eq!(editor_args(cursor), ["mcp", "--agent", "cursor"]);
        assert_eq!(editor_args(claude), ["mcp", "--agent", "claude"]);
    }

    #[test]
    fn gemini_prefers_nested_mcp_config_when_present() {
        let temp_root = std::env::temp_dir().join(format!(
            "cortex_control_center_editor_test_{}",
            std::process::id()
        ));
        let gemini_nested = temp_root.join(".gemini").join("settings").join("mcp.json");
        let gemini_legacy = temp_root.join(".gemini").join("settings.json");
        fs::create_dir_all(gemini_nested.parent().unwrap()).expect("create gemini settings dir");
        fs::write(&gemini_nested, "{}").expect("write nested gemini config");
        fs::write(&gemini_legacy, "{}").expect("write legacy gemini config");

        let targets = editor_targets(&temp_root);
        let gemini = targets.iter().find(|target| target.id == "gemini").unwrap();

        assert_eq!(editor_config_path(gemini), gemini_nested);

        let _ = fs::remove_file(gemini_nested);
        let _ = fs::remove_file(gemini_legacy);
        let _ = fs::remove_dir_all(temp_root.join(".gemini"));
        let _ = fs::remove_dir(temp_root);
    }
}
