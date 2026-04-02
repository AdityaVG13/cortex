use std::path::PathBuf;
use std::fs;
use uuid::Uuid;

const CORTEX_DIR_NAME: &str = ".cortex";

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

/// Write the current process PID to `~/.cortex/cortex.pid`.
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

    let query = format!(
        "(Get-CimInstance Win32_Process -Filter \"ProcessId = {pid}\").CommandLine"
    );
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

/// Returns the default database path: `~/cortex/cortex.db`.
pub fn db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("cortex").join("cortex.db")
}
