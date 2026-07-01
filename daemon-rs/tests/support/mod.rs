use std::process::{Child, Command};

pub mod harness;

pub use harness::{
    adapter_conformance_guard, assert_path_scoped_to_home, daemon_spawn_test_guard, health_ok,
    http_request, http_status, normalize_path_for_compare, post_json, post_raw, read_token,
    request_json, request_json_with_headers, reserve_port, shutdown_daemon,
    shutdown_daemon_best_effort, singleton_transport_test_guard, spawn_daemon, split_http_body,
    unique_temp_dir, wait_for_exit, wait_for_health, JsonHttpResponse, HEALTH_POLL_INTERVAL,
    STARTUP_TIMEOUT,
};

pub trait SpawnTrackedExt {
    fn spawn_tracked(&mut self, context: &str) -> Child;
}

impl SpawnTrackedExt for Command {
    fn spawn_tracked(&mut self, context: &str) -> Child {
        let child = self
            .spawn()
            .unwrap_or_else(|err| panic!("{context}: {err}"));
        track_child_for_cleanup(&child);
        child
    }
}

pub fn terminate_child_tree(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }

    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }

    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn track_child_for_cleanup(_child: &Child) {
    #[cfg(windows)]
    {
        windows_cleanup_job::assign_child_to_cleanup_job(_child);
    }
}

#[cfg(windows)]
mod windows_cleanup_job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    const ERROR_ACCESS_DENIED: i32 = 5;

    fn cleanup_job_handle() -> Option<HANDLE> {
        static CLEANUP_JOB: OnceLock<isize> = OnceLock::new();
        let raw = *CLEANUP_JOB.get_or_init(|| {
            let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if job.is_null() || job == INVALID_HANDLE_VALUE {
                return 0;
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                unsafe { CloseHandle(job) };
                return 0;
            }
            job as isize
        });
        if raw == 0 { None } else { Some(raw as HANDLE) }
    }

    pub(crate) fn assign_child_to_cleanup_job(child: &Child) {
        let Some(job) = cleanup_job_handle() else {
            return;
        };
        let process_handle = child.as_raw_handle() as HANDLE;
        let ok = unsafe { AssignProcessToJobObject(job, process_handle) };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_ACCESS_DENIED) {
                eprintln!(
                    "[test-support] assign process {} to cleanup job failed: {err}",
                    child.id()
                );
            }
        }
    }
}
