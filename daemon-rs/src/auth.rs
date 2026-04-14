// SPDX-License-Identifier: MIT
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use fs2::FileExt;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const CORTEX_DIR_NAME: &str = ".cortex";
const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

// ---------------------------------------------------------------------------
// CortexPaths -- centralized path + port resolver
// ---------------------------------------------------------------------------

/// Resolved paths for all Cortex runtime files.
/// Priority: CLI flag > env var > default.
#[derive(Debug, Clone)]
pub struct CortexPaths {
    pub home: PathBuf,
    pub db: PathBuf,
    pub token: PathBuf,
    pub pid: PathBuf,
    pub lock: PathBuf,
    pub port: u16,
    pub bind: String,
    pub models: PathBuf,
    #[allow(dead_code)]
    pub write_buffer: PathBuf,
}

impl CortexPaths {
    /// Resolve paths from environment variables only (no CLI args).
    pub fn resolve() -> Self {
        Self::resolve_with_overrides(None, None, None, None)
    }

    /// Resolve paths with optional CLI overrides.
    pub fn resolve_with_overrides(
        home_override: Option<&str>,
        db_override: Option<&str>,
        port_override: Option<u16>,
        bind_override: Option<&str>,
    ) -> Self {
        let home = home_override
            .map(PathBuf::from)
            .or_else(|| std::env::var("CORTEX_HOME").ok().map(PathBuf::from))
            .unwrap_or_else(cortex_dir);

        let db = db_override
            .map(PathBuf::from)
            .or_else(|| std::env::var("CORTEX_DB").ok().map(PathBuf::from))
            .unwrap_or_else(|| home.join("cortex.db"));

        let port = port_override
            .or_else(|| {
                std::env::var("CORTEX_PORT")
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
            .unwrap_or(7437);
        let env_bind = std::env::var("CORTEX_BIND").ok();
        let bind = resolve_bind(bind_override, env_bind.as_deref());

        Self {
            token: home.join("cortex.token"),
            pid: home.join("cortex.pid"),
            lock: home.join("cortex.lock"),
            models: home.join("models"),
            write_buffer: home.join("write_buffer.jsonl"),
            home,
            db,
            port,
            bind,
        }
    }

    /// Parse --home, --db, --port, --bind flags from CLI args.
    pub fn resolve_from_args(args: &[String]) -> Self {
        let home = Self::find_flag(args, "--home");
        let db = Self::find_flag(args, "--db");
        let port = Self::find_flag(args, "--port").and_then(|s| s.parse().ok());
        let bind = Self::find_flag(args, "--bind");
        Self::resolve_with_overrides(home.as_deref(), db.as_deref(), port, bind.as_deref())
    }

    fn find_flag(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }

    /// Serialize to JSON for `cortex paths --json`.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "home": self.home.display().to_string(),
            "db": self.db.display().to_string(),
            "token": self.token.display().to_string(),
            "pid": self.pid.display().to_string(),
            "port": self.port,
            "bind": &self.bind,
            "models": self.models.display().to_string(),
        })
        .to_string()
    }
}

fn normalize_bind(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_bind(bind_override: Option<&str>, env_bind: Option<&str>) -> String {
    bind_override
        .and_then(normalize_bind)
        .or_else(|| env_bind.and_then(normalize_bind))
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

// ---------------------------------------------------------------------------
// Legacy migration
// ---------------------------------------------------------------------------

/// Returns the legacy database path: `~/cortex/cortex.db`.
pub fn legacy_db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("cortex").join("cortex.db")
}

/// Migrate legacy DB from `~/cortex/cortex.db` to the canonical location.
/// Copies (never moves) to preserve the original as a safety net.
pub fn migrate_legacy_db(paths: &CortexPaths) -> Result<bool, String> {
    let legacy = legacy_db_path();
    if !legacy.exists() || paths.db.exists() {
        return Ok(false);
    }

    fs::create_dir_all(paths.db.parent().unwrap_or(&paths.home))
        .map_err(|e| format!("create dir: {e}"))?;

    fs::copy(&legacy, &paths.db).map_err(|e| format!("copy db: {e}"))?;

    // Copy WAL and SHM if present
    for ext in ["db-wal", "db-shm"] {
        let src = legacy.with_extension(ext);
        if src.exists() {
            let dst = paths.db.with_extension(ext);
            fs::copy(&src, &dst).map_err(|e| format!("copy {ext}: {e}"))?;
        }
    }

    // Verify integrity of the copy
    let conn =
        rusqlite::Connection::open(&paths.db).map_err(|e| format!("open migrated db: {e}"))?;
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|e| format!("integrity check: {e}"))?;
    if check != "ok" {
        // Remove the bad copy, leave legacy intact
        let _ = fs::remove_file(&paths.db);
        return Err(format!("integrity check failed on migrated db: {check}"));
    }

    eprintln!(
        "[cortex] Migrated brain from {} to {}",
        legacy.display(),
        paths.db.display()
    );
    Ok(true)
}

