// SPDX-License-Identifier: MIT
use serde::Serialize;
use std::fs;
use std::io;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug)]
pub struct SidecarStatus {
    pub running: bool,
    pub pid: Option<u32>,
}

#[derive(Default)]
pub struct SidecarDaemon {
    child: Option<Child>,
    exe_path: Option<PathBuf>,
    runtime_copy_dir: Option<PathBuf>,
    runtime_copy_path: Option<PathBuf>,
}

impl SidecarDaemon {
    pub fn with_exe_path(path: PathBuf, runtime_copy_dir: Option<PathBuf>) -> Self {
        Self {
            child: None,
            exe_path: Some(path),
            runtime_copy_dir,
            runtime_copy_path: None,
        }
    }

    pub fn status(&mut self) -> SidecarStatus {
        self.reap_if_exited();
        SidecarStatus {
            running: self.child.is_some(),
            pid: self.child.as_ref().map(|c| c.id()),
        }
    }

    pub fn start(&mut self) -> Result<SidecarStatus, String> {
        self.reap_if_exited();
        if self.child.is_some() {
            return Ok(self.status());
        }

        let exe = self
            .exe_path
            .as_ref()
            .ok_or_else(|| "Cortex binary path not configured".to_string())?;

        if !exe.exists() {
            return Err(format!("Cortex binary not found at {}", exe.display()));
        }

        let exe = exe.clone();
        let spawn_path = self.prepare_spawn_path(&exe);
        let mut command = Command::new(&spawn_path);
        command
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Failed to start cortex daemon: {e}"))?;

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return Err(format!(
                        "Cortex daemon exited immediately with status {status}. Reuse the existing daemon or check daemon logs."
                    ));
                }
                Ok(None) if started.elapsed() < Duration::from_millis(250) => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Ok(None) => break,
                Err(err) => {
                    return Err(format!(
                        "Failed to query cortex daemon status after start: {err}"
                    ));
                }
            }
        }

        self.runtime_copy_path = (spawn_path != exe).then_some(spawn_path);
        self.child = Some(child);
        Ok(self.status())
    }

    pub fn stop(&mut self) -> Result<SidecarStatus, String> {
        if let Some(mut child) = self.child.take() {
            let pid = child.id();
            match child.try_wait() {
                Ok(Some(_)) => {
                    return Ok(SidecarStatus {
                        running: false,
                        pid: None,
                    });
                }
                Ok(None) => {}
                Err(err) => {
                    self.child = Some(child);
                    return Err(format!(
                        "Failed to query cortex daemon process {pid} before stop: {err}"
                    ));
                }
            }

            if let Err(err) = child.kill() {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        return Ok(SidecarStatus {
                            running: false,
                            pid: None,
                        });
                    }
                    Ok(None) => {
                        self.child = Some(child);
                        return Err(format!("Failed to stop cortex daemon process {pid}: {err}"));
                    }
                    Err(wait_err) => {
                        self.child = Some(child);
                        return Err(format!(
                            "Failed to stop cortex daemon process {pid}: {err} (and could not confirm exit: {wait_err})"
                        ));
                    }
                }
            }

            // Poll with timeout to prevent hang if kill doesn't take effect immediately.
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if start.elapsed() < std::time::Duration::from_secs(2) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    Ok(None) => {
                        self.child = Some(child);
                        return Err(format!(
                            "Timed out waiting for cortex daemon process {pid} to exit"
                        ));
                    }
                    Err(err) => {
                        self.child = Some(child);
                        return Err(format!(
                            "Failed while waiting for cortex daemon process {pid} to exit: {err}"
                        ));
                    }
                }
            }
        }
        self.cleanup_active_runtime_copy();
        Ok(SidecarStatus {
            running: false,
            pid: None,
        })
    }

    fn reap_if_exited(&mut self) {
        let exited = match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
            None => false,
        };
        if exited {
            self.child = None;
            self.cleanup_active_runtime_copy();
        }
    }

    fn prepare_spawn_path(&mut self, source: &Path) -> PathBuf {
        if let Some(runtime_copy) = self.prepare_runtime_copy(source) {
            return runtime_copy;
        }
        self.runtime_copy_path = None;
        source.to_path_buf()
    }

    fn prepare_runtime_copy(&mut self, source: &Path) -> Option<PathBuf> {
        if !should_use_runtime_copy(source) {
            return None;
        }

        let runtime_dir = self.runtime_copy_dir.as_ref()?;
        if let Err(err) = fs::create_dir_all(runtime_dir) {
            eprintln!(
                "[cortex-control-center] Failed to create runtime copy dir {}: {}",
                runtime_dir.display(),
                err
            );
            return None;
        }

        let runtime_path = runtime_copy_path(runtime_dir, source);
        if let Err(err) = copy_if_changed(source, &runtime_path) {
            eprintln!(
                "[cortex-control-center] Failed to refresh runtime daemon copy {} -> {}: {}",
                source.display(),
                runtime_path.display(),
                err
            );
            return None;
        }

        cleanup_stale_runtime_copies(runtime_dir, &runtime_path);
        Some(runtime_path)
    }

    fn cleanup_active_runtime_copy(&mut self) {
        if let Some(path) = self.runtime_copy_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl Drop for SidecarDaemon {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn should_use_runtime_copy(path: &Path) -> bool {
    cfg!(debug_assertions) && is_workspace_daemon_binary(path)
}

fn is_workspace_daemon_binary(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case("daemon-rs"))
            .unwrap_or(false)
    })
}

