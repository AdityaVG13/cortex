// SPDX-License-Identifier: MIT
//! Shared daemon lifecycle utilities: health checks, spawn, respawn.
//!
//! Used by both the CLI (`ensure_daemon`) and the MCP proxy (auto-respawn).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::auth::CortexPaths;
use fs2::FileExt;

const RESPAWN_HEALTH_TIMEOUT_SECS: u64 = 90;
const RESPAWN_COORDINATION_WAIT_SECS: u64 = 20;
const RUNTIME_COPY_STALE_SECS: u64 = 600;

fn health_probe_base(bind: &str, port: u16) -> String {
    let bind = bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{port}")
}

/// Check if the daemon responds to /health within 2s.
async fn daemon_healthy_at(bind: &str, port: u16, expected_paths: Option<&CortexPaths>) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let health_url = format!("{}/health", health_probe_base(bind, port));
    let response = match client.get(health_url).send().await {
        Ok(response) => response,
        Err(_) => return false,
    };

    let status = response.status().as_u16();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return false,
    };

    is_cortex_health_payload(status, &body, Some(port), expected_paths)
}

pub async fn daemon_healthy(paths: &CortexPaths) -> bool {
    daemon_healthy_at(&paths.bind, paths.port, Some(paths)).await
}

fn normalize_runtime_path(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    #[cfg(windows)]
    {
        normalized = normalized.to_ascii_lowercase();
    }
    normalized
}

fn path_field_matches(value: Option<&serde_json::Value>, expected: &Path) -> bool {
    let expected = normalize_runtime_path(&expected.to_string_lossy());
    value
        .and_then(|field| field.as_str())
        .map(normalize_runtime_path)
        .is_some_and(|actual| actual == expected)
}

