use crate::constants::*;
use crate::cortex_http::request::{send_cortex_request_with_port, RequestTimeouts};
use crate::daemon::paths::{daemon_port, resolved_cortex_paths, token_path, ResolvedCortexPaths};
use std::fs;
use std::path::Path;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub struct CortexReachabilityProbe {
    pub reachable: bool,
    pub starting: bool,
    pub identity_mismatch: bool,
}
pub fn readiness_state_with_identity_fallback(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&ResolvedCortexPaths>,
) -> (Option<bool>, bool) {
    if let Some(ready) = cortex_readiness_state(status, body, expected_port, expected_paths) {
        return (Some(ready), false);
    }
    if expected_paths.is_some() {
        if let Some(ready) = cortex_readiness_state(status, body, expected_port, None) {
            return (Some(ready), true);
        }
    }
    (None, false)
}

pub fn health_state_with_identity_fallback(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&ResolvedCortexPaths>,
) -> (bool, bool) {
    if is_cortex_health_response(status, body, expected_port, expected_paths) {
        return (true, false);
    }
    if expected_paths.is_some() && is_cortex_health_response(status, body, expected_port, None) {
        return (true, true);
    }
    (false, false)
}

pub fn probe_cortex_reachability_with_port(port: u16, timeout_ms: u64) -> CortexReachabilityProbe {
    let expected_paths = resolved_cortex_paths();
    let readiness_response = send_cortex_request_with_port(
        port,
        "GET",
        "/readiness",
        "",
        None,
        RequestTimeouts {
            connect: Duration::from_millis(timeout_ms),
            read: Duration::from_millis(timeout_ms),
            write: Duration::from_millis(timeout_ms),
        },
    );

    if let Ok(resp) = readiness_response {
        let (readiness_state, identity_mismatch) = readiness_state_with_identity_fallback(
            resp.status,
            &resp.body,
            Some(port),
            Some(&expected_paths),
        );
        if let Some(ready) = readiness_state {
            return CortexReachabilityProbe {
                reachable: ready,
                starting: !ready,
                identity_mismatch,
            };
        }
    }

    let health_response = send_cortex_request_with_port(
        port,
        "GET",
        "/health",
        "",
        None,
        RequestTimeouts {
            connect: Duration::from_millis(timeout_ms),
            read: Duration::from_millis(timeout_ms),
            write: Duration::from_millis(timeout_ms),
        },
    );

    if let Ok(resp) = health_response {
        let (healthy, identity_mismatch) = health_state_with_identity_fallback(
            resp.status,
            &resp.body,
            Some(port),
            Some(&expected_paths),
        );
        if healthy {
            return CortexReachabilityProbe {
                reachable: true,
                starting: false,
                identity_mismatch,
            };
        }
    }

    CortexReachabilityProbe::default()
}

pub fn is_cortex_reachable_with_port(port: u16, timeout_ms: u64) -> bool {
    probe_cortex_reachability_with_port(port, timeout_ms).reachable
}

pub async fn wait_for_reachability(port: u16, target: bool, timeout: Duration) -> bool {
    tauri::async_runtime::spawn_blocking(move || {
        wait_for_reachability_blocking(port, target, timeout)
    })
    .await
    .unwrap_or(false)
}

pub fn wait_for_reachability_blocking(port: u16, target: bool, timeout: Duration) -> bool {
    let started = std::time::Instant::now();
    loop {
        if is_cortex_reachable_with_port(port, DAEMON_REACHABILITY_TIMEOUT_MS) == target {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(DAEMON_WAIT_POLL_MS));
    }
}
pub fn read_auth_token_once() -> Result<String, String> {
    let path = token_path()?;
    let token = fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read token at {}: {err}", path.display()))?;
    Ok(token.trim().to_string())
}

pub fn auth_token_ready() -> bool {
    matches!(read_auth_token_once(), Ok(token) if !token.is_empty())
}

pub fn read_auth_token_with_retry_blocking(timeout: Duration) -> Result<String, String> {
    let path = token_path()?;
    if !is_cortex_reachable_with_port(daemon_port(), DAEMON_REACHABILITY_TIMEOUT_MS) {
        return read_auth_token_once();
    }

    let started = std::time::Instant::now();
    let mut last_error = format!("Auth token not ready at {}", path.display());

    loop {
        match fs::read_to_string(&path) {
            Ok(token) => {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
                last_error = format!("Auth token file is empty at {}", path.display());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                last_error = format!("Auth token file not found at {}", path.display());
            }
            Err(err) => {
                last_error = format!("Failed to read token at {}: {err}", path.display());
            }
        }

        if started.elapsed() >= timeout {
            return Err(last_error);
        }

        std::thread::sleep(Duration::from_millis(AUTH_TOKEN_POLL_MS));
    }
}

pub async fn read_auth_token_with_retry() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        read_auth_token_with_retry_blocking(Duration::from_millis(AUTH_TOKEN_WAIT_MS))
    })
    .await
    .map_err(|err| format!("Auth token wait task failed: {err}"))?
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

fn health_path_field_matches(value: Option<&serde_json::Value>, expected: Option<&Path>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let expected = normalize_runtime_path(&expected.to_string_lossy());
    value
        .and_then(|field| field.as_str())
        .map(normalize_runtime_path)
        .is_some_and(|actual| actual == expected)
}

pub fn cortex_readiness_state(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&ResolvedCortexPaths>,
) -> Option<bool> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(body.trim()) else {
        return None;
    };

    let ready = json.get("ready").and_then(|value| value.as_bool())?;
    let runtime = json.get("runtime").and_then(|value| value.as_object())?;
    let stats = json.get("stats").and_then(|value| value.as_object())?;
    let runtime_port = runtime
        .get("port")
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok());

    if let Some(expected_port) = expected_port {
        if runtime_port != Some(expected_port) {
            return None;
        }
    }

    if let Some(paths) = expected_paths {
        if !health_path_field_matches(stats.get("home"), paths.home.as_deref()) {
            return None;
        }
        if !health_path_field_matches(runtime.get("token_path"), paths.token.as_deref()) {
            return None;
        }
        if !health_path_field_matches(runtime.get("db_path"), paths.db.as_deref()) {
            return None;
        }
        if !health_path_field_matches(runtime.get("pid_path"), paths.pid.as_deref()) {
            return None;
        }
    }

    if ready && !(200..300).contains(&status) {
        return None;
    }
    if !ready && status != 503 && !(200..300).contains(&status) {
        return None;
    }

    let readiness_status = json.get("status").and_then(|value| value.as_str());
    let expected_status = if ready { "ready" } else { "starting" };
    if let Some(readiness_status) = readiness_status {
        if readiness_status != expected_status {
            return None;
        }
    }

    Some(ready)
}

pub fn is_cortex_health_response(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&ResolvedCortexPaths>,
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
        if !health_path_field_matches(stats.and_then(|obj| obj.get("home")), paths.home.as_deref())
        {
            return false;
        }
        if !health_path_field_matches(
            runtime.and_then(|obj| obj.get("token_path")),
            paths.token.as_deref(),
        ) {
            return false;
        }
        if !health_path_field_matches(
            runtime.and_then(|obj| obj.get("db_path")),
            paths.db.as_deref(),
        ) {
            return false;
        }
        if !health_path_field_matches(
            runtime.and_then(|obj| obj.get("pid_path")),
            paths.pid.as_deref(),
        ) {
            return false;
        }
    }

    matches!(health_status, Some("ok" | "degraded")) && runtime.is_some() && stats.is_some()
}