fn runtime_copy_path(runtime_dir: &Path, source: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    runtime_dir.join(format!(
        "cortex-dev-run-{}{}",
        std::process::id(),
        extension
    ))
}

fn cleanup_stale_runtime_copies(runtime_dir: &Path, active_path: &Path) {
    let entries = match fs::read_dir(runtime_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if !name.starts_with("cortex-dev-run-") || path == active_path {
            continue;
        }
        let _ = fs::remove_file(path);
    }
}

fn copy_if_changed(src: &Path, dest: &Path) -> io::Result<()> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => existing != fs::read(src)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => true,
        Err(err) => return Err(err),
    };

    if needs_copy {
        fs::copy(src, dest)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{cleanup_stale_runtime_copies, is_workspace_daemon_binary, runtime_copy_path};
    use std::fs;
    use std::path::Path;

    #[test]
    fn workspace_daemon_binary_detects_repo_daemon_paths() {
        let path = Path::new(
            r"C:\Users\aditya\cortex\daemon-rs\target-control-center-dev\debug\cortex.exe",
        );
        assert!(is_workspace_daemon_binary(path));
    }

    #[test]
    fn workspace_daemon_binary_ignores_installed_and_sidecar_paths() {
        let installed = Path::new(r"C:\Users\aditya\.cortex\bin\cortex.exe");
        let sidecar = Path::new(
            r"C:\Users\aditya\cortex\desktop\cortex-control-center\src-tauri\target\debug\cortex.exe",
        );
        assert!(!is_workspace_daemon_binary(installed));
        assert!(!is_workspace_daemon_binary(sidecar));
    }

    #[test]
    fn runtime_copy_path_is_process_scoped() {
        let runtime_dir = Path::new(r"C:\Users\aditya\.cortex\runtime\control-center-dev");
        let source = Path::new(r"C:\Users\aditya\cortex\daemon-rs\target\debug\cortex.exe");
        let path = runtime_copy_path(runtime_dir, source);
        let name = path.file_name().and_then(|value| value.to_str()).unwrap();

        assert!(path.starts_with(runtime_dir));
        assert!(name.starts_with("cortex-dev-run-"));
        assert!(name.ends_with(".exe"));
    }

    #[test]
    fn cleanup_stale_runtime_copies_keeps_active_file() {
        let runtime_dir = std::env::temp_dir().join(format!(
            "cortex_sidecar_cleanup_test_{}",
            std::process::id()
        ));
        fs::create_dir_all(&runtime_dir).expect("create runtime dir");

        let active = runtime_dir.join("cortex-dev-run-active.exe");
        let stale = runtime_dir.join("cortex-dev-run-stale.exe");
        let unrelated = runtime_dir.join("notes.txt");
        fs::write(&active, b"active").expect("write active");
        fs::write(&stale, b"stale").expect("write stale");
        fs::write(&unrelated, b"note").expect("write unrelated");

        cleanup_stale_runtime_copies(&runtime_dir, &active);

        assert!(active.exists());
        assert!(!stale.exists());
        assert!(unrelated.exists());

        let _ = fs::remove_file(active);
        let _ = fs::remove_file(unrelated);
        let _ = fs::remove_dir(runtime_dir);
    }
}
