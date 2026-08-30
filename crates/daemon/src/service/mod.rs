#[cfg(windows)]
const SERVICE_NAME: &str = "CortexDaemon";
#[cfg(windows)]
const DISPLAY_NAME: &str = "Cortex Memory Daemon";
#[cfg(windows)]
const DESCRIPTION: &str = "Always-on AI memory daemon -- serves Claude, Gemini, Codex, Cursor, and local LLMs via HTTP (:7437) and MCP.";

#[cfg(windows)]
fn run_sc(args: &[&str], success: &str, tolerated: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("sc.exe");
    command.args(args).creation_flags(CREATE_NO_WINDOW);
    match command.output() {
        Ok(output) if output.status.success() => {
            eprintln!("{success}");
            true
        }
        Ok(output) => {
            let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            if tolerated.iter().any(|needle| text.contains(needle)) {
                eprintln!("{success}");
                true
            } else {
                eprintln!("{text}");
                false
            }
        }
        Err(err) => {
            eprintln!("[cortex] Failed to run sc.exe: {err}");
            false
        }
    }
}

#[cfg(not(windows))]
fn unsupported() -> bool {
    eprintln!("[cortex] Windows service management is only available on Windows");
    false
}

#[cfg(windows)]
pub fn install() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("[cortex] Failed to get current executable path");
        return false;
    };
    let exe = exe.to_string_lossy();
    let bin_path = format!("\"{exe}\" service-run");
    let ok = run_sc(
        &["create", SERVICE_NAME, "binPath=", &bin_path, "start=", "demand", "DisplayName=", DISPLAY_NAME],
        &format!("[cortex] Service '{SERVICE_NAME}' installed"),
        &["1073"],
    );
    if ok {
        let _ = run_sc(&["description", SERVICE_NAME, DESCRIPTION], "", &[]);
    }
    ok
}

#[cfg(not(windows))]
pub fn install() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn uninstall() -> bool {
    let _ = run_sc(&["stop", SERVICE_NAME], "[cortex] Service stopped", &["1062"]);
    run_sc(&["delete", SERVICE_NAME], "[cortex] Service uninstalled", &[])
}

#[cfg(not(windows))]
pub fn uninstall() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn start() -> bool {
    run_sc(&["start", SERVICE_NAME], "[cortex] Service started", &["1056"])
}

#[cfg(not(windows))]
pub fn start() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn stop() -> bool {
    run_sc(&["stop", SERVICE_NAME], "[cortex] Service stopped", &["1062"])
}

#[cfg(not(windows))]
pub fn stop() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn status() -> bool {
    run_sc(&["query", SERVICE_NAME], "[cortex] Service status queried", &[])
}

#[cfg(not(windows))]
pub fn status() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn ensure() -> bool {
    status() || install() && start()
}

#[cfg(not(windows))]
pub fn ensure() -> bool {
    unsupported()
}

#[cfg(windows)]
pub fn ensure_ready() -> bool {
    ensure()
}

#[cfg(windows)]
mod scm {
    use std::ffi::OsString;
    use windows_service::define_windows_service;
    use windows_service::service_dispatcher;

    define_windows_service!(ffi_service_main, cortex_service_main);

    pub fn dispatch() {
        if let Err(err) = service_dispatcher::start(super::SERVICE_NAME, ffi_service_main) {
            eprintln!("[cortex] Failed to start service dispatcher: {err}");
            std::process::exit(1);
        }
    }

    fn cortex_service_main(_arguments: Vec<OsString>) {
        match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(crate::run_daemon(crate::auth::CortexPaths::resolve(), std::future::pending::<()>())),
            Err(err) => eprintln!("[cortex-service] Failed to create tokio runtime: {err}"),
        }
    }
}

#[cfg(windows)]
pub fn dispatch_service() {
    scm::dispatch();
}

#[cfg(not(windows))]
pub fn dispatch_service() {
    eprintln!("[cortex] service-run is only available on Windows");
}
