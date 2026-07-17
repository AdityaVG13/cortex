use super::keys::cortex_dir;
use super::paths::{CortexPaths, BASE62};
use std::fs;
use std::path::PathBuf;
#[allow(dead_code)]
pub fn write_pid() {
    let dir = cortex_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[cortex] WARNING: cannot create {}: {e}", dir.display());
    }
    fs::write(dir.join("cortex.pid"), std::process::id().to_string()).ok();
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
    unsafe { libc::kill(pid, 0) == 0 }
}
pub fn db_path() -> PathBuf {
    cortex_dir().join("cortex.db")
}
pub(crate) fn fnv1a16(input: &[u8]) -> u16 {
    let mut hash: u32 = 0x811C9DC5;
    for byte in input {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    (hash & 0xFFFF) as u16
}
pub(crate) fn left_pad_base62(num: u16, width: usize) -> String {
    let mut s = base62_encode_u64(num as u64);
    while s.len() < width {
        s.insert(0, '0');
    }
    s
}
pub(crate) fn base62_encode_u64(mut num: u64) -> String {
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
pub(crate) fn base62_encode_bytes(bytes: &[u8]) -> String {
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
