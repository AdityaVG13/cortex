// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod budget;
mod commands;
mod constants;
mod cortex_http;
mod daemon;
mod editor;
mod tray;

use commands::{
    daemon_status, detect_editors, fetch_cortex, hide_to_tray, post_cortex, quit_app, read_auth_token, read_budget_config, save_budget_config, setup_editors,
    start_daemon, stop_daemon, write_dev_verification_report,
};
use constants::SUPERVISOR_TICK_MS;
use daemon::paths::find_cortex_binary;
use daemon::supervisor::{bootstrap_daemon_on_startup, supervisor_tick};
use daemon::{shutdown_daemon, AppInstanceGuard, DaemonState, LifecycleState};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tauri::Manager;
use tray::{hide_main_window, hide_to_tray_on_close, setup_tray};

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
            if hide_to_tray_on_close() {
                setup_tray(app)?;
            }
            let bootstrap_handle = app.handle().clone();
            let supervisor_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    bootstrap_daemon_on_startup(&bootstrap_handle);
                })
                .await;
            });

            // Watchdog: re-spawn the daemon if it disappears for any reason
            // (panic, OOM, manual kill, crash). The daemon is the user's
            // memory store — it must stay up unless they explicitly stop it.
            // Runs on a plain OS thread so it survives any Tauri runtime hiccup.
            std::thread::Builder::new()
                .name("cortex-daemon-supervisor".to_string())
                .spawn(move || {
                    let consecutive_failures = AtomicU32::new(0);
                    loop {
                        std::thread::sleep(Duration::from_millis(SUPERVISOR_TICK_MS));
                        supervisor_tick(&supervisor_handle, &consecutive_failures);
                    }
                })
                .map_err(|err| std::io::Error::new(err.kind(), format!("failed to spawn cortex daemon supervisor thread: {err}")))?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let lifecycle = window.app_handle().state::<LifecycleState>();
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
            read_budget_config,
            save_budget_config,
            fetch_cortex,
            post_cortex,
            write_dev_verification_report,
            setup_editors,
            detect_editors
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Failed to build Cortex Control Center: {err}");
            return;
        }
    };

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
