// SPDX-License-Identifier: MIT
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use fs2::FileExt;
use std::fs;
#[cfg(windows)]
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const CORTEX_DIR_NAME: &str = ".cortex";
const CORTEX_GLOBAL_LOCK_NAME: &str = "cortex.global.lock";
const CORTEX_GLOBAL_LOCK_HOME_ENV: &str = "CORTEX_GLOBAL_LOCK_HOME";
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
    pub ipc_endpoint: Option<String>,
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
            .unwrap_or(crate::DEFAULT_CORTEX_PORT);
        let env_bind = std::env::var("CORTEX_BIND").ok();
        let bind = resolve_bind(bind_override, env_bind.as_deref());
        let ipc_endpoint = resolve_ipc_endpoint(&home, port);

        Self {
            token: home.join("cortex.token"),
            pid: home.join("cortex.pid"),
            lock: home.join("cortex.lock"),
            ipc_endpoint,
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
            "ipc_endpoint": self.ipc_endpoint.clone(),
            "ipc_kind": if self.ipc_endpoint.is_some() {
                Some(default_ipc_kind())
            } else {
                None
            },
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

fn default_ipc_kind() -> &'static str {
    if cfg!(windows) {
        "named-pipe"
    } else {
        "unix-socket"
    }
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn resolve_ipc_endpoint(home: &std::path::Path, port: u16) -> Option<String> {
    if env_truthy("CORTEX_DISABLE_IPC") {
        return None;
    }

    if let Ok(raw) = std::env::var("CORTEX_IPC_ENDPOINT") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if cfg!(windows) {
        return Some(format!(r"\\.\pipe\cortex-daemon-{port}"));
    }

    let socket = home.join("runtime").join(format!("cortexd-{port}.sock"));
    Some(socket.display().to_string())
}

#[cfg(unix)]
pub(crate) fn restrict_file_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(windows)]
struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this guard owns the token handle returned by OpenProcessToken.
            unsafe {
                let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct LocalMemory(*mut std::ffi::c_void);

#[cfg(windows)]
impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this guard owns memory allocated by a Win32 local allocator.
            unsafe {
                let _ = windows_sys::Win32::Foundation::LocalFree(self.0);
            }
        }
    }
}

#[cfg(windows)]
struct CurrentUserSid {
    _token_info: Vec<usize>,
    sid: windows_sys::Win32::Security::PSID,
}

#[cfg(windows)]
fn windows_path_to_wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn win32_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

#[cfg(windows)]
fn current_user_sid() -> io::Result<CurrentUserSid> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Security::{
        GetTokenInformation, IsValidSid, TokenUser, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a pseudo-handle for this process, and
    // OpenProcessToken initializes `token` on success.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);

    let mut required_len = 0u32;
    // SAFETY: this size query intentionally passes a null output buffer and
    // zero length so Windows reports the required TOKEN_USER buffer size.
    unsafe {
        let _ = GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required_len);
    }
    if required_len == 0 {
        return Err(io::Error::last_os_error());
    }

    let word_size = std::mem::size_of::<usize>();
    let word_count = (required_len as usize).div_ceil(word_size);
    let mut token_info = vec![0usize; word_count];
    let mut returned_len = 0u32;
    // SAFETY: `token_info` is a writable, word-aligned buffer large enough for
    // the TOKEN_USER data size returned by the previous GetTokenInformation call.
    let filled = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            token_info.as_mut_ptr().cast(),
            (token_info.len() * word_size) as u32,
            &mut returned_len,
        )
    };
    if filled == 0 {
        return Err(io::Error::last_os_error());
    }
    if returned_len < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user information is too small",
        ));
    }

    // SAFETY: the buffer was populated by GetTokenInformation(TokenUser) and
    // is word-aligned, so reading the leading TOKEN_USER record is valid.
    let token_user = unsafe { *token_info.as_ptr().cast::<TOKEN_USER>() };
    if token_user.User.Sid.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is missing",
        ));
    }
    // SAFETY: token_user.User.Sid came from the validated TOKEN_USER buffer.
    if unsafe { IsValidSid(token_user.User.Sid) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is invalid",
        ));
    }

    Ok(CurrentUserSid {
        _token_info: token_info,
        sid: token_user.User.Sid,
    })
}

