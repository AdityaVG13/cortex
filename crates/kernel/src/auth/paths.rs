use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
pub const CORTEX_DIR_NAME: &str = ".cortex";
pub const CORTEX_GLOBAL_LOCK_NAME: &str = "cortex.global.lock";
pub const CORTEX_GLOBAL_LOCK_HOME_ENV: &str = "CORTEX_GLOBAL_LOCK_HOME";
pub const BASE62: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// Token / secret files are UUID-sized. Match the Control Center cap so a
/// replaced huge file cannot OOM `read_token_from` at runtime open.
pub const MAX_SECRET_FILE_BYTES: u64 = 8 * 1024;
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
    /// `--home --db /tmp/x` must not treat `--db` as the home path.
    /// `--home -- --odd` keeps a home whose name starts with `--`.
    fn find_flag(args: &[String], flag: &str) -> Option<String> {
        let mut i = 0usize;
        while i < args.len() {
            if args[i] == flag {
                if let Some(value) = args.get(i + 1) {
                    if value == "--" {
                        if let Some(explicit) = args.get(i + 2) {
                            if !explicit.trim().is_empty() {
                                return Some(explicit.clone());
                            }
                        }
                    } else if !is_flag_token(value) && !value.trim().is_empty() {
                        return Some(value.clone());
                    }
                }
                i += 1;
                continue;
            }
            i += 1;
        }
        None
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
fn is_flag_token(value: &str) -> bool {
    value.starts_with("--")
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
// CloseHandle / LocalFree sites. Inner K1-K5 and K8-K9 stay (A) FFI.
// K6-K7 are safe reads of the K5-filled TOKEN_USER buffer (PSID at offset 0;
// documented IsValidSid checks: SID_REVISION and SubAuthorityCount).
//
// FFI boundary contract (60-FFI-PATTERNS):
// C/Win32 promises: OpenProcessToken, GetTokenInformation, and CloseHandle are
// BOOL APIs (0 = failure). BOOL failures convey GetLastError, which is
// per-thread and valid only until the next Win32 call on this thread.
// SetEntriesInAclW and SetNamedSecurityInfoW return a DWORD Win32 code
// (ERROR_SUCCESS or a WinError.h value); do not call GetLastError for those.
// None of these APIs callback into Rust, longjmp, or unwind through Rust
// frames. GetCurrentProcess is a non-owning pseudo-handle and must never be
// CloseHandle'd. SetNamedSecurityInfoW copies DACL data; it does not take
// ownership of the ACL pointer. TOKEN_USER / EXPLICIT_ACCESS_W / TRUSTEE_W /
// ACL are windows_sys #[repr(C)] layouts; SID bytes in the token buffer are
// little-endian (Win32). SID validation is a safe in-buffer parse (Revision
// == SID_REVISION and SubAuthorityCount <= 15), matching the documented
// IsValidSid contract without a Win32 call.
// Rust promises: `restrict_file_to_owner` is the thin safe wrapper (F-1).
// Paths become UTF-16 with a trailing NUL and no interior NUL. This crate
// calls Win32; Win32 never calls back, so catch_unwind at an extern-"C" entry
// (F-3) does not apply. Between a BOOL FFI call and last_os_error() there is
// no other FFI and no panic. DWORD APIs map via win32_error(result).
// Handle / heap ownership: OpenProcessToken's HANDLE is owned by OwnedHandle
// (take on success; adopt on failure so a closeable write cannot leak).
// SetEntriesInAclW's ACL is LocalAlloc memory owned by LocalMemory, adopt()'d
// before the error branch so LocalFree runs on success, error, and unwind.
// Panic-in-Drop: OwnedHandle / LocalMemory Drop null the field first, then
// CloseHandle / LocalFree, discard the return, and never call last_os_error
// (Drop cannot surface an error; a second FFI in Drop would clobber a pending
// GetLastError). Drop is non-panicking.
// Thread safety: these calls are safe on distinct objects; GetLastError is
// per-thread; the facade holds no shared state.
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
        // Win32 CloseHandle: BOOL, no callback, no longjmp, no unwind. Drop is
        // non-panicking and does not call last_os_error (panic-in-Drop
        // forbidden; a GetLastError read here would also clobber a pending
        // error from the thread that is dropping us).
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
    fn as_acl(&self) -> *const windows_sys::Win32::Security::ACL {
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
        // Win32 LocalFree: matching deallocator for that LocalAlloc, no
        // callback, no longjmp, no unwind. Drop is non-panicking: the return
        // is discarded and last_os_error is not called (panic-in-Drop;
        // would also clobber a pending GetLastError).
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
        GetTokenInformation, TokenUser, SID_AND_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let mut token: HANDLE = null_mut();
    // SAFETY (K3, A): GetCurrentProcess returns a pseudo-handle valid for
    // this process for the duration of the call -- it is never owned and
    // must never be CloseHandle'd. `token` is a writable out-param on our
    // stack. TOKEN_QUERY is a documented access mask.
    // Win32 OpenProcessToken: BOOL; on success writes an owned token HANDLE
    // the caller must CloseHandle; on failure returns 0 and GetLastError
    // is set. last_os_error is captured BEFORE adopt/Drop so CloseHandle
    // cannot clobber it. Success then take() owns the HANDLE; failure still
    // adopt()'s whatever was written so a closeable handle cannot leak.
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
    // SAFETY (K4, A): `token.as_raw()` is an open token we own (borrowed, not
    // consumed). A null buffer with length 0 is the documented size query;
    // Win32 does not write through that null pointer. It writes `required_len`
    // and returns a length error we ignore.
    // GetLastError is read immediately only if `required_len` stays 0; no
    // other FFI sits between the call and last_os_error.
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
    // bytes; Win32 writes at most TokenInformationLength bytes before we read.
    // The token HANDLE is borrowed, not consumed. BOOL 0 => last_os_error
    // immediately (the Error is built before OwnedHandle Drop runs, so
    // CloseHandle cannot clobber it).
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
    // K6 (C): TOKEN_USER.User.Sid is a PSID at offset 0 of the buffer K5
    // filled. windows-sys SID_AND_ATTRIBUTES.Sid is the first field, so the
    // leading usize is that pointer. Reading the owned Vec needs no unsafe.
    // Later FFI (K8) uses wrapping_add on this Vec after the in-buffer check.
    const _: () = {
        assert!(std::mem::offset_of!(TOKEN_USER, User) == 0);
        assert!(std::mem::offset_of!(SID_AND_ATTRIBUTES, Sid) == 0);
    };
    let sid_addr = token_info[0];
    if sid_addr == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is missing",
        ));
    }
    let init_len = std::cmp::min(returned_len as usize, token_info.len() * word_size);
    let buf_start = token_info.as_ptr() as usize;
    let buf_end = buf_start.saturating_add(init_len);
    if sid_addr < buf_start || sid_addr >= buf_end {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is outside the token buffer",
        ));
    }
    let sid_offset = sid_addr - buf_start;
    const SID_FIXED: usize = 8;
    const SID_MAX_SUB_AUTHORITIES: u8 = 15;
    const SID_REVISION: u8 = 1;
    if sid_offset.saturating_add(SID_FIXED) > init_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID header is truncated",
        ));
    }
    // K7 (C): documented IsValidSid contract is Revision == SID_REVISION and
    // SubAuthorityCount <= SID_MAX_SUB_AUTHORITIES, plus a readable SID. The
    // SID bytes live in this owned Vec; a Win32 IsValidSid call is not required.
    let Some(revision) = token_info_le_byte(&token_info, sid_offset) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID header is truncated",
        ));
    };
    if revision != SID_REVISION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token user SID is invalid",
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
    // Unique owner: NonNull::new takes *mut. as_ptr().cast_mut() would be a
    // *const-to-*mut cast (bucket 14). Later FFI only reads this SID.
    let sid = NonNull::new(
        token_info
            .as_mut_ptr()
            .cast::<u8>()
            .wrapping_add(sid_offset)
            .cast(),
    )
    .ok_or_else(|| {
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
        // Win32 SetEntriesInAclW returns a DWORD error code (not GetLastError);
        // map via win32_error(result). On success *NewAcl is LocalAlloc memory
        // we must LocalFree. No panic between the FFI call and adopt. No
        // callback / longjmp. TRUSTEE_IS_SID: ptstrName is a SID pointer, not
        // a string (no NUL-termination obligation on that field).
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
    // Win32 SetNamedSecurityInfoW returns a DWORD (not GetLastError); map
    // via win32_error(result). It copies DACL data and does not take ownership
    // of the ACL (LocalMemory still owns it; Drop still LocalFree's). LPCWSTR
    // ObjectName is our `wide_path` (trailing NUL, interior NUL rejected in
    // windows_path_to_wide). No callback / longjmp. No panic between the FFI
    // call and the DWORD check.
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
/// Open a home-local automatic file without following a planted symlink
/// (Unix `O_NOFOLLOW`) or Windows reparse point. Operator `--file` paths
/// may still follow; this is for files Cortex opens on its own.
pub fn open_nofollow(path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to follow a reparse point",
            ));
        }
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::File::open(path)
    }
}

