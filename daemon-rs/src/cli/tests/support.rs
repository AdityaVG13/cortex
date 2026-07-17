// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use crate::cli::*;
    use crate::*;
    use std::fs;
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const SPAWN_PARENT_TEST_CHILD_ENV: &str = "CORTEX_SPAWN_PARENT_TEST_CHILD";
    const CONTROL_CENTER_LOCK_TEST_CHILD_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_CHILD";
    const CONTROL_CENTER_LOCK_TEST_HOME_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOME";
    const CONTROL_CENTER_LOCK_TEST_READY_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_READY";
    const CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOLD_MS";

    fn openapi_spec_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("specs")
            .join("cortex-openapi.yaml")
    }

    struct ScopedEnvVar {
        key: &'static str,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            std::env::set_var(key, value);
            Self { key }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            std::env::remove_var(self.key);
        }
    }

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_{name}_{unique}"))
    }

    fn run_preflight(paths: &auth::CortexPaths) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
            .block_on(startup_single_daemon_preflight(paths))
    }

    fn run_ensure_daemon(
        paths: &auth::CortexPaths,
        agent: Option<&str>,
        emit_port: bool,
        allow_service_ensure: bool,
    ) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
            .block_on(ensure_daemon(paths, agent, emit_port, allow_service_ensure))
    }

    fn spawn_response_server(
        listener: TcpListener,
        status_line: &str,
        content_type: &str,
        body: String,
        max_requests: usize,
    ) -> std::thread::JoinHandle<()> {
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
                        if served > 0
                            && last_served_at.is_some_and(|last| {
                                now.duration_since(last) >= idle_grace_after_response
                            })
                        {
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

    fn spawn_preflight_response_server(
        listener: TcpListener,
        status_line: &str,
        content_type: &str,
        body: String,
    ) -> std::thread::JoinHandle<()> {
        spawn_response_server(listener, status_line, content_type, body, 4)
