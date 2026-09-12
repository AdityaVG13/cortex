use std::fs;
#[cfg(windows)]
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
static PROCESS_PATHS: OnceLock<CortexPaths> = OnceLock::new();
impl CortexPaths {
    /// Pin `--home` / `--db` (or the env resolve) for later `resolve()` callers.
    /// Does not mutate the process environment.
    pub fn install_process_paths(paths: &Self) {
        let _ = PROCESS_PATHS.set(paths.clone());
    }
    pub(crate) fn process_paths() -> Option<&'static Self> {
        PROCESS_PATHS.get()
    }
    /// After [`Self::install_process_paths`], returns that cell. Otherwise
    /// reads operator `CORTEX_HOME` / `CORTEX_DB`.
    pub fn resolve() -> Self {
        if let Some(paths) = PROCESS_PATHS.get() {
            return paths.clone();
        }
        Self::resolve_with_overrides(None, None)
    }
    /// Set `CORTEX_HOME` / `CORTEX_DB` on a child only (safe `Command::env`).
    pub fn apply_to_command(&self, command: &mut std::process::Command) {
        command.env("CORTEX_HOME", &self.home);
        command.env("CORTEX_DB", &self.db);
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
// wrap of the same FFI, not a safe language form.
// Safe facade: `restrict_file_to_owner` is the only crate-visible entry and
// holds no raw HANDLE / ACL. OwnedHandle / LocalMemory Drop are the only
// CloseHandle / LocalFree sites. Inner K1-K9 calls stay (A).
#[cfg(windows)]
fn handle_is_closeable(handle: windows_sys::Win32::Foundation::HANDLE) -> bool {
    !handle.is_null() && handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE
}
#[cfg(windows)]
struct OwnedHandle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl OwnedHandle {
    fn adopt(handle: windows_sys::Win32::Foundation::HANDLE) -> Self {
        Self(handle)
    }
    fn take(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<Self> {
        if !handle_is_closeable(handle) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows returned an invalid process token handle",
            ));
        }
        Ok(Self(handle))
    }
    fn as_raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0
    }
}
#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let handle = std::mem::replace(&mut self.0, std::ptr::null_mut());
        if !handle_is_closeable(handle) {
            return;
        }
        // SAFETY (K1, A): `handle` is a process-token HANDLE this wrapper
        // obtained from OpenProcessToken and owns exclusively. Null and
        // INVALID_HANDLE_VALUE were rejected. CloseHandle is the documented
        // destructor; Drop cannot surface the BOOL, so the result is discarded.
        // The field is nulled first so a second Drop cannot CloseHandle twice.
        // Unavoidable BECAUSE HANDLE close is Win32 FFI (Nomicon § FFI).
        // Alternatives FAIL: (1) leak -- handle exhaustion, not equivalent;
        // (2) std OwnedHandle::from_raw_handle -- still unsafe, same
        // obligation; (3) skip Drop -- leaks the token opened at K3.
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(handle);
        }
    }
}
#[cfg(windows)]
struct LocalMemory(*mut std::ffi::c_void);
#[cfg(windows)]
impl LocalMemory {
    fn adopt(ptr: *mut std::ffi::c_void) -> Self {
        Self(ptr)
    }
    fn is_null(&self) -> bool {
        self.0.is_null()
    }
    fn as_acl(&self) -> *mut windows_sys::Win32::Security::ACL {
        self.0.cast()
    }
}
#[cfg(windows)]
impl Drop for LocalMemory {
    fn drop(&mut self) {
        let ptr = std::mem::replace(&mut self.0, std::ptr::null_mut());
        if ptr.is_null() {
            return;
        }
        // SAFETY (K2, A): `ptr` is the ACL pointer SetEntriesInAclW
        // returned. Win32 allocates that ACL with LocalAlloc and requires
        // LocalFree. Non-null was checked; we own the pointer. The field
        // is nulled first so a second Drop cannot LocalFree twice.
        // Unavoidable BECAUSE LocalAlloc heap identity is not the Rust
        // allocator (Nomicon § FFI).
        // Alternatives FAIL: (1) leak -- LocalAlloc leak, not equivalent;
        // (2) Box/Vec drop -- wrong heap, UB; (3) GlobalFree/HeapFree --
        // wrong Win32 heap, UB.
        unsafe {
            let _ = windows_sys::Win32::Foundation::LocalFree(ptr);
        }
    }
}
#[cfg(windows)]
struct CurrentUserSid {
    _token_info: Vec<usize>,
    sid: std::ptr::NonNull<std::ffi::c_void>,
}
#[cfg(windows)]
fn windows_path_to_wide(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path contains an interior NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}
#[cfg(windows)]
fn win32_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}
/// Little-endian byte from a `Vec<usize>` token buffer. Win32 writes the
/// in-memory SID as bytes; Windows hosts are little-endian.
#[cfg(windows)]
fn token_info_le_byte(words: &[usize], offset: usize) -> Option<u8> {
    let word_size = std::mem::size_of::<usize>();
    let word = *words.get(offset / word_size)?;
    let shift = (offset % word_size) * 8;
    Some(((word >> shift) & 0xff) as u8)
}
#[cfg(windows)]
fn current_user_sid() -> io::Result<CurrentUserSid> {
    use std::ptr::{null_mut, NonNull};
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
        let err = io::Error::last_os_error();
        drop(OwnedHandle::adopt(token));
        return Err(err);
    }
    let token = OwnedHandle::take(token)?;
    let mut required_len = 0u32;
    // SAFETY (K4, A): `token.as_raw()` is an open token we own. A null
    // buffer with length 0 is the documented size query; Windows writes
    // `required_len` and returns a length error we ignore.
    // Unavoidable BECAUSE token-info size query is Win32 FFI (Nomicon § FFI).
    // Alternatives FAIL: (1) guess TOKEN_USER size -- SID payload is
    // variable-length, under-alloc is a write overflow; (2) skip query and
    // use a huge stack buffer -- still the same FFI write; (3) crate-wrapped
    // GetTokenInformation -- same extern-C call.
    unsafe {
        let _ = GetTokenInformation(token.as_raw(), TokenUser, null_mut(), 0, &mut required_len);
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
            token.as_raw(),
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
    // SID pointer derived from this copy remains in-bounds after the
    // post-read range check below.
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
    let init_len = std::cmp::min(returned_len as usize, token_info.len() * word_size);
    let buf_start = token_info.as_ptr() as usize;
    let buf_end = buf_start.saturating_add(init_len);
    let sid_addr = token_user.User.Sid as usize;
    if sid_addr < buf_start || sid_addr >= buf_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is outside the token buffer",
        ));
    }
    let sid_offset = sid_addr - buf_start;
    const SID_FIXED: usize = 8;
    const SID_MAX_SUB_AUTHORITIES: u8 = 15;
    if sid_offset.saturating_add(SID_FIXED) > init_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID header is truncated",
        ));
    }
    let Some(sub_count) = token_info_le_byte(&token_info, sid_offset + 1) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID header is truncated",
        ));
    };
    if sub_count > SID_MAX_SUB_AUTHORITIES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID has too many sub-authorities",
        ));
    }
    let sid_len = SID_FIXED + (sub_count as usize) * 4;
    if sid_offset.saturating_add(sid_len) > init_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID extends past the token buffer",
        ));
    }
    // SAFETY (K7, A): `token_user.User.Sid` was checked non-null, in the
    // initialized prefix, and the SID byte length implied by
    // SubAuthorityCount fits in that prefix. IsValidSid only reads.
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
    let sid = NonNull::new(token_user.User.Sid).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is missing",
        )
    })?;
    Ok(CurrentUserSid {
        _token_info: token_info,
        sid,
    })
}
#[cfg(windows)]
impl CurrentUserSid {
    fn owner_only_acl(&self) -> io::Result<LocalMemory> {
        use std::ptr::{null, null_mut};
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::Security::Authorization::{
            SetEntriesInAclW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS, TRUSTEE_IS_SID,
            TRUSTEE_IS_USER, TRUSTEE_W,
        };
        use windows_sys::Win32::Security::{ACL, NO_INHERITANCE};
        use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
        let access = EXPLICIT_ACCESS_W {
            grfAccessPermissions: FILE_ALL_ACCESS,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: self.sid.as_ptr().cast(),
            },
        };
        let mut acl: *mut ACL = null_mut();
        // SAFETY (K8, A): `access` is a fully initialized EXPLICIT_ACCESS_W on
        // the stack; its trustee SID points into `self`, which is borrowed for
        // this call. `acl` is a writable out-param. The old ACL is null
        // (no merge). Adopt wraps whatever pointer Windows wrote so LocalFree
        // runs on success, error, and unwind.
        // Unavoidable BECAUSE ACL construction is Win32 FFI that returns a
        // LocalAlloc pointer (Nomicon § FFI).
        // Alternatives FAIL: (1) std fs permissions -- no DACL builder;
        // (2) hand-written SECURITY_DESCRIPTOR bytes -- still SetNamedSecurityInfo
        // FFI plus layout UB; (3) windows-acl crate -- same FFI.
        let result = unsafe { SetEntriesInAclW(1, &access, null(), &mut acl) };
        let acl = LocalMemory::adopt(acl.cast());
        if result != ERROR_SUCCESS {
            return Err(win32_error(result));
        }
        if acl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned a null ACL",
            ));
        }
        Ok(acl)
    }
}
#[cfg(windows)]
fn apply_protected_file_dacl(path: &Path, acl: &LocalMemory) -> io::Result<()> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    if acl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows ACL handle is null",
        ));
    }
    let wide_path = windows_path_to_wide(path)?;
    // SAFETY (K9, A): `wide_path` is a NUL-terminated UTF-16 path with no
    // interior NUL. `acl` is the ACL we created and still own via
    // LocalMemory. Owner/group/SACL pointers are null, which Win32 allows
    // when those security-info bits are unset.
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
            acl.as_acl(),
            null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(win32_error(result));
    }
    Ok(())
}
#[cfg(windows)]
pub fn restrict_file_to_owner(path: &Path) -> io::Result<()> {
    let acl = current_user_sid()?.owner_only_acl()?;
    apply_protected_file_dacl(path, &acl)
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
