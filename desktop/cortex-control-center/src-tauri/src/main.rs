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
use daemon::paths::find_cortex_binary;
use daemon::supervisor::{bootstrap_daemon_on_startup, run_supervisor_loop};
use daemon::{join_supervisor, request_supervisor_stop, shutdown_daemon, AppInstanceGuard, DaemonState, LifecycleState, SupervisorControl};
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
        .manage(SupervisorControl::new())
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

            let thread = std::thread::Builder::new()
                .name("cortex-daemon-supervisor".to_string())
                .spawn(move || {
                    run_supervisor_loop(&supervisor_handle);
                })
                .map_err(|err| std::io::Error::new(err.kind(), format!("failed to spawn cortex daemon supervisor thread: {err}")))?;
            app.state::<SupervisorControl>().attach(thread);

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
                request_supervisor_stop(app_handle);
                // Pause respawn, then join, then kill. Kill-then-join left a
                // window where an in-flight tick could spawn a new child after
                // abort and before the supervisor thread saw the stop flag;
                // WAL TRUNCATE then raced that writer.
                app_handle.state::<DaemonState>().pause_supervisor();
                join_supervisor(app_handle);
                shutdown_daemon(app_handle);
            }
        }
        tauri::RunEvent::Exit => {
            request_supervisor_stop(app_handle);
            app_handle.state::<DaemonState>().pause_supervisor();
            join_supervisor(app_handle);
            shutdown_daemon(app_handle);
        }
        _ => {}
    });
}
