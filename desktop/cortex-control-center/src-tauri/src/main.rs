// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex Control Center.
//
// Cortex Control Center is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sidecar;

use rusqlite::Connection;
use serde::Serialize;
use sidecar::SidecarDaemon;
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
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
  Ok(cortex_home()?.join("cortex").join("cortex.db"))
}

fn is_cortex_reachable() -> bool {
  let addr = SocketAddr::from(([127, 0, 0, 1], 7437));
  TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

fn find_cortex_binary() -> Option<PathBuf> {
  if let Ok(exe) = env::current_exe() {
    if let Some(dir) = exe.parent() {
      let sidecar = dir.join("cortex.exe");
      if sidecar.exists() {
        return Some(sidecar);
      }
    }
  }

  if let Ok(home) = cortex_home() {
    let dev_path = home
      .join("cortex")
      .join("daemon-rs")
      .join("target")
      .join("release")
      .join("cortex.exe");
    if dev_path.exists() {
      return Some(dev_path);
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

  let conn = Connection::open(&db_path)
    .map_err(|err| format!("Failed to open DB for shutdown flush {}: {err}", db_path.display()))?;
  conn
    .execute_batch(
      r#"
    PRAGMA journal_mode = WAL;
    PRAGMA wal_checkpoint(TRUNCATE);
    PRAGMA optimize;
    "#,
    )
    .map_err(|err| format!("Failed to flush WAL on shutdown {}: {err}", db_path.display()))?;
  conn
    .close()
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
  let _ = daemon_state.stop();
  let _ = flush_cortex_db_on_shutdown();
  let lifecycle = app.state::<LifecycleState>();
  lifecycle.request_quit();
  app.exit(0);
  Ok(())
}

#[tauri::command]
fn daemon_status(state: State<DaemonState>) -> Result<DaemonCommandResult, String> {
  let (running, pid) = state.status()?;
  let reachable = is_cortex_reachable();
  let message = if running && reachable {
    format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
  } else if running {
    format!(
      "Cortex daemon running (pid {}) but not reachable on :7437 yet.",
      pid.unwrap_or_default()
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
fn start_daemon(state: State<DaemonState>) -> Result<DaemonCommandResult, String> {
  let (running, pid) = state.start()?;
  let reachable = is_cortex_reachable();
  let message = if reachable {
    format!("Cortex daemon running (pid {}).", pid.unwrap_or_default())
  } else {
    format!(
      "Cortex daemon started (pid {}) but :7437 is not reachable yet.",
      pid.unwrap_or_default()
    )
  };

  Ok(DaemonCommandResult {
    running,
    reachable,
    pid,
    message,
  })
}

#[tauri::command]
fn stop_daemon(state: State<DaemonState>) -> Result<DaemonCommandResult, String> {
  let (was_running, _) = state.status()?;
  let _ = state.stop();
  let reachable = is_cortex_reachable();
  let message = if was_running {
    "Stopped Cortex daemon.".to_string()
  } else if reachable {
    "Sidecar not running (external daemon is still reachable).".to_string()
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

// ─── MCP Auto-Registration ──────────────────────────────────────────────────

#[derive(Serialize)]
struct EditorDetection {
  name: String,
  detected: bool,
  registered: bool,
  message: String,
}

fn cortex_exe_path() -> Option<PathBuf> {
  let home = cortex_home().ok()?;
  let path = home
    .join("cortex")
    .join("daemon-rs")
    .join("target")
    .join("release")
    .join("cortex.exe");
  if path.exists() {
    Some(path)
  } else {
    None
  }
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
  let cortex_exe = cortex_exe_path()
    .ok_or("Could not find cortex.exe at ~/cortex/daemon-rs/target/release/")?;
  let exe_str = cortex_exe.to_string_lossy().replace('/', "\\");

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
      read_auth_token,
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