// ---------------------------------------------------------------------------
// Daemon lock
// ---------------------------------------------------------------------------

/// Acquire an exclusive file lock on `~/.cortex/cortex.lock`.
/// Returns the lock file handle (lock is held as long as the handle lives).
pub fn acquire_daemon_lock(paths: &CortexPaths) -> Result<fs::File, String> {
    fs::create_dir_all(&paths.home).map_err(|e| format!("create home: {e}"))?;
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&paths.lock)
        .map_err(|e| format!("open lock: {e}"))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| "another cortex instance holds the lock".to_string())?;
    Ok(lock_file)
}

/// Returns `~/.cortex` (or `$HOME/.cortex` on non-Windows).
pub fn cortex_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("CORTEX_HOME") {
        if !explicit.trim().is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(CORTEX_DIR_NAME)
}

/// Generate a fresh UUID token, write it to the resolved token path, and
/// return the token string.
pub fn generate_token_for(paths: &CortexPaths) -> String {
    let token = Uuid::new_v4().simple().to_string();
    if let Err(e) = fs::create_dir_all(paths.token.parent().unwrap_or(&paths.home)) {
        eprintln!(
            "[cortex] WARNING: cannot create {}: {e}",
            paths.token.parent().unwrap_or(&paths.home).display()
        );
    }
    if let Err(e) = fs::write(&paths.token, &token) {
        eprintln!("[cortex] WARNING: cannot write token: {e}");
    }
    token
}

/// Generate a fresh UUID token, write it to `~/.cortex/cortex.token`, and
/// return the token string.
pub fn generate_token() -> String {
    generate_token_for(&CortexPaths::resolve())
}

/// Read an existing token from the resolved token path.
pub fn read_token_from(paths: &CortexPaths) -> Option<String> {
    fs::read_to_string(&paths.token)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Read existing shared token from `~/.cortex/cortex.token`.
pub fn read_token() -> Option<String> {
    read_token_from(&CortexPaths::resolve())
}

/// Generate an in-memory token without mutating shared auth files.
pub fn generate_ephemeral_token() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Generate a `ctx_` API key:
/// - body: base62-encoded random bytes (43 chars)
/// - checksum: 16-bit FNV-1a over the body, base62 (3 chars, left-padded)
pub fn generate_ctx_api_key() -> String {
    let mut random = Vec::with_capacity(32);
    random.extend_from_slice(Uuid::new_v4().as_bytes());
    random.extend_from_slice(Uuid::new_v4().as_bytes());

    let mut body = base62_encode_bytes(&random);
    if body.len() < 43 {
        // Extremely unlikely, but keep a stable key shape.
        let extra = base62_encode_bytes(Uuid::new_v4().as_bytes());
        body.push_str(&extra);
    }
    body.truncate(43);

    let checksum_num = fnv1a16(body.as_bytes());
    let checksum = left_pad_base62(checksum_num, 3);

    format!("ctx_{body}{checksum}")
}

/// Hash an API key with Argon2id.
pub fn hash_api_key_argon2id(api_key: &str) -> Result<String, String> {
    let params = Params::new(64 * 1024, 3, 4, None).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes()).map_err(|e| e.to_string())?;
    argon2
        .hash_password(api_key.as_bytes(), &salt)
        .map(|p| p.to_string())
        .map_err(|e| e.to_string())
}

