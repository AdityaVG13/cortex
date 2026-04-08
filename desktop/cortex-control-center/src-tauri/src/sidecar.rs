// SPDX-License-Identifier: MIT
use serde::Serialize;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

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
        let child = command
            .spawn()
            .map_err(|e| format!("Failed to start cortex daemon: {e}"))?;

        self.child = Some(child);
        Ok(self.status())
    }

    pub fn stop(&mut self) -> Result<SidecarStatus, String> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            // Poll with timeout to prevent hang if kill doesn't take effect immediately
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if start.elapsed() < std::time::Duration::from_secs(2) => {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    _ => break,
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