#[cfg(windows)]
pub(crate) fn restrict_file_to_owner(path: &Path) -> io::Result<()> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    let current_user = current_user_sid()?;
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: current_user.sid.cast(),
        },
    };
    let mut acl: *mut ACL = null_mut();
    // SAFETY: `access` references the current-user SID buffer, which remains
    // alive for this call; `acl` receives LocalFree-owned memory on success.
    let result = unsafe { SetEntriesInAclW(1, &access, null(), &mut acl) };
    if result != ERROR_SUCCESS {
        return Err(win32_error(result));
    }
    let _acl_guard = LocalMemory(acl.cast());

    let wide_path = windows_path_to_wide(path);
    // SAFETY: `wide_path` is null-terminated, `acl` is a valid ACL produced by
    // SetEntriesInAclW, and null owner/group/SACL preserve those fields.
    let result = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(win32_error(result));
    }

    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn restrict_file_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub(crate) fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents)?;
        file.flush()?;
        restrict_file_to_owner(path)?;
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        use std::io::Write as _;

        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        restrict_file_to_owner(path)
            .and_then(|_| file.write_all(contents))
            .and_then(|_| file.flush())
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
    let busy_timeout_ms = crate::db::SQLITE_BUSY_TIMEOUT_MS;
    conn.execute_batch(&format!("PRAGMA busy_timeout = {busy_timeout_ms};"))
        .map_err(|e| format!("configure migrated db busy timeout: {e}"))?;
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

fn default_home_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn global_lock_path() -> PathBuf {
    if let Ok(explicit) = std::env::var(CORTEX_GLOBAL_LOCK_HOME_ENV) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join(CORTEX_GLOBAL_LOCK_NAME);
        }
    }
    default_home_root()
        .join(CORTEX_DIR_NAME)
        .join(CORTEX_GLOBAL_LOCK_NAME)
}

/// Acquire an exclusive global daemon lock to enforce one active Cortex daemon
/// per user, even when different homes/binaries are involved.
fn acquire_global_daemon_lock_at(path: &Path) -> Result<fs::File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create global lock dir: {e}"))?;
    }
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("open global lock: {e}"))?;
    lock_file
        .try_lock_exclusive()
        .map_err(|_| "another cortex instance holds the lock".to_string())?;
    Ok(lock_file)
}

pub fn acquire_global_daemon_lock() -> Result<fs::File, String> {
    acquire_global_daemon_lock_at(&global_lock_path())
}

/// Returns `~/.cortex` (or `$HOME/.cortex` on non-Windows).
pub fn cortex_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("CORTEX_HOME") {
        if !explicit.trim().is_empty() {
            return PathBuf::from(explicit);
        }
    }
    default_home_root().join(CORTEX_DIR_NAME)
}

/// Generate a fresh UUID token, write it to the resolved token path, and
/// return the token string.
pub fn try_generate_token_for(paths: &CortexPaths) -> Result<String, String> {
    let token = Uuid::new_v4().simple().to_string();
    try_write_token_for(paths, &token)?;
    Ok(token)
}

/// Write a shared auth token to the resolved token path.
pub fn try_write_token_for(paths: &CortexPaths, token: &str) -> Result<(), String> {
    let token_dir = paths.token.parent().unwrap_or(&paths.home);
    fs::create_dir_all(token_dir)
        .map_err(|e| format!("cannot create token directory {}: {e}", token_dir.display()))?;
    write_secret_file(&paths.token, token.as_bytes())
        .map_err(|e| format!("cannot write token file {}: {e}", paths.token.display()))?;
    Ok(())
}

