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
