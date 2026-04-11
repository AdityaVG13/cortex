// SPDX-License-Identifier: MIT
use serde::Serialize;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
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
}

impl SidecarDaemon {
    pub fn with_exe_path(path: PathBuf) -> Self {
        Self {
            child: None,
            exe_path: Some(path),
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

        let mut command = Command::new(exe);
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
        }
    }
}

impl Drop for SidecarDaemon {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
