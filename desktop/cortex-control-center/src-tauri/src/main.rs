#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod embedded_daemon;

use embedded_daemon::{EmbeddedDaemon, EmbeddedDaemonStatus};
use serde::Serialize;
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

#[derive(Default)]
struct DaemonState {
  daemon: Mutex<EmbeddedDaemon>,
}

impl DaemonState {
  fn status(&self) -> Result<EmbeddedDaemonStatus, String> {
    let daemon = self
      .daemon
      .lock()
      .map_err(|_| "Failed to lock embedded daemon state".to_string())?;
    Ok(daemon.status())
  }

  fn start(&self) -> Result<EmbeddedDaemonStatus, String> {
    let mut daemon = self
      .daemon
      .lock()
      .map_err(|_| "Failed to lock embedded daemon state".to_string())?;
    daemon.start()
  }

  fn stop(&self) -> Result<EmbeddedDaemonStatus, String> {
    let mut daemon = self
      .daemon
      .lock()
      .map_err(|_| "Failed to lock embedded daemon state".to_string())?;
    daemon.stop()
  }
}

#[derive(Default)]
struct LifecycleState {
  explicit_quit: AtomicBool,
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

fn is_cortex_reachable() -> bool {
  let addr = SocketAddr::from(([127, 0, 0, 1], 7437));
  TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
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

fn shutdown_embedded_daemon<R: Runtime>(app: &tauri::AppHandle<R>) {
  let daemon_state = app.state::<DaemonState>();
  let _ = daemon_state.stop();
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
fn daemon_status(state: State<DaemonState>) -> Result<DaemonCommandResult, String> {
  let embedded = state.status()?;
  let running = embedded.running;
  let pid = embedded.pid;
  let reachable = is_cortex_reachable();
  let message = if running && reachable {
    format!(
      "Embedded Cortex daemon running (pid {}).",
      pid.unwrap_or_default()
    )
  } else if running {
    format!(
      "Embedded Cortex daemon running (pid {}) but not reachable on :7437 yet.",
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
  let embedded = state.start()?;
  let running = embedded.running;
  let pid = embedded.pid;
  let reachable = is_cortex_reachable();
  let message = if reachable {
    format!(
      "Embedded Cortex daemon running (pid {}).",
      pid.unwrap_or_default()
    )
  } else {
    format!(
      "Embedded Cortex daemon started (pid {}) but :7437 is not reachable yet.",
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
  let was_running = state.status()?.running;
  let _ = state.stop()?;
  let reachable = is_cortex_reachable();
  let message = if was_running {
    "Stopped embedded Cortex daemon.".to_string()
  } else if reachable {
    "Embedded daemon is not running (external daemon is still reachable).".to_string()
  } else {
    "Embedded daemon is already stopped.".to_string()
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

fn main() {
  let app = tauri::Builder::default()
    .manage(DaemonState::default())
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
      read_auth_token
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
        shutdown_embedded_daemon(app_handle);
      }
    }
    tauri::RunEvent::Exit => {
      shutdown_embedded_daemon(app_handle);
    }
    _ => {}
  });
}
