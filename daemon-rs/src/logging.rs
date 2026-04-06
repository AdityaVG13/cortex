// SPDX-License-Identifier: MIT
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

#[allow(dead_code)]
/// Return `~/.cortex/cortex-daemon.log`.
fn log_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".cortex")
        .join("cortex-daemon.log")
}

#[allow(dead_code)]
/// Append a timestamped line to the daemon log file.
/// Silently ignores write errors so logging never blocks the daemon.
pub fn log_line(message: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let ts = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ");
        let _ = writeln!(file, "[{ts}] {message}");
    }
}

