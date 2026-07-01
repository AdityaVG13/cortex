use std::process::Command;

#[cfg(windows)]
use crate::constants::CREATE_NO_WINDOW_FLAG;

#[cfg(windows)]
pub(crate) fn apply_hidden_process_flags(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW_FLAG);
}

#[cfg(not(windows))]
pub(crate) fn apply_hidden_process_flags(_command: &mut Command) {}

pub(crate) fn apply_hidden_daemon_process_flags(command: &mut Command) {
    apply_hidden_process_flags(command);
}
