use crate::constants::*;
use crate::daemon::state::LifecycleState;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, Runtime};

pub fn show_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn hide_main_window<R: Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = window.minimize();
        let _ = window.hide();
    }
}

pub fn hide_to_tray_on_close() -> bool {
    true
}

pub fn request_app_quit<R: Runtime>(app: &tauri::AppHandle<R>) {
    let lifecycle = app.state::<LifecycleState>();
    lifecycle.request_quit();
    app.exit(0);
}
pub fn setup_tray<R: Runtime>(app: &tauri::App<R>) -> tauri::Result<()> {
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

#[cfg(test)]
mod tests {
    use crate::daemon::state::LifecycleState;

    use super::hide_to_tray_on_close;

    #[test]
    fn close_button_policy_hides_to_tray_until_explicit_quit() {
        let lifecycle = LifecycleState::default();

        assert!(hide_to_tray_on_close());
        assert!(!lifecycle.is_quit_requested());

        lifecycle.request_quit();
        assert!(lifecycle.is_quit_requested());
    }
}
