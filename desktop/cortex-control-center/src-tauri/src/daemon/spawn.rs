use crate::constants::*;
use crate::cortex_http::readiness::{is_cortex_reachable_with_port, probe_cortex_reachability_with_port, wait_for_reachability_blocking, CortexReachabilityProbe};
use crate::daemon::paths::find_cortex_binary;
use crate::daemon::process::apply_hidden_process_flags;
use crate::daemon::state::DaemonState;
use std::time::Duration;
use std::process::Command;

fn command_output_summary(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "<no output>".to_string(),
    }
}

pub fn try_service_ensure(port: u16) -> Result<bool, String> {
    if !cfg!(windows) {
        return Ok(false);
    }

    let Some(cortex_bin) = find_cortex_binary() else {
        return Ok(false);
    };

    let mut command = Command::new(&cortex_bin);
    command.args(["service", "ensure"]);
    apply_hidden_process_flags(&mut command);
    let output = command.output().map_err(|err| {
        format!(
            "Failed to run `{}` service ensure: {err}",
            cortex_bin.display()
        )
    })?;

    if !output.status.success() {
        return Err(format!(
            "`cortex service ensure` failed: {}",
            command_output_summary(&output)
        ));
    }

    if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) {
        return Ok(true);
    }

    Ok(wait_for_reachability_blocking(
        port,
        true,
        Duration::from_millis(SERVICE_ENSURE_WAIT_MS),
    ))
}

pub fn try_local_app_managed_ensure(
    state: &DaemonState,
    port: u16,
) -> Result<CortexReachabilityProbe, String> {
    state.ensure_local_daemon()?;
    if wait_for_reachability_blocking(
        port,
        true,
        Duration::from_millis(LOCAL_DAEMON_START_WAIT_MS),
    ) {
        return Ok(probe_cortex_reachability_with_port(
            port,
            DAEMON_REACHABILITY_TIMEOUT_MS,
        ));
    }

    // Re-probe once after the initial wait before declaring "still starting".
    // A surviving child process alone is not enough signal; it can be stale.
    let post_wait_probe = probe_cortex_reachability_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS);
    if local_probe_allows_starting_retry(&post_wait_probe) {
        return Ok(post_wait_probe);
    }

    let (_, pid) = state.status()?;
    Err(local_app_managed_start_timeout_message(state, pid, port))
}

pub fn local_probe_allows_starting_retry(probe: &CortexReachabilityProbe) -> bool {
    probe.reachable || probe.starting
}

pub fn local_app_managed_start_timeout_message(
    state: &DaemonState,
    pid: Option<u32>,
    port: u16,
) -> String {
    let base = if let Some(pid) = pid {
        format!("App-managed daemon spawned (pid {pid}) but never became reachable on :{port}.")
    } else {
        format!("App-managed daemon spawn did not produce a live daemon on :{port}.")
    };

    match state.stop() {
        Ok(()) => format!("{base} Control Center cleared the stale app-managed startup state."),
        Err(err) => format!(
            "{base} Control Center could not clear the stale app-managed startup state: {err}"
        ),
    }
}
