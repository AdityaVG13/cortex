// SPDX-License-Identifier: MIT
//! Shared daemon lifecycle utilities: health checks, spawn, respawn.
//!
//! Used by both the CLI (`ensure_daemon`) and the MCP proxy (auto-respawn).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::auth::CortexPaths;

const RESPAWN_HEALTH_TIMEOUT_SECS: u64 = 90;
const RESPAWN_COORDINATION_WAIT_SECS: u64 = 20;
const RUNTIME_COPY_STALE_SECS: u64 = 600;

/// Check if the daemon responds to /health within 2s.
pub async fn daemon_healthy(port: u16) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let response = match client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
    {
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
    let spawn_exe =
        prepare_spawn_executable(paths, &exe).map_err(|e| format!("prepare spawn copy: {e}"))?;
    let mut cmd = Command::new(spawn_exe);
    cmd.arg("serve")
        .env("CORTEX_HOME", &paths.home)
        .env("CORTEX_DB", &paths.db)
        .env("CORTEX_PORT", paths.port.to_string())
        .env("CORTEX_WAIT_FOR_DAEMON_LOCK", "1")
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

fn prepare_spawn_executable(paths: &CortexPaths, source: &Path) -> std::io::Result<PathBuf> {
    if !is_workspace_daemon_binary(source) {
        return Ok(source.to_path_buf());
    }

    let runtime_dir = paths.home.join("runtime").join("daemon-lifecycle");
    fs::create_dir_all(&runtime_dir)?;
    cleanup_stale_runtime_copies(&runtime_dir);

    let runtime_path = runtime_copy_path(&runtime_dir, source);
    fs::copy(source, &runtime_path)?;
    Ok(runtime_path)
}

fn is_workspace_daemon_binary(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("daemon-rs"))
            .unwrap_or(false)
    })
}

fn runtime_copy_path(runtime_dir: &Path, source: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    runtime_dir.join(format!(
        "cortex-daemon-run-{}-{}{}",
        std::process::id(),
        unique,
        extension
    ))
}

fn cleanup_stale_runtime_copies(runtime_dir: &Path) {
    let now = std::time::SystemTime::now();
    let Ok(entries) = fs::read_dir(runtime_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("cortex-daemon-run-") {
            continue;
        }
        let stale = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|age| age.as_secs() >= RUNTIME_COPY_STALE_SECS)
            .unwrap_or(false);
        if stale {
            let _ = fs::remove_file(path);
        }
    }
}

/// Attempt to respawn the daemon and wait for it to become healthy.
/// Returns true if the daemon is healthy after respawn.
pub async fn try_respawn(paths: &CortexPaths) -> bool {
    let respawn_lock = match crate::auth::acquire_daemon_lock(paths) {
        Ok(lock) => lock,
        Err(_) => {
            eprintln!(
                "[cortex-lifecycle] Respawn already in progress; waiting for daemon health on port {}",
                paths.port
            );
            return wait_for_health(
                paths.port,
                Duration::from_secs(RESPAWN_COORDINATION_WAIT_SECS),
            )
            .await;
        }
    };

    if daemon_healthy(paths.port).await {
        return true;
    }

    eprintln!(
        "[cortex-lifecycle] Daemon appears dead on port {}; attempting respawn",
        paths.port
    );

    if let Err(e) = spawn_daemon(paths) {
        eprintln!("[cortex-lifecycle] Respawn failed: {e}");
        return false;
    }

    drop(respawn_lock);
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
        assert!(!is_cortex_health_payload(
            500,
            r#"{"status":"ok","runtime":{}}"#
        ));
    }
}
