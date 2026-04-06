// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
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
    pub models: PathBuf,
    #[allow(dead_code)]
    pub write_buffer: PathBuf,
}

impl CortexPaths {
    /// Resolve paths from environment variables only (no CLI args).
    pub fn resolve() -> Self {
        Self::resolve_with_overrides(None, None, None)
    }

    /// Resolve paths with optional CLI overrides.
    pub fn resolve_with_overrides(
        home_override: Option<&str>,
        db_override: Option<&str>,
        port_override: Option<u16>,
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

        Self {
            token: home.join("cortex.token"),
            pid: home.join("cortex.pid"),
            lock: home.join("cortex.lock"),
            models: home.join("models"),
            write_buffer: home.join("write_buffer.jsonl"),
            home,
            db,
            port,
        }
    }

    /// Parse --home, --db, --port flags from CLI args.
    pub fn resolve_from_args(args: &[String]) -> Self {
        let home = Self::find_flag(args, "--home");
        let db = Self::find_flag(args, "--db");
        let port = Self::find_flag(args, "--port").and_then(|s| s.parse().ok());
        Self::resolve_with_overrides(home.as_deref(), db.as_deref(), port)
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
            "models": self.models.display().to_string(),
        })
        .to_string()
    }
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

    fs::copy(&legacy, &paths.db)
        .map_err(|e| format!("copy db: {e}"))?;

    // Copy WAL and SHM if present
    for ext in ["db-wal", "db-shm"] {
        let src = legacy.with_extension(ext);
        if src.exists() {
            let dst = paths.db.with_extension(ext);
            fs::copy(&src, &dst).map_err(|e| format!("copy {ext}: {e}"))?;
        }
    }

    // Verify integrity of the copy
    let conn = rusqlite::Connection::open(&paths.db)
        .map_err(|e| format!("open migrated db: {e}"))?;
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
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(CORTEX_DIR_NAME)
}

/// Generate a fresh UUID token, write it to `~/.cortex/cortex.token`, and
/// return the token string.
pub fn generate_token() -> String {
    let token = Uuid::new_v4().simple().to_string();
    let dir = cortex_dir();
    fs::create_dir_all(&dir).ok();
    fs::write(dir.join("cortex.token"), &token).ok();
    token
}

/// Read existing shared token from `~/.cortex/cortex.token`.
pub fn read_token() -> Option<String> {
    fs::read_to_string(cortex_dir().join("cortex.token"))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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
    fs::create_dir_all(&dir).ok();
    fs::write(dir.join("cortex.pid"), std::process::id().to_string()).ok();
}

/// Kill a stale daemon process if PID file exists and process is still alive.
pub fn kill_stale_daemon() {
    let pid_path = cortex_dir().join("cortex.pid");
    if !pid_path.exists() {
        return;
    }

    if let Ok(pid_str) = fs::read_to_string(&pid_path) {
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            // Don't kill ourselves.
            if pid == std::process::id() {
                return;
            }

            #[cfg(windows)]
            {
                use std::process::Command;
                if !pid_looks_like_cortex(pid) {
                    let _ = fs::remove_file(&pid_path);
                    return;
                }
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }

            #[cfg(unix)]
            {
                if !pid_looks_like_cortex(pid) {
                    let _ = fs::remove_file(&pid_path);
                    return;
                }
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }

            eprintln!("[cortex] Killed stale daemon (PID {pid})");
            let _ = fs::remove_file(&pid_path);

            // Brief pause for port release.
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

#[cfg(windows)]
fn pid_looks_like_cortex(pid: u32) -> bool {
    use std::process::Command;

    let query =
        format!("(Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\").CommandLine");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", &query])
        .output();
    let Ok(out) = output else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let cmd = String::from_utf8_lossy(&out.stdout).to_lowercase();
    cmd.contains("cortex")
}

#[cfg(unix)]
fn pid_looks_like_cortex(pid: u32) -> bool {
    let path = format!("/proc/{pid}/cmdline");
    let Ok(raw) = fs::read(path) else {
        return false;
    };
    let cmd = String::from_utf8_lossy(&raw).to_lowercase();
    cmd.contains("cortex")
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

