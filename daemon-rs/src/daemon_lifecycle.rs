// SPDX-License-Identifier: MIT
//! Shared daemon lifecycle utilities: health checks, spawn, respawn.
//!
//! Used by both the CLI (`ensure_daemon`) and the MCP proxy (auto-respawn).

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::auth::CortexPaths;
use fs2::FileExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use uuid::Uuid;

const RESPAWN_HEALTH_TIMEOUT_SECS: u64 = 90;
const RESPAWN_COORDINATION_WAIT_SECS: u64 = 20;
const RUNTIME_COPY_STALE_SECS: u64 = 600;
const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
const PLUGIN_CLAUDE_OWNER_TAG: &str = "plugin-claude";
const DAEMON_OWNER_SIGNING_KEY_FILE: &str = "daemon-owner-signing.key";
const DAEMON_OWNER_TOKEN_VERSION: &str = "v1";
const DAEMON_OWNER_TOKEN_TTL_SECS: u64 = 180;
pub const DAEMON_OWNER_TOKEN_ENV: &str = "CORTEX_DAEMON_OWNER_TOKEN";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonOwnerMode {
    AppControlCenter,
    PluginClaude,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RespawnPolicyDecision {
    pub mode: DaemonOwnerMode,
    pub allow_respawn: bool,
    pub reason: &'static str,
}

pub fn classify_owner_mode(owner_tag: Option<&str>) -> DaemonOwnerMode {
    match owner_tag {
        Some(owner) if owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG) => {
            DaemonOwnerMode::AppControlCenter
        }
        Some(owner) if owner.eq_ignore_ascii_case(PLUGIN_CLAUDE_OWNER_TAG) => {
            DaemonOwnerMode::PluginClaude
        }
        _ => DaemonOwnerMode::Unknown,
    }
}

pub fn evaluate_respawn_policy(
    owner_tag: Option<&str>,
    control_center_active: bool,
) -> RespawnPolicyDecision {
    let mode = classify_owner_mode(owner_tag);
    if control_center_active && !matches!(mode, DaemonOwnerMode::AppControlCenter) {
        return RespawnPolicyDecision {
            mode,
            allow_respawn: false,
            reason: "control-center-active-non-app-owner",
        };
    }
    let reason = if control_center_active {
        "control-center-active-app-owner"
    } else {
        "control-center-inactive"
    };
    RespawnPolicyDecision {
        mode,
        allow_respawn: true,
        reason,
    }
}

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
pub fn spawn_daemon(paths: &CortexPaths, owner_tag: Option<&str>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("resolve current exe: {e}"))?;
    let spawn_exe =
        prepare_spawn_executable(paths, &exe).map_err(|e| format!("prepare spawn copy: {e}"))?;
    let launcher_pid = std::process::id();
    let mut cmd = Command::new(spawn_exe);
    cmd.arg("serve")
        .env("CORTEX_HOME", &paths.home)
        .env("CORTEX_DB", &paths.db)
        .env("CORTEX_PORT", paths.port.to_string())
        .env("CORTEX_BIND", &paths.bind)
        .env("CORTEX_SPAWN_PARENT_PID", launcher_pid.to_string())
        .env("CORTEX_WAIT_FOR_DAEMON_LOCK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(owner_tag) = owner_tag {
        let token = issue_owner_token_for_spawn(paths, owner_tag, launcher_pid)?;
        validate_spawned_owner_claim(paths, Some(owner_tag), Some(launcher_pid), Some(&token))?;
        cmd.env("CORTEX_DAEMON_OWNER", owner_tag);
        cmd.env(DAEMON_OWNER_TOKEN_ENV, token);
    }

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

    let runtime_path = stable_runtime_path(&runtime_dir, source);

    // Only copy when the stable runtime binary is missing or source size
    // changed (new build).  Reusing the same path prevents Windows
    // SmartScreen from re-prompting on every spawn of an unsigned binary.
    let needs_copy = match (fs::metadata(source), fs::metadata(&runtime_path)) {
        (Ok(src_meta), Ok(dst_meta)) => {
            let size_changed = src_meta.len() != dst_meta.len();
            let source_newer = match (src_meta.modified(), dst_meta.modified()) {
                (Ok(src_modified), Ok(dst_modified)) => src_modified > dst_modified,
                _ => true,
            };
            size_changed || source_newer
        }
        (Ok(_), Err(_)) => true,
        _ => true,
    };

    if needs_copy {
        fs::copy(source, &runtime_path)?;
    }
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

