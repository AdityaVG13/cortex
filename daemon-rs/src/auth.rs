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

/// Write the current process PID to `~/.cortex/cortex.pid`.
pub fn write_pid() {
    let dir = cortex_dir();
    fs::create_dir_all(&dir).ok();
    fs::write(dir.join("cortex.pid"), std::process::id().to_string()).ok();
}

/// Return `true` when the `Authorization` header carries a valid Bearer token
/// matching `expected_token`.
pub fn validate_auth(headers: &axum::http::HeaderMap, expected_token: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v) == expected_token)
        .unwrap_or(false)
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
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }

            #[cfg(unix)]
            {
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

/// Returns the default database path: `~/cortex/cortex.db`.
pub fn db_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("cortex").join("cortex.db")
}
