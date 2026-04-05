// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex Control Center.
//
// Cortex Control Center is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

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
            let _ = child.wait();
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

