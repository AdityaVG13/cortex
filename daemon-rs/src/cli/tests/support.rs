// SPDX-License-Identifier: MIT
use crate::cli::daemon::{
    control_center_is_active, startup_single_daemon_preflight, CONTROL_CENTER_LOCK_FILE,
};
use crate::cli::*;
use crate::*;
use fs2::FileExt;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
pub(crate) const SPAWN_PARENT_TEST_CHILD_ENV: &str = "CORTEX_SPAWN_PARENT_TEST_CHILD";
pub(crate) const CONTROL_CENTER_LOCK_TEST_CHILD_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_CHILD";
pub(crate) const CONTROL_CENTER_LOCK_TEST_HOME_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOME";
pub(crate) const CONTROL_CENTER_LOCK_TEST_READY_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_READY";
pub(crate) const CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOLD_MS";
pub(crate) fn openapi_spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("specs").join("cortex-openapi.yaml")
}
pub(crate) struct ScopedEnvVar {
    key: &'static str,
}
impl ScopedEnvVar {
    pub(crate) fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }
}
impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
    }
}
pub(crate) fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
    crate::test_env::lock()
}
pub(crate) fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    std::env::temp_dir().join(format!("cortex_{name}_{unique}"))
}
pub(crate) fn run_preflight(paths: &auth::CortexPaths) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
        .block_on(startup_single_daemon_preflight(paths))
}
pub(crate) fn run_ensure_daemon(paths: &auth::CortexPaths, agent: Option<&str>, emit_port: bool, allow_service_ensure: bool) -> Result<(), String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
        .block_on(ensure_daemon(paths, agent, emit_port, allow_service_ensure))
}
pub(crate) fn spawn_response_server(listener: TcpListener, status_line: &str, content_type: &str, body: String, max_requests: usize) -> std::thread::JoinHandle<()> {
    let status_line = status_line.to_string();
    let content_type = content_type.to_string();
    let max_requests = max_requests.max(1);
    std::thread::spawn(move || {
        let _ = listener.set_nonblocking(true);
        let deadline = Instant::now() + Duration::from_secs(15);
        let idle_grace_after_response = Duration::from_millis(500);
        let mut served = 0_usize;
        let mut last_served_at: Option<Instant> = None;
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request_buffer = [0_u8; 2048];
                    let _ = stream.read(&mut request_buffer);
                    let response = format!(
                        "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    served += 1;
                    last_served_at = Some(Instant::now());
                    if served >= max_requests {
                        break;
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    let now = Instant::now();
                    if served > 0 && last_served_at.is_some_and(|last| now.duration_since(last) >= idle_grace_after_response) {
                        break;
                    }
                    if now >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
    })
}
pub(crate) fn spawn_preflight_response_server(listener: TcpListener, status_line: &str, content_type: &str, body: String) -> std::thread::JoinHandle<()> {
    spawn_response_server(listener, status_line, content_type, body, 4)
}
pub(crate) fn wait_for_control_center_lock(paths: &auth::CortexPaths, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if control_center_is_active(paths).unwrap_or(false) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}
pub(crate) fn control_center_lock_holder_child_process() {
    if std::env::var(CONTROL_CENTER_LOCK_TEST_CHILD_ENV).ok().as_deref() != Some("1") {
        return;
    }
    let home = std::env::var(CONTROL_CENTER_LOCK_TEST_HOME_ENV).expect("control-center lock test home env missing");
    let ready_file = std::env::var(CONTROL_CENTER_LOCK_TEST_READY_ENV).expect("control-center lock ready marker env missing");
    let hold_ms = std::env::var(CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV).ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(1500);
    let lock_path = PathBuf::from(home).join("runtime").join(CONTROL_CENTER_LOCK_FILE);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).expect("create lock parent dir");
    }
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open lock file");
    lock_file.try_lock_exclusive().expect("acquire control-center lock");
    std::fs::write(ready_file, b"locked").expect("write lock ready marker");
    std::thread::sleep(Duration::from_millis(hold_ms));
}
