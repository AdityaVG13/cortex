// SPDX-License-Identifier: MIT
//! Shared daemon lifecycle utilities: health checks, spawn, respawn.
//!
//! Used by both the CLI (`ensure_daemon`) and the MCP proxy (auto-respawn).

use std::process::{Command, Stdio};
use std::time::Duration;

use crate::auth::CortexPaths;

const RESPAWN_HEALTH_TIMEOUT_SECS: u64 = 90;

/// Check if the daemon responds to /health within 2s.
pub async fn daemon_healthy(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let response = match client.get(format!("http://127.0.0.1:{port}/health")).send().await {
        Ok(response) => response,
        Err(_) => return false,
    };

    let status = response.status().as_u16();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return false,
    };

    is_cortex_health_payload(status, &body)
}

fn is_cortex_health_payload(status: u16, body: &str) -> bool {
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

/// Poll /health until success or timeout.
pub async fn wait_for_health(port: u16, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        if daemon_healthy(port).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// Spawn the daemon as a detached background process.
pub fn spawn_daemon(paths: &CortexPaths) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current exe: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("serve")
        .env("CORTEX_HOME", &paths.home)
        .env("CORTEX_DB", &paths.db)
        .env("CORTEX_PORT", paths.port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Start the child in a new session so it survives the parent CLI process.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }

    cmd.spawn().map_err(|e| format!("spawn daemon: {e}"))?;
    Ok(())
}

/// Attempt to respawn the daemon and wait for it to become healthy.
/// Returns true if the daemon is healthy after respawn.
pub async fn try_respawn(paths: &CortexPaths) -> bool {
    eprintln!(
        "[cortex-lifecycle] Daemon appears dead on port {}; attempting respawn",
        paths.port
    );

    if let Err(e) = spawn_daemon(paths) {
        eprintln!("[cortex-lifecycle] Respawn failed: {e}");
        return false;
    }

    let healthy =
        wait_for_health(paths.port, Duration::from_secs(RESPAWN_HEALTH_TIMEOUT_SECS)).await;
    if healthy {
        eprintln!(
            "[cortex-lifecycle] Daemon respawned successfully on port {}",
            paths.port
        );
    } else {
        eprintln!(
            "[cortex-lifecycle] Daemon did not become healthy within {}s after respawn",
            RESPAWN_HEALTH_TIMEOUT_SECS
        );
    }
    healthy
}

#[cfg(test)]
mod tests {
    use super::is_cortex_health_payload;

    #[test]
    fn cortex_health_payload_accepts_expected_shapes() {
        assert!(is_cortex_health_payload(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#
        ));
        assert!(is_cortex_health_payload(
            200,
            r#"{"status":"degraded","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#
        ));
    }

    #[test]
    fn cortex_health_payload_rejects_non_cortex_bodies() {
        assert!(!is_cortex_health_payload(200, r#"{"status":"ok"}"#));
        assert!(!is_cortex_health_payload(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0"}}"#
        ));
        assert!(!is_cortex_health_payload(200, "<html>ok</html>"));
        assert!(!is_cortex_health_payload(500, r#"{"status":"ok","runtime":{}}"#));
    }
}