/// Generate a fresh UUID token for the resolved token path.
pub fn try_generate_token() -> Result<String, String> {
    try_generate_token_for(&CortexPaths::resolve())
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

const CTX_KEY_BODY_LEN: usize = 43;
const CTX_KEY_CHECKSUM_LEN: usize = 3;

/// Cheap structural validation for `ctx_` API keys before Argon2 verification.
pub fn verify_ctx_api_key_checksum(candidate: &str) -> bool {
    if !candidate.starts_with("ctx_") {
        return false;
    }
    let payload = &candidate[4..];
    if payload.len() != CTX_KEY_BODY_LEN + CTX_KEY_CHECKSUM_LEN {
        return false;
    }
    if !payload
        .as_bytes()
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric())
    {
        return false;
    }

    let (body, checksum) = payload.split_at(CTX_KEY_BODY_LEN);
    let expected = left_pad_base62(fnv1a16(body.as_bytes()), CTX_KEY_CHECKSUM_LEN);
    constant_time_eq(checksum, expected.as_str())
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut diff = a.len() ^ b.len();
    let max_len = a.len().max(b.len());

    for idx in 0..max_len {
        let left = a.get(idx).copied().unwrap_or(0);
        let right = b.get(idx).copied().unwrap_or(0);
        diff |= usize::from(left ^ right);
    }

    diff == 0
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
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: `pid` has been range-checked for the platform `pid_t`.
    // Passing signal 0 performs an existence/permission probe and does not
    // deliver a signal.
    unsafe { libc::kill(pid, 0) == 0 }
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
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const LOCK_CHILD_MODE_ENV: &str = "CORTEX_LOCK_TEST_CHILD_MODE";
    const LOCK_CHILD_HOME_ENV: &str = "CORTEX_LOCK_TEST_CHILD_HOME";
    const LOCK_CHILD_READY_ENV: &str = "CORTEX_LOCK_TEST_CHILD_READY_FILE";
    const LOCK_CHILD_HOLD_MS_ENV: &str = "CORTEX_LOCK_TEST_CHILD_HOLD_MS";

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_auth_{name}_{unique}"))
    }

    #[test]
    fn verify_ctx_api_key_checksum_accepts_generated_keys() {
        let key = generate_ctx_api_key();
        assert!(verify_ctx_api_key_checksum(&key));
        assert!(!verify_ctx_api_key_checksum("ctx_short"));
        assert!(!verify_ctx_api_key_checksum(&format!("ctx_{}", "A".repeat(46))));
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
    fn cleanup_stale_pid_lock_removes_pid_outside_platform_range() {
        let home_dir = temp_test_dir("stale_pid_large");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, None, None);
        let stale_pid = u32::MAX;
        fs::write(&paths.pid, stale_pid.to_string()).unwrap();
        fs::write(&paths.lock, "locked").unwrap();

        let cleaned = cleanup_stale_pid_lock(&paths);
        assert_eq!(cleaned, Some(stale_pid));
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn token_helpers_respect_overridden_home() {
        let home_dir = temp_test_dir("token_home");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let token = try_generate_token_for(&paths).expect("token generation should succeed");

        assert_eq!(read_token_from(&paths).as_deref(), Some(token.as_str()));
        assert_eq!(paths.token, home_dir.join("cortex.token"));
        assert!(paths.token.exists());
        assert_eq!(paths.bind, "127.0.0.1");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[cfg(unix)]
    #[test]
    fn generated_token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let home_dir = temp_test_dir("token_permissions");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let _ = try_generate_token_for(&paths).expect("token generation should succeed");

        let mode = fs::metadata(&paths.token).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[cfg(windows)]
    #[test]
    fn generated_token_file_has_protected_owner_acl() {
        use std::ptr::null_mut;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::Security::Authorization::{
            GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
            SE_FILE_OBJECT, TRUSTEE_IS_SID,
        };
        use windows_sys::Win32::Security::{
            EqualSid, GetSecurityDescriptorControl, ACL, DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        };

        let home_dir = temp_test_dir("token_windows_acl");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let _ = try_generate_token_for(&paths).expect("token generation should succeed");

        let wide_path = windows_path_to_wide(&paths.token);
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `wide_path` is null-terminated and output pointers are valid
        // for GetNamedSecurityInfoW to initialize.
        let result = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(
            result,
            ERROR_SUCCESS,
            "GetNamedSecurityInfoW failed: {}",
            win32_error(result)
        );
        let _descriptor_guard = LocalMemory(descriptor.cast());
        assert!(!dacl.is_null(), "secret file DACL should be present");

        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `descriptor` is owned by `_descriptor_guard` and remains
        // valid until the guard is dropped.
        let ok = unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) };
        assert_ne!(
            ok,
            0,
            "GetSecurityDescriptorControl failed: {}",
            io::Error::last_os_error()
        );
        assert_eq!(control & SE_DACL_PROTECTED, SE_DACL_PROTECTED);

        let mut count = 0u32;
        let mut entries: *mut EXPLICIT_ACCESS_W = null_mut();
        // SAFETY: `dacl` points into the security descriptor returned above and
        // remains valid while `_descriptor_guard` is alive.
        let result = unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) };
        assert_eq!(
            result,
            ERROR_SUCCESS,
            "GetExplicitEntriesFromAclW failed: {}",
            win32_error(result)
        );
        let _entries_guard = LocalMemory(entries.cast());
        assert_eq!(count, 1, "secret file should have one explicit ACE");

        // SAFETY: GetExplicitEntriesFromAclW returned at least one entry.
        let entry = unsafe { *entries };
        assert_eq!(entry.grfAccessMode, GRANT_ACCESS);
        assert_eq!(entry.Trustee.TrusteeForm, TRUSTEE_IS_SID);
        let expected_access = FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE;
        assert_eq!(
            entry.grfAccessPermissions & expected_access,
            expected_access
        );

        let current_user = current_user_sid().expect("read current user SID");
        let trustee_sid: PSID = entry.Trustee.ptstrName.cast();
        assert!(!trustee_sid.is_null(), "trustee SID should be present");
        // SAFETY: both SIDs come from Windows security APIs and were validated
        // before this comparison.
        assert_ne!(unsafe { EqualSid(trustee_sid, current_user.sid) }, 0);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn try_generate_token_for_reports_directory_failures() {
        let home_dir = temp_test_dir("token_home_is_file");
        if let Some(parent) = home_dir.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&home_dir, "not a directory").unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let err = try_generate_token_for(&paths).expect_err("token generation should fail");
        assert!(
            err.contains("cannot create token directory"),
            "unexpected error: {err}"
        );
        assert!(!paths.token.exists());

        let _ = fs::remove_file(&home_dir);
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

    #[test]
    fn acquire_global_daemon_lock_rejects_duplicate_instances() {
        let _guard = env_guard();
        let global_home = temp_test_dir("global_lock");
        fs::create_dir_all(&global_home).unwrap();
        let lock_path = global_home.join(CORTEX_GLOBAL_LOCK_NAME);

        let first =
            acquire_global_daemon_lock_at(&lock_path).expect("first global lock should succeed");
        let err =
            acquire_global_daemon_lock_at(&lock_path).expect_err("second global lock should fail");
        assert!(err.contains("another cortex instance"));

        drop(first);
        let second = acquire_global_daemon_lock_at(&lock_path)
            .expect("lock should be reacquired after release");
        drop(second);

        let _ = fs::remove_dir_all(&global_home);
    }

    #[test]
    fn acquire_global_daemon_lock_cross_process_child() {
        if std::env::var(LOCK_CHILD_MODE_ENV).ok().as_deref() != Some("1") {
            return;
        }

        let global_home = std::env::var(LOCK_CHILD_HOME_ENV).expect("child global lock home env");
        let lock_path = PathBuf::from(global_home).join(CORTEX_GLOBAL_LOCK_NAME);
        let ready_file =
            PathBuf::from(std::env::var(LOCK_CHILD_READY_ENV).expect("child ready file env"));
        let hold_ms = std::env::var(LOCK_CHILD_HOLD_MS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1500);

        let lock = acquire_global_daemon_lock_at(&lock_path).expect("child acquires global lock");
        fs::write(&ready_file, b"locked").expect("write ready marker");
        std::thread::sleep(Duration::from_millis(hold_ms));
        drop(lock);
    }

    #[test]
    fn acquire_global_daemon_lock_rejects_cross_process_duplicate_instances() {
        let _guard = env_guard();
        let global_home = temp_test_dir("global_lock_cross_process");
        fs::create_dir_all(&global_home).unwrap();
        let global_home_str = global_home.to_string_lossy().to_string();
        let lock_path = global_home.join(CORTEX_GLOBAL_LOCK_NAME);
        let ready_file = global_home.join("cross-process-ready");
        let hold_ms = 2000_u64;

        let current_exe = std::env::current_exe().expect("resolve current test binary path");
        let mut child = Command::new(current_exe)
            .arg("--exact")
            .arg("auth::tests::acquire_global_daemon_lock_cross_process_child")
            .arg("--nocapture")
            .env(LOCK_CHILD_MODE_ENV, "1")
            .env(LOCK_CHILD_HOME_ENV, &global_home_str)
            .env(
                LOCK_CHILD_READY_ENV,
                ready_file.to_string_lossy().to_string(),
            )
            .env(LOCK_CHILD_HOLD_MS_ENV, hold_ms.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn lock-holder child");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_file.exists() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child lock helper never reported readiness");
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let duplicate = acquire_global_daemon_lock_at(&lock_path)
            .expect_err("cross-process duplicate must fail");
        assert!(duplicate.contains("another cortex instance"));

        let status = child.wait().expect("wait on lock-holder child");
        assert!(status.success(), "child process should exit successfully");

        let after_release = acquire_global_daemon_lock_at(&lock_path)
            .expect("lock should succeed after child exit");
        drop(after_release);

        let _ = fs::remove_dir_all(&global_home);
    }
}
