use std::fs;
#[cfg(windows)]
use std::io;
use std::path::{Path, PathBuf};
pub const CORTEX_DIR_NAME: &str = ".cortex";
pub const CORTEX_GLOBAL_LOCK_NAME: &str = "cortex.global.lock";
pub const CORTEX_GLOBAL_LOCK_HOME_ENV: &str = "CORTEX_GLOBAL_LOCK_HOME";
pub const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
#[derive(Debug, Clone)]
pub struct CortexPaths {
    pub home: PathBuf,
    pub db: PathBuf,
    pub token: PathBuf,
    pub pid: PathBuf,
    pub lock: PathBuf,
    #[allow(dead_code)]
    pub write_buffer: PathBuf,
}
impl CortexPaths {
    pub fn resolve() -> Self {
        Self::resolve_with_overrides(None, None)
    }
    pub fn resolve_with_overrides(home_override: Option<&str>, db_override: Option<&str>) -> Self {
        let home = home_override
            .map(PathBuf::from)
            .or_else(|| std::env::var("CORTEX_HOME").ok().map(PathBuf::from))
            .unwrap_or_else(|| default_home_root().join(CORTEX_DIR_NAME));
        let db = db_override
            .map(PathBuf::from)
            .or_else(|| std::env::var("CORTEX_DB").ok().map(PathBuf::from))
            .unwrap_or_else(|| home.join("cortex.db"));
        Self {
            token: home.join("cortex.token"),
            pid: home.join("cortex.pid"),
            lock: home.join("cortex.lock"),
            write_buffer: home.join("write_buffer.jsonl"),
            home,
            db,
        }
    }
    pub fn resolve_from_args(args: &[String]) -> Self {
        let home = Self::find_flag(args, "--home");
        let db = Self::find_flag(args, "--db");
        Self::resolve_with_overrides(home.as_deref(), db.as_deref())
    }
    fn find_flag(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    }
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "home": self.home.display().to_string(),
            "db": self.db.display().to_string(),
            "token": self.token.display().to_string(),
            "pid": self.pid.display().to_string(),
        })
        .to_string()
    }
    /// Operator-owned live capture sidecar. `CORTEX_CAPTURE` overrides this file.
    pub fn capture_sidecar(&self) -> PathBuf {
        self.home.join("capture.json")
    }
}
pub fn default_home_root() -> PathBuf {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
#[cfg(unix)]
pub fn restrict_file_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}
// K1-K9 cluster: (A) STRICTLY_UNAVOIDABLE. Owner-only DACL on a secret file
// is a Win32 token/ACL FFI surface. Unavoidable BECAUSE HANDLE / PSID / ACL
// pointers and the C ABI are outside Rust's type system (Nomicon § FFI
// https://doc.rust-lang.org/nomicon/ffi.html ; Reference § External blocks
// https://doc.rust-lang.org/reference/items/external-blocks.html ; canonical
// unavoidable §2: nix/rustix/windows crates bottom out in the same unsafe).
// Cluster alternatives FAIL: (1) std::fs::Permissions -- Windows std has no
// owner-SID DACL; (2) skip ACL -- not equivalent to unix 0o600
// restrict_file_to_owner; (3) windows-acl / `windows` crate -- convenience
// wrap of the same FFI, not a safe language form. A later Windows-wrap
// may cluster these into one safe facade; the inner calls stay (A).
#[cfg(windows)]
struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY (K1, A): `self.0` is a process-token HANDLE this wrapper
            // obtained from OpenProcessToken and owns exclusively. Non-null
            // was checked. CloseHandle is the documented destructor; Drop
            // cannot surface the BOOL, so the result is discarded.
            // Unavoidable BECAUSE HANDLE close is Win32 FFI (Nomicon § FFI).
            // Alternatives FAIL: (1) leak -- handle exhaustion, not equivalent;
            // (2) std OwnedHandle::from_raw_handle -- still unsafe, same
            // obligation; (3) skip Drop -- leaks the token opened at K3.
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
            // SAFETY (K2, A): `self.0` is the ACL pointer SetEntriesInAclW
            // returned. Win32 allocates that ACL with LocalAlloc and requires
            // LocalFree. Non-null was checked; we own the pointer.
            // Unavoidable BECAUSE LocalAlloc heap identity is not the Rust
            // allocator (Nomicon § FFI).
            // Alternatives FAIL: (1) leak -- LocalAlloc leak, not equivalent;
            // (2) Box/Vec drop -- wrong heap, UB; (3) GlobalFree/HeapFree --
            // wrong Win32 heap, UB.
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
    // SAFETY (K3, A): GetCurrentProcess returns a pseudo-handle valid for
    // this process for the duration of the call. `token` is a writable
    // out-param on our stack. TOKEN_QUERY is a documented access mask.
    // Unavoidable BECAUSE process-token open is Win32 FFI with no std API
    // (Nomicon § FFI; Reference § External blocks).
    // Alternatives FAIL: (1) std::process -- no token/SID API; (2) whoami /
    // windows-acl crates -- same OpenProcessToken FFI; (3) well-known
    // Everyone SID -- weaker than owner-only, not equivalent to 0o600.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);
    let mut required_len = 0u32;
    // SAFETY (K4, A): `token.0` is an open token we own. A null buffer with
    // length 0 is the documented size query; Windows writes `required_len`
    // and returns a length error we ignore.
    // Unavoidable BECAUSE token-info size query is Win32 FFI (Nomicon § FFI).
    // Alternatives FAIL: (1) guess TOKEN_USER size -- SID payload is
    // variable-length, under-alloc is a write overflow; (2) skip query and
    // use a huge stack buffer -- still the same FFI write; (3) crate-wrapped
    // GetTokenInformation -- same extern-C call.
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
    // SAFETY (K5, A): `token_info` is a heap Vec whose byte length is at
    // least `required_len`. The exclusive pointer is valid for that many
    // bytes; Win32 writes the buffer before we read it.
    // Unavoidable BECAUSE filling TOKEN_USER is Win32 FFI into caller
    // memory (Nomicon § FFI; Reference § Behavior considered undefined
    // https://doc.rust-lang.org/reference/behavior-considered-undefined.html).
    // Alternatives FAIL: (1) read SID from env/whoami text -- not the
    // process token, wrong principal; (2) MaybeUninit without FFI -- no
    // bytes to init; (3) crate-wrapped GetTokenInformation -- same FFI write.
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
    // SAFETY (K6, A): GetTokenInformation succeeded and `returned_len` is
    // at least size_of::<TOKEN_USER>(). The buffer is usize-aligned and
    // stays owned by `token_info` for the rest of this function, so the
    // SID pointer derived from this copy remains in-bounds.
    // Unavoidable BECAUSE interpreting a C-written TOKEN_USER is a raw
    // pointer read of a #[repr(C)] FFI struct (Reference § Pointer types
    // https://doc.rust-lang.org/reference/types/pointer.html).
    // Alternatives FAIL: (1) bytemuck/zerocopy -- TOKEN_USER contains a
    // PSID pointer, not Pod; (2) byte-parse without deref -- still trusts
    // the same C layout via raw reads; (3) transmute -- still unsafe, no
    // safer.
    let token_user = unsafe { *token_info.as_ptr().cast::<TOKEN_USER>() };
    if token_user.User.Sid.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is missing",
        ));
    }
    // SAFETY (K7, A): `token_user.User.Sid` was checked non-null and
    // addresses bytes inside the still-live `token_info` allocation.
    // IsValidSid only reads.
    // Unavoidable BECAUSE SID validation is Win32 FFI (Nomicon § FFI).
    // Alternatives FAIL: (1) skip IsValidSid -- would feed a garbage SID
    // to SetEntriesInAclW; (2) hand-rolled SID parser -- reimplements
    // Win32 SID layout and still reads FFI memory; (3) crate-wrapped
    // IsValidSid -- same extern-C call.
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
pub fn restrict_file_to_owner(path: &Path) -> io::Result<()> {
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
    // SAFETY (K8, A): `access` is a fully initialized EXPLICIT_ACCESS_W on
    // the stack; its trustee SID points into `current_user`, which is still
    // alive. `acl` is a writable out-param. The old ACL is null (no merge).
    // Unavoidable BECAUSE ACL construction is Win32 FFI that returns a
    // LocalAlloc pointer (Nomicon § FFI).
    // Alternatives FAIL: (1) std fs permissions -- no DACL builder;
    // (2) hand-written SECURITY_DESCRIPTOR bytes -- still SetNamedSecurityInfo
    // FFI plus layout UB; (3) windows-acl crate -- same FFI.
    let result = unsafe { SetEntriesInAclW(1, &access, null(), &mut acl) };
    if result != ERROR_SUCCESS {
        return Err(win32_error(result));
    }
    let _acl_guard = LocalMemory(acl.cast());
    let wide_path = windows_path_to_wide(path);
    // SAFETY (K9, A): `wide_path` is a NUL-terminated UTF-16 path. `acl` is
    // the ACL we just created and still own via `_acl_guard`. Owner/group/
    // SACL pointers are null, which Win32 allows when those security-info
    // bits are unset.
    // Unavoidable BECAUSE applying a DACL is Win32 FFI (Nomicon § FFI).
    // Alternatives FAIL: (1) std::fs::set_permissions -- cannot express a
    // PROTECTED owner-only DACL; (2) icacls via Command -- locale/PATH
    // fragile, not a language form; (3) skip -- unix path stays 0o600,
    // Windows would leave the secret world-readable.
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
pub fn restrict_file_to_owner(_path: &Path) -> std::io::Result<()> {
    Ok(())
}
pub fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
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
        Ok(())
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
