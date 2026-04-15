// SPDX-License-Identifier: MIT
//! Shared daemon lifecycle utilities: health checks and runtime identity checks.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::CortexPaths;
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

const DAEMON_OWNER_SIGNING_KEY_FILE: &str = "daemon-owner-signing.key";
const DAEMON_OWNER_TOKEN_VERSION: &str = "v1";
const DAEMON_OWNER_TOKEN_TTL_SECS: u64 = 180;
pub const DAEMON_OWNER_TOKEN_ENV: &str = "CORTEX_DAEMON_OWNER_TOKEN";
pub const SPAWN_PARENT_START_TIME_ENV: &str = "CORTEX_SPAWN_PARENT_START_TIME";

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedOwnerToken {
    issued_at: u64,
    parent_pid: u32,
    nonce: String,
    signature: Vec<u8>,
}

fn daemon_owner_runtime_dir(paths: &CortexPaths) -> PathBuf {
    paths.home.join("runtime")
}

fn daemon_owner_signing_key_path(paths: &CortexPaths) -> PathBuf {
    daemon_owner_runtime_dir(paths).join(DAEMON_OWNER_SIGNING_KEY_FILE)
}

fn generate_owner_signing_key() -> [u8; 32] {
    let mut key = [0_u8; 32];
    key[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(Uuid::new_v4().as_bytes());
    key
}

fn load_or_create_owner_signing_key(paths: &CortexPaths) -> Result<Vec<u8>, String> {
    let runtime_dir = daemon_owner_runtime_dir(paths);
    fs::create_dir_all(&runtime_dir).map_err(|e| format!("create runtime dir: {e}"))?;
    let key_path = daemon_owner_signing_key_path(paths);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&key_path)
        .map_err(|e| format!("open daemon owner signing key: {e}"))?;
    file.lock_exclusive()
        .map_err(|e| format!("lock daemon owner signing key: {e}"))?;

    let mut key_bytes = Vec::new();
    file.read_to_end(&mut key_bytes)
        .map_err(|e| format!("read daemon owner signing key: {e}"))?;
    if key_bytes.len() != 32 {
        key_bytes = generate_owner_signing_key().to_vec();
        file.set_len(0)
            .map_err(|e| format!("truncate daemon owner signing key: {e}"))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| format!("seek daemon owner signing key: {e}"))?;
        file.write_all(&key_bytes)
            .map_err(|e| format!("write daemon owner signing key: {e}"))?;
        file.flush()
            .map_err(|e| format!("flush daemon owner signing key: {e}"))?;
    }
    let _ = file.unlock();
    Ok(key_bytes)
}

