use super::keys::cortex_dir;
use super::paths::{CortexPaths, BASE62};
use std::fs;
use std::path::PathBuf;
/// Record this process in `paths.pid` so destructive CLI (`cortex restore`)
/// can see a live `cortex serve`. The flock on `paths.lock` is the real
/// exclusion; this file is the documented, inspectable gate.
pub fn write_pid_file(paths: &CortexPaths) -> Result<(), String> {
    fs::create_dir_all(&paths.home).map_err(|e| format!("create home: {e}"))?;
    fs::write(&paths.pid, format!("{}\n", std::process::id()))
        .map_err(|e| format!("write {}: {e}", paths.pid.display()))
}

/// Remove `paths.pid` only when it still names this process.
pub fn remove_own_pid_file(paths: &CortexPaths) {
    let Ok(recorded) = fs::read_to_string(&paths.pid) else {
        return;
    };
    if recorded.trim().parse::<u32>().ok() == Some(std::process::id()) {
        let _ = fs::remove_file(&paths.pid);
    }
}
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

/// Returns the pid recorded in the daemon pid file while that process is
/// alive (i.e. a daemon appears active); `None` when the file is absent,
/// unparseable, or the recorded pid is dead. Complement of
/// `stale_pid_candidate`; used by destructive CLI paths (`cortex restore`)
/// that must refuse while a daemon may be running.
pub fn pid_file_live_pid(paths: &CortexPaths) -> Option<u32> {
    if !paths.pid.exists() {
        return None;
    }
    let pid = fs::read_to_string(&paths.pid)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())?;
    if pid == std::process::id() || process_is_running(pid) {
        return Some(pid);
    }
    None
}
/// True when `pid` names a live process. Pid 0 is never a recorded daemon
/// (POSIX `kill(0, ·)` is process-group; Windows 0 is the Idle process), so
/// both OS probes reject it before any FFI. `EPERM` / `ERROR_ACCESS_DENIED`
/// count as alive: the process exists; this caller just cannot signal/open it.
fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    process_is_running_os(pid)
}
#[cfg(windows)]
fn process_is_running_os(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY (K10W, A): `pid` is a DWORD already rejected as 0 by
    // `process_is_running`. PROCESS_QUERY_LIMITED_INFORMATION is the
    // least-privilege existence probe; inherit-handle is BOOL FALSE (0).
    // OpenProcess returns HANDLE, not BOOL: success is a closeable handle;
    // documented failure is NULL with GetLastError set. INVALID_HANDLE_VALUE
    // is CreateFile's failure sentinel, not OpenProcess's -- it is not owned
    // and last-error is not this call's (stale ERROR_INVALID_PARAMETER would
    // otherwise report a live pid dead). No other pointers.
    // Between a closeable OpenProcess result and CloseHandle there is only
    // sentinel comparison (no panic), so OwnedHandle is not required.
    //
    // ACCESS_DENIED is the EPERM analog (process exists, we cannot open it).
    // INVALID_PARAMETER is the usual "no such process" for a dead/unused PID.
    // Any other GetLastError -- including a failed query we cannot classify --
    // is treated as alive so `pid_file_live_pid` / restore stay closed.
    // `last_os_error` is read immediately, and only after NULL; CloseHandle is
    // not called on that path, so it cannot clobber the OpenProcess error.
    //
    // Unavoidable BECAUSE process-open is Win32 FFI (Nomicon § FFI
    // https://doc.rust-lang.org/nomicon/ffi.html ; Reference § External
    // blocks https://doc.rust-lang.org/reference/items/external-blocks.html ;
    // canonical unavoidable §2). Alternatives FAIL: (1) tasklist.exe -- spawn
    // or non-success was treated as dead, opening restore against a live
    // daemon; (2) sysinfo -- can omit foreign-user / protected pids, same
    // silent-proceed hole; (3) windows / windows-acl crates -- same
    // OpenProcess FFI, not a safe language form.
    let handle: HANDLE = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(err) if err == ERROR_INVALID_PARAMETER as i32 => false,
            Some(err) if err == ERROR_ACCESS_DENIED as i32 => true,
            _ => true,
        };
    }
    if handle == INVALID_HANDLE_VALUE {
        return true;
    }
    // SAFETY (K10W-close, A): `handle` is the exclusive OpenProcess success
    // result (NULL and INVALID_HANDLE_VALUE were rejected). CloseHandle is
    // the documented destructor; the BOOL is discarded. The only code since
    // OpenProcess is sentinel comparison, which cannot panic, so a Drop
    // wrapper would not change leak behavior.
    unsafe {
        let _ = CloseHandle(handle);
    }
    true
}
#[cfg(unix)]
fn process_is_running_os(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY (K10, A): signal 0 only checks existence/permission and delivers
    // nothing. `process_is_running` rejected pid 0; `try_from` plus `pid > 0`
    // guarantee a single positive pid_t, so `kill` cannot take the special
    // 0 / negative forms (process group / all processes). No pointers.
    // The wrapper is non-panicking (no `?` / unwrap / expect).
    //
    // errno is thread-local on supported POSIX; `last_os_error` is read
    // immediately after a non-zero return and is the only libc-adjacent call
    // on that path (C-3 / 60-FFI-PATTERNS). Callers never see raw errno.
    //
    // EPERM means the signal was denied but the target EXISTS (foreign-user
    // daemon, or a seccomp/sandbox denying kill outright). Treating it as dead
    // made `pid_file_live_pid` -- and through it the `cortex restore`
    // destructive-op gate and stale-pid cleanup -- silently proceed against a
    // live daemon in exactly the environments where signals are restricted.
    // Dead only on ESRCH; EPERM and any other/missing errno count as alive
    // so those gates stay closed when the probe is uncertain.
    //
    // Unavoidable BECAUSE POSIX process-existence is a libc syscall (Nomicon
    // § FFI https://doc.rust-lang.org/nomicon/ffi.html ; Reference § External
    // blocks https://doc.rust-lang.org/reference/items/external-blocks.html ;
    // canonical unavoidable §2: nix/rustix wrap the same kill(2)).
    // Alternatives FAIL: (1) /proc/{pid} -- absent on macOS; (2) sysinfo
    // (already a kernel dep) -- snapshots can omit foreign-user pids, flipping
    // EPERM-alive to dead and opening the restore/stale-pid gates; (3)
    // Command("kill","-0") -- exit 1 collapses ESRCH and EPERM, same silent-
    // proceed bug this comment documents. Already a one-call wrapper; not (C).
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
pub fn db_path() -> PathBuf {
    cortex_dir().join("cortex.db")
}
pub fn fnv1a16(input: &[u8]) -> u16 {
    let mut hash: u32 = 0x811C9DC5;
    for byte in input {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    (hash & 0xFFFF) as u16
}
pub fn left_pad_base62(num: u16, width: usize) -> String {
    let mut s = base62_encode_u64(num as u64);
    while s.len() < width {
        s.insert(0, '0');
    }
    s
}
pub fn base62_encode_u64(mut num: u64) -> String {
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
pub fn base62_encode_bytes(bytes: &[u8]) -> String {
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