pub(crate) fn is_cortex_health_payload(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&CortexPaths>,
) -> bool {
    if !(200..300).contains(&status) {
        return false;
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return false;
    };

    let health_status = json.get("status").and_then(|value| value.as_str());
    let runtime = json.get("runtime").and_then(|value| value.as_object());
    let stats = json.get("stats").and_then(|value| value.as_object());
    let runtime_port = runtime
        .and_then(|runtime| runtime.get("port"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok());

    if let Some(expected_port) = expected_port {
        if runtime_port != Some(expected_port) {
            return false;
        }
    }

    if let Some(paths) = expected_paths {
        if !path_field_matches(stats.and_then(|obj| obj.get("home")), &paths.home) {
            return false;
        }
        if !path_field_matches(runtime.and_then(|obj| obj.get("token_path")), &paths.token) {
            return false;
        }
        if !path_field_matches(runtime.and_then(|obj| obj.get("pid_path")), &paths.pid) {
            return false;
        }
        if !path_field_matches(runtime.and_then(|obj| obj.get("db_path")), &paths.db) {
            return false;
        }
    }

    matches!(health_status, Some("ok" | "degraded")) && runtime.is_some() && stats.is_some()
}

/// Poll /health on the resolved bind host/port until success or timeout.
/// Requires runtime identity to match the expected local Cortex paths.
pub async fn wait_for_health(paths: &CortexPaths, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() <= timeout {
        if daemon_healthy(paths).await {
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
        .env("CORTEX_BIND", &paths.bind)
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

fn acquire_respawn_coordination_lock(paths: &CortexPaths) -> Result<fs::File, String> {
    fs::create_dir_all(&paths.home).map_err(|e| format!("create home: {e}"))?;
    let respawn_lock = paths.home.join("cortex.respawn.lock");
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(respawn_lock)
        .map_err(|e| format!("open respawn lock: {e}"))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| "another respawn attempt is already in progress".to_string())?;
    Ok(lock_file)
}

/// Attempt to respawn the daemon and wait for it to become healthy.
/// Returns true if the daemon is healthy after respawn.
pub async fn try_respawn(paths: &CortexPaths) -> bool {
    let _respawn_coordinator = match acquire_respawn_coordination_lock(paths) {
        Ok(lock) => lock,
        Err(_) => {
            eprintln!(
                "[cortex-lifecycle] Respawn already in progress; waiting for daemon health on port {}",
                paths.port
            );
            return wait_for_health(paths, Duration::from_secs(RESPAWN_COORDINATION_WAIT_SECS))
                .await;
        }
    };

    let daemon_lock = match crate::auth::acquire_daemon_lock(paths) {
        Ok(lock) => lock,
        Err(_) => {
            eprintln!(
                "[cortex-lifecycle] Daemon lock is held by another process; waiting for health on port {}",
                paths.port
            );
            return wait_for_health(paths, Duration::from_secs(RESPAWN_COORDINATION_WAIT_SECS))
                .await;
        }
    };

    if daemon_healthy(paths).await {
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

    drop(daemon_lock);
    let healthy = wait_for_health(paths, Duration::from_secs(RESPAWN_HEALTH_TIMEOUT_SECS)).await;
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
    use super::{acquire_respawn_coordination_lock, is_cortex_health_payload};
    use crate::auth::CortexPaths;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_lifecycle_{name}_{unique}"))
    }

    #[test]
    fn cortex_health_payload_accepts_expected_shapes() {
        assert!(is_cortex_health_payload(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#,
            Some(7437),
            None,
        ));
        assert!(is_cortex_health_payload(
            200,
            r#"{"status":"degraded","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#,
            Some(7437),
            None,
        ));
    }

    #[test]
    fn cortex_health_payload_rejects_non_cortex_bodies() {
        assert!(!is_cortex_health_payload(
            200,
            r#"{"status":"ok"}"#,
            Some(7437),
            None
        ));
        assert!(!is_cortex_health_payload(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0"}}"#,
            Some(7437),
            None,
        ));
        assert!(!is_cortex_health_payload(
            200,
            "<html>ok</html>",
            Some(7437),
            None
        ));
        assert!(!is_cortex_health_payload(
            500,
            r#"{"status":"ok","runtime":{}}"#,
            Some(7437),
            None,
        ));
        assert!(!is_cortex_health_payload(
            200,
            r#"{"status":"ok","runtime":{"version":"0.5.0","port":9000},"stats":{"memories":1}}"#,
            Some(7437),
            None,
        ));
    }

    #[test]
    fn cortex_health_payload_rejects_identity_mismatch_for_local_expectations() {
        let home_dir = temp_test_dir("identity_mismatch");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );

        let valid_body = json!({
            "status": "ok",
            "stats": {
                "memories": 1,
                "home": paths.home.display().to_string()
            },
            "runtime": {
                "version": "0.5.0",
                "port": paths.port,
                "db_path": paths.db.display().to_string(),
                "token_path": paths.token.display().to_string(),
                "pid_path": paths.pid.display().to_string()
            }
        })
        .to_string();
        assert!(is_cortex_health_payload(
            200,
            &valid_body,
            Some(paths.port),
            Some(&paths),
        ));

        let bad_token_body = json!({
            "status": "ok",
            "stats": {
                "memories": 1,
                "home": paths.home.display().to_string()
            },
            "runtime": {
                "version": "0.5.0",
                "port": paths.port,
                "db_path": paths.db.display().to_string(),
                "token_path": "C:/wrong/token",
                "pid_path": paths.pid.display().to_string()
            }
        })
        .to_string();
        assert!(!is_cortex_health_payload(
            200,
            &bad_token_body,
            Some(paths.port),
            Some(&paths),
        ));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn respawn_coordination_lock_rejects_concurrent_holder() {
        let home_dir = temp_test_dir("respawn_lock");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );

        let first = acquire_respawn_coordination_lock(&paths).expect("first lock should succeed");
        let second = acquire_respawn_coordination_lock(&paths).unwrap_err();
        assert!(second.contains("already in progress"));
        drop(first);

        let _ = std::fs::remove_dir_all(&home_dir);
    }
}
