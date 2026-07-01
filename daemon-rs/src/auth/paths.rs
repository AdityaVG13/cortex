// SPDX-License-Identifier: MIT
use std::fs;
#[cfg(windows)]
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const CORTEX_DIR_NAME: &str = ".cortex";
pub(crate) const CORTEX_GLOBAL_LOCK_NAME: &str = "cortex.global.lock";
pub(crate) const CORTEX_GLOBAL_LOCK_HOME_ENV: &str = "CORTEX_GLOBAL_LOCK_HOME";
pub(crate) const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

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
            .unwrap_or_else(|| default_home_root().join(CORTEX_DIR_NAME));

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

pub(crate) fn default_home_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
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