/// Verify a plaintext API key against an Argon2id hash.
pub fn verify_api_key_argon2id(api_key: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(v) => v,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(api_key.as_bytes(), &parsed)
        .is_ok()
}

/// Write the current process PID to `~/.cortex/cortex.pid`.
#[allow(dead_code)]
pub fn write_pid() {
    let dir = cortex_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[cortex] WARNING: cannot create {}: {e}", dir.display());
    }
    fs::write(dir.join("cortex.pid"), std::process::id().to_string()).ok();
}

/// Remove stale PID file when the recorded daemon process no longer exists.
pub fn cleanup_stale_pid_lock(paths: &CortexPaths) -> Option<u32> {
    let pid = stale_pid_candidate(paths)?;

    let _ = fs::remove_file(&paths.pid);
    eprintln!("[cortex] Cleaned stale PID file (process {pid} not running)");
    Some(pid)
}

pub fn stale_pid_candidate(paths: &CortexPaths) -> Option<u32> {
    if !paths.pid.exists() {
        return None;
    }

    let pid = fs::read_to_string(&paths.pid)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())?;

    if pid == std::process::id() || process_is_running(pid) {
        return None;
    }

    Some(pid)
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    use std::process::Command;

    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output();
    let Ok(out) = output else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.contains(&format!("\"{pid}\""))
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Returns the canonical database path: `~/.cortex/cortex.db`.
pub fn db_path() -> PathBuf {
    cortex_dir().join("cortex.db")
}

fn fnv1a16(input: &[u8]) -> u16 {
    let mut hash: u32 = 0x811C9DC5;
    for byte in input {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    (hash & 0xFFFF) as u16
}

fn left_pad_base62(num: u16, width: usize) -> String {
    let mut s = base62_encode_u64(num as u64);
    while s.len() < width {
        s.insert(0, '0');
    }
    s
}

fn base62_encode_u64(mut num: u64) -> String {
    if num == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while num > 0 {
        out.push(BASE62[(num % 62) as usize] as char);
        num /= 62;
    }
    out.iter().rev().collect()
}

fn base62_encode_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let mut digits: Vec<u8> = vec![0];
    for &byte in bytes {
        let mut carry = byte as u32;
        for digit in &mut digits {
            let value = (*digit as u32) * 256 + carry;
            *digit = (value % 62) as u8;
            carry = value / 62;
        }
        while carry > 0 {
            digits.push((carry % 62) as u8);
            carry /= 62;
        }
    }
    digits
        .iter()
        .rev()
        .map(|d| BASE62[*d as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_auth_{name}_{unique}"))
    }

    #[test]
    fn cleanup_stale_pid_lock_removes_dead_process_pid_only() {
        let home_dir = temp_test_dir("stale_pid");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, None, None);
        fs::write(&paths.pid, "999999").unwrap();
        fs::write(&paths.lock, "locked").unwrap();

        let cleaned = cleanup_stale_pid_lock(&paths);
        assert_eq!(cleaned, Some(999999));
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn token_helpers_respect_overridden_home() {
        let home_dir = temp_test_dir("token_home");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(54967), None);

        let token = generate_token_for(&paths);

        assert_eq!(read_token_from(&paths).as_deref(), Some(token.as_str()));
        assert_eq!(paths.token, home_dir.join("cortex.token"));
        assert!(paths.token.exists());
        assert_eq!(paths.bind, "127.0.0.1");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn resolve_bind_prefers_cli_then_env_then_default() {
        assert_eq!(resolve_bind(Some("0.0.0.0"), Some("10.10.0.5")), "0.0.0.0");
        assert_eq!(resolve_bind(Some("   "), Some("10.10.0.5")), "10.10.0.5");
        assert_eq!(resolve_bind(None, Some("   ")), "127.0.0.1");
    }

    #[test]
    fn resolve_from_args_parses_bind_flag() {
        let home_dir = temp_test_dir("bind_flag");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let args = vec![
            "cortex".to_string(),
            "serve".to_string(),
            "--home".to_string(),
            home_str,
            "--bind".to_string(),
            "0.0.0.0".to_string(),
        ];
        let paths = CortexPaths::resolve_from_args(&args);
        assert_eq!(paths.bind, "0.0.0.0");

        let _ = fs::remove_dir_all(&home_dir);
    }
}