/// Deterministic runtime copy path.  A stable name (no PID/timestamp) means
/// Windows SmartScreen evaluates the binary once; subsequent spawns reuse the
/// allowed path without re-prompting.
fn stable_runtime_path(runtime_dir: &Path, source: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    runtime_dir.join(format!("cortex-daemon-run{extension}"))
}

fn is_runtime_copy_name(name: &str) -> bool {
    name == "cortex-daemon-run"
        || name.starts_with("cortex-daemon-run.")
        || name.starts_with("cortex-daemon-run-")
}

/// Remove old timestamped copies (pre-stable-path migration) and the stable
/// copy when it has gone stale.
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
        if !is_runtime_copy_name(name) {
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
pub async fn try_respawn(paths: &CortexPaths, owner_tag: Option<&str>) -> bool {
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

    let respawn_policy =
        evaluate_respawn_policy(owner_tag, crate::auth::control_center_is_active(paths));
    if !respawn_policy.allow_respawn {
        eprintln!(
            "[cortex-lifecycle] Respawn denied (reason='{}', mode={:?}, owner='{}')",
            respawn_policy.reason,
            respawn_policy.mode,
            owner_tag.unwrap_or("unknown")
        );
        return wait_for_health(paths, Duration::from_secs(RESPAWN_COORDINATION_WAIT_SECS)).await;
    }

    if let Err(e) = spawn_daemon(paths, owner_tag) {
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
    use super::{
        acquire_respawn_coordination_lock, build_owner_token, evaluate_respawn_policy,
        is_cortex_health_payload, is_runtime_copy_name, issue_owner_token_for_spawn,
        load_or_create_owner_signing_key, stable_runtime_path, validate_spawned_owner_claim,
        DaemonOwnerMode, DAEMON_OWNER_TOKEN_TTL_SECS,
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

    #[test]
    fn respawn_policy_allows_control_center_owner_when_app_lock_is_active() {
        let decision = evaluate_respawn_policy(Some("control-center"), true);
        assert_eq!(decision.mode, DaemonOwnerMode::AppControlCenter);
        assert!(decision.allow_respawn);
        assert_eq!(decision.reason, "control-center-active-app-owner");
    }

    #[test]
    fn respawn_policy_denies_non_app_owner_when_app_lock_is_active() {
        let plugin = evaluate_respawn_policy(Some("plugin-claude"), true);
        assert_eq!(plugin.mode, DaemonOwnerMode::PluginClaude);
        assert!(!plugin.allow_respawn);
        assert_eq!(plugin.reason, "control-center-active-non-app-owner");

        let unknown = evaluate_respawn_policy(None, true);
        assert_eq!(unknown.mode, DaemonOwnerMode::Unknown);
        assert!(!unknown.allow_respawn);
        assert_eq!(unknown.reason, "control-center-active-non-app-owner");
    }

    #[test]
    fn respawn_policy_allows_non_app_owner_when_app_lock_is_inactive() {
        let decision = evaluate_respawn_policy(Some("plugin-claude"), false);
        assert_eq!(decision.mode, DaemonOwnerMode::PluginClaude);
        assert!(decision.allow_respawn);
        assert_eq!(decision.reason, "control-center-inactive");
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

    #[test]
    fn runtime_copy_name_classifier_matches_stable_and_legacy_paths() {
        assert!(is_runtime_copy_name("cortex-daemon-run"));
        assert!(is_runtime_copy_name("cortex-daemon-run.exe"));
        assert!(is_runtime_copy_name("cortex-daemon-run-123-456.exe"));
        assert!(!is_runtime_copy_name("cortex-daemon"));
        assert!(!is_runtime_copy_name("daemon-cortex-run.exe"));
    }

    #[test]
    fn stable_runtime_path_is_deterministic() {
        let runtime_dir = temp_test_dir("stable_runtime_path");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime dir");
        let source = runtime_dir.join("cortex.exe");
        std::fs::write(&source, b"binary").expect("seed source");

        let first = stable_runtime_path(&runtime_dir, &source);
        let second = stable_runtime_path(&runtime_dir, &source);
        assert_eq!(first, second);
        assert!(first
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name == "cortex-daemon-run.exe"));

        let _ = std::fs::remove_dir_all(&runtime_dir);
    }
}