/// Append to a home-local ledger without following a planted symlink.
pub fn open_append_nofollow(path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to append through a reparse point",
            ));
        }
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }
}

/// Read a secret without following a planted symlink/reparse and without
/// slurping an attacker-replaced huge file. Write already uses O_NOFOLLOW.
pub fn read_secret_file(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = open_nofollow(path)?;
    read_secret_file_bounded(&mut file)
}

fn read_secret_file_bounded(file: &mut fs::File) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    file.take(MAX_SECRET_FILE_BYTES + 1).read_to_end(&mut buf)?;
    if buf.len() as u64 > MAX_SECRET_FILE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secret file exceeds maximum size",
        ));
    }
    Ok(buf)
}

pub fn write_secret_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        // O_NOFOLLOW: a planted symlink must not redirect the secret. Mode is
        // only applied on create; if the path already existed world-readable,
        // fchmod before truncate so new bytes never sit at 0644.
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.set_len(0)?;
        file.write_all(contents)?;
        file.flush()?;
        restrict_file_to_owner(path)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::io::Write as _;
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        // FILE_FLAG_OPEN_REPARSE_POINT is the O_NOFOLLOW analog: a planted
        // symlink must not redirect the secret. Apply the owner DACL before
        // truncate so a SetNamedSecurityInfo failure cannot wipe an existing
        // token (Unix fchmod-before-set_len).
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to write secret through a reparse point",
            ));
        }
        restrict_file_to_owner(path)?;
        file.set_len(0)?;
        file.write_all(contents)?;
        file.flush()?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
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