#[cfg(test)]
fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex string length must be even".to_string());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    let parse_nibble = |ch: u8| -> Result<u8, String> {
        match ch {
            b'0'..=b'9' => Ok(ch - b'0'),
            b'a'..=b'f' => Ok(ch - b'a' + 10),
            b'A'..=b'F' => Ok(ch - b'A' + 10),
            _ => Err("invalid hex character".to_string()),
        }
    };
    for index in (0..bytes.len()).step_by(2) {
        let hi = parse_nibble(bytes[index])?;
        let lo = parse_nibble(bytes[index + 1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn sign_owner_token_claim(
    key: &[u8],
    owner_tag: &str,
    parent_pid: u32,
    issued_at: u64,
    nonce: &str,
) -> Result<Vec<u8>, String> {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|e| format!("init owner token signer: {e}"))?;
    let payload = format!(
        "{}|{}|{}|{}|{}",
        DAEMON_OWNER_TOKEN_VERSION, owner_tag, parent_pid, issued_at, nonce
    );
    mac.update(payload.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

#[cfg(test)]
fn build_owner_token(
    key: &[u8],
    owner_tag: &str,
    parent_pid: u32,
    issued_at: u64,
    nonce: &str,
) -> Result<String, String> {
    let signature = sign_owner_token_claim(key, owner_tag, parent_pid, issued_at, nonce)?;
    Ok(format!(
        "{}.{}.{}.{}.{}",
        DAEMON_OWNER_TOKEN_VERSION,
        issued_at,
        parent_pid,
        nonce,
        encode_hex(&signature)
    ))
}

fn parse_owner_token(token: &str) -> Result<ParsedOwnerToken, String> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 5 {
        return Err("owner token format is invalid".to_string());
    }
    if parts[0] != DAEMON_OWNER_TOKEN_VERSION {
        return Err("owner token version is unsupported".to_string());
    }
    let issued_at = parts[1]
        .parse::<u64>()
        .map_err(|_| "owner token issued timestamp is invalid".to_string())?;
    let parent_pid = parts[2]
        .parse::<u32>()
        .map_err(|_| "owner token parent pid is invalid".to_string())?;
    let nonce = parts[3].trim().to_string();
    if nonce.is_empty() {
        return Err("owner token nonce is missing".to_string());
    }
    let signature = decode_hex(parts[4])?;
    Ok(ParsedOwnerToken {
        issued_at,
        parent_pid,
        nonce,
        signature,
    })
}

#[cfg(test)]
fn issue_owner_token_for_spawn(
    paths: &CortexPaths,
    owner_tag: &str,
    parent_pid: u32,
) -> Result<String, String> {
    let key = load_or_create_owner_signing_key(paths)?;
    let issued_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let nonce = Uuid::new_v4().simple().to_string();
    build_owner_token(&key, owner_tag, parent_pid, issued_at, &nonce)
}

pub fn validate_spawned_owner_claim(
    paths: &CortexPaths,
    owner_tag: Option<&str>,
    parent_pid: Option<u32>,
    owner_token: Option<&str>,
) -> Result<(), String> {
    let Some(owner_tag) = owner_tag.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let Some(parent_pid) = parent_pid else {
        // Direct `cortex serve` invocations may set owner metadata without
        // spawn linkage. Only enforce token validation for spawned owners.
        return Ok(());
    };
    let token = owner_token
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "spawned owner claim is missing ownership token".to_string())?;

    let parsed = parse_owner_token(token)?;
    if parsed.parent_pid != parent_pid {
        return Err(format!(
            "owner token parent mismatch (token={}, env={})",
            parsed.parent_pid, parent_pid
        ));
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    if now.saturating_sub(parsed.issued_at) > DAEMON_OWNER_TOKEN_TTL_SECS {
        return Err("owner token is stale".to_string());
    }

    let key = load_or_create_owner_signing_key(paths)?;
    let expected_signature = sign_owner_token_claim(
        &key,
        owner_tag,
        parsed.parent_pid,
        parsed.issued_at,
        &parsed.nonce,
    )?;
    if expected_signature != parsed.signature {
        return Err("owner token signature mismatch".to_string());
    }
    Ok(())
}

fn health_probe_base(bind: &str, port: u16) -> String {
    let bind = bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{port}")
}

fn health_probe_url(bind: &str, port: u16) -> String {
    format!("{}/health", health_probe_base(bind, port))
}

fn readiness_probe_url(bind: &str, port: u16) -> String {
    format!("{}/readiness", health_probe_base(bind, port))
}

/// Check if the daemon is ready within a short timeout.
/// Prefers `/readiness` and falls back to `/health` for backward compatibility.
async fn daemon_healthy_at(bind: &str, port: u16, expected_paths: Option<&CortexPaths>) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };

    let readiness_url = readiness_probe_url(bind, port);
    if let Ok(response) = client.get(&readiness_url).send().await {
        let status = response.status().as_u16();
        if let Ok(body) = response.text().await {
            if let Some(ready) =
                readiness_state_from_payload(status, &body, Some(port), expected_paths)
            {
                return ready;
            }
        }
    }

    let health_url = health_probe_url(bind, port);
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

pub(crate) fn readiness_state_from_payload(
    status: u16,
    body: &str,
    expected_port: Option<u16>,
    expected_paths: Option<&CortexPaths>,
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
        if !path_field_matches(stats.get("home"), &paths.home) {
            return None;
        }
        if !path_field_matches(runtime.get("token_path"), &paths.token) {
            return None;
        }
        if !path_field_matches(runtime.get("pid_path"), &paths.pid) {
            return None;
        }
        if !path_field_matches(runtime.get("db_path"), &paths.db) {
            return None;
        }
    }

    // Ready payloads are expected to be 2xx. Not-ready payloads are expected
    // to be 503 (with optional legacy 2xx compatibility).
    if ready && !(200..300).contains(&status) {
        return None;
    }
    if !ready {
        let expected_not_ready_status = status == 503 || (200..300).contains(&status);
        if !expected_not_ready_status {
            return None;
        }
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

/// Poll readiness on the resolved bind host/port until success or timeout.
/// Prefers `/readiness` and falls back to `/health` for backward compatibility.
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

#[cfg(test)]
mod tests {
    use super::{
        build_owner_token, is_cortex_health_payload, issue_owner_token_for_spawn,
        load_or_create_owner_signing_key, readiness_state_from_payload,
        validate_spawned_owner_claim, DAEMON_OWNER_TOKEN_TTL_SECS,
    };
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
    fn cortex_readiness_payload_reports_ready_and_starting_states() {
        let ready = serde_json::json!({
            "status": "ready",
            "ready": true,
            "runtime": { "port": 7437 },
            "stats": { "home": "C:/Users/aditya/.cortex" }
        })
        .to_string();
        assert_eq!(
            readiness_state_from_payload(200, &ready, Some(7437), None),
            Some(true)
        );

        let starting = serde_json::json!({
            "status": "starting",
            "ready": false,
            "runtime": { "port": 7437 },
            "stats": { "home": "C:/Users/aditya/.cortex" }
        })
        .to_string();
        assert_eq!(
            readiness_state_from_payload(503, &starting, Some(7437), None),
            Some(false)
        );
    }

    #[test]
    fn cortex_readiness_payload_rejects_invalid_shapes() {
        assert_eq!(
            readiness_state_from_payload(200, r#"{"status":"ready"}"#, Some(7437), None),
            None
        );
        assert_eq!(
            readiness_state_from_payload(
                200,
                r#"{"status":"ready","ready":true,"runtime":{"port":9000},"stats":{"home":"C:/Users/aditya/.cortex"}}"#,
                Some(7437),
                None
            ),
            None
        );
        assert_eq!(
            readiness_state_from_payload(
                500,
                r#"{"status":"starting","ready":false,"runtime":{"port":7437},"stats":{"home":"C:/Users/aditya/.cortex"}}"#,
                Some(7437),
                None
            ),
            None
        );
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
    fn owner_token_round_trip_validates_for_spawned_owner() {
        let home_dir = temp_test_dir("owner_token_round_trip");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );
        let token =
            issue_owner_token_for_spawn(&paths, "plugin-claude", 4242).expect("issue owner token");
        validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(4242), Some(&token))
            .expect("validate owner token");
        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn owner_token_validation_rejects_parent_or_owner_mismatch() {
        let home_dir = temp_test_dir("owner_token_mismatch");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );
        let token =
            issue_owner_token_for_spawn(&paths, "plugin-claude", 1111).expect("issue owner token");

        let wrong_parent =
            validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(2222), Some(&token))
                .unwrap_err();
        assert!(wrong_parent.contains("parent mismatch"));

        let wrong_owner =
            validate_spawned_owner_claim(&paths, Some("control-center"), Some(1111), Some(&token))
                .unwrap_err();
        assert!(wrong_owner.contains("signature mismatch"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn owner_token_validation_rejects_missing_or_stale_token() {
        let home_dir = temp_test_dir("owner_token_stale");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );

        let missing = validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(9999), None)
            .unwrap_err();
        assert!(missing.contains("missing ownership token"));

        let key = load_or_create_owner_signing_key(&paths).expect("load owner signing key");
        let stale_issued = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
            .saturating_sub(DAEMON_OWNER_TOKEN_TTL_SECS + 10);
        let stale_token =
            build_owner_token(&key, "plugin-claude", 9999, stale_issued, "stale_nonce")
                .expect("build stale token");
        let stale_error = validate_spawned_owner_claim(
            &paths,
            Some("plugin-claude"),
            Some(9999),
            Some(&stale_token),
        )
        .unwrap_err();
        assert!(stale_error.contains("stale"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn owner_token_validation_skips_unspawned_owner_claims() {
        let home_dir = temp_test_dir("owner_token_unspawned");
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        );
        validate_spawned_owner_claim(&paths, Some("control-center"), None, None)
            .expect("unspawned owner claims should remain backwards compatible");
        let _ = std::fs::remove_dir_all(&home_dir);
    }
}
