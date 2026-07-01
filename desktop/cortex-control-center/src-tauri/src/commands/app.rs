use crate::daemon::state::LifecycleState;
use crate::tray::hide_main_window;
use std::env;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) -> Result<(), String> {
    let lifecycle = app.state::<LifecycleState>();
    lifecycle.request_quit();
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn hide_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    hide_main_window(&app);
    Ok(())
}

#[tauri::command]
pub fn write_dev_verification_report(content: String) -> Result<String, String> {
    if !cfg!(debug_assertions) {
        return Err("Dev verification reporting is only available in debug builds.".to_string());
    }

    let report_path = env::var("CORTEX_DEV_VERIFY_REPORT_PATH")
        .map(PathBuf::from)
        .map_err(|_| "CORTEX_DEV_VERIFY_REPORT_PATH is not configured.".to_string())?;

    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    fs::write(&report_path, content)
        .map_err(|err| format!("Failed to write {}: {err}", report_path.display()))?;

    Ok(report_path.display().to_string())
}
