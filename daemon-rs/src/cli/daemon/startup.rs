use crate::auth;
use crate::cli::boot::boot_agent;
use crate::cli::common::{env_trimmed, local_daemon_base_url, normalize_option, parse_truthy_flag, single_daemon_test_bypass_enabled};
use crate::daemon_lifecycle;
use crate::transport;
#[cfg(not(windows))]
use daemon_lifecycle::issue_owner_token_for_spawn;
use daemon_lifecycle::{
    daemon_healthy, is_cortex_health_payload, readiness_state_from_payload, validate_spawned_owner_claim, wait_for_health,
    DAEMON_OWNER_TOKEN_ENV, SPAWN_PARENT_START_TIME_ENV,
};
use fs2::FileExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
pub(crate) const CONTROL_CENTER_LOCK_FILE: &str = "control-center.lock";
pub(crate) const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
pub(crate) const SPAWN_PARENT_PID_ENV: &str = "CORTEX_SPAWN_PARENT_PID";
pub(crate) const ORPHAN_WATCH_INTERVAL_SECS: u64 = 2;
pub(crate) const DEFAULT_EMBED_BACKFILL_BATCH_SIZE: usize = 200;
pub(crate) const DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS: usize = 8;
pub(crate) const DEFAULT_EMBED_BACKFILL_INTERVAL_SECS: u64 = 120;
pub(crate) const DEFAULT_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES: usize = 64;
pub(crate) const DEFAULT_STARTUP_INDEX_DELAY_SECS: u64 = 5;
pub(crate) const DEFAULT_STARTUP_AGING_DELAY_SECS: u64 = 20;
pub(crate) const DEFAULT_STARTUP_EMBED_DELAY_SECS: u64 = 30;
pub(crate) const DEFAULT_STARTUP_CRYSTALLIZE_DELAY_SECS: u64 = 45;
pub(crate) const DEFAULT_STARTUP_STORAGE_GOVERNOR_DELAY_SECS: u64 = 12;
pub(crate) const STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES: usize = 3;
pub(crate) const STARTUP_STORAGE_GOVERNOR_CATCHUP_INTERVAL_SECS: u64 = 90;
pub(crate) const BACKGROUND_DB_LOCK_RETRY_MS: u64 = 50;
pub(crate) const BACKGROUND_DB_LOCK_DEFAULT_MAX_WAIT_MS: u64 = 2_000;
pub(crate) const APP_MANAGED_STARTUP_HEAVY_DELAY_SECS: u64 = 45;
pub(crate) const APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS: u64 = 120;
pub(crate) const APP_MANAGED_AGING_STARTUP_OFFSET_SECS: u64 = 15;
pub(crate) const APP_MANAGED_EMBED_STARTUP_OFFSET_SECS: u64 = 30;
pub(crate) const APP_MANAGED_CRYSTALLIZE_STARTUP_OFFSET_SECS: u64 = 45;
pub(crate) const DEFAULT_IDLE_SHUTDOWN_CHECK_INTERVAL_SECS: u64 = 5;
pub(crate) const DEFAULT_IDLE_SHUTDOWN_MIN_UPTIME_SECS: u64 = 120;
pub(crate) const STARTUP_INDEX_DELAY_ENV: &str = "CORTEX_STARTUP_INDEX_DELAY_SECS";
pub(crate) const STARTUP_AGING_DELAY_ENV: &str = "CORTEX_STARTUP_AGING_DELAY_SECS";
pub(crate) const STARTUP_EMBED_DELAY_ENV: &str = "CORTEX_STARTUP_EMBED_DELAY_SECS";
pub(crate) const STARTUP_CRYSTALLIZE_DELAY_ENV: &str = "CORTEX_STARTUP_CRYSTALLIZE_DELAY_SECS";
pub(crate) const STARTUP_STORAGE_GOVERNOR_DELAY_ENV: &str = "CORTEX_STARTUP_STORAGE_GOVERNOR_DELAY_SECS";
pub(crate) const BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV: &str = "CORTEX_BACKGROUND_DB_LOCK_MAX_WAIT_MS";
pub(crate) const EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV: &str = "CORTEX_EMBED_BACKFILL_DRAIN_ON_STARTUP";
pub(crate) const EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES_ENV: &str = "CORTEX_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES";
pub(crate) const IDLE_SHUTDOWN_SECS_ENV: &str = "CORTEX_IDLE_SHUTDOWN_SECS";
pub(crate) const IDLE_SHUTDOWN_MIN_UPTIME_SECS_ENV: &str = "CORTEX_IDLE_SHUTDOWN_MIN_UPTIME_SECS";
pub(crate) const DAEMON_STARTUP_WAIT_SECS: u64 = 90;
pub(crate) const DEFAULT_DAEMON_LOCK_WAIT_SECS: u64 = 15;
pub(crate) const DAEMON_LOCK_RETRY_INTERVAL_MS: u64 = 100;
pub(crate) const DAEMON_LOCK_HANDOFF_GRACE_SECS: u64 = 3;
pub(crate) const DAEMON_LOCAL_SPAWN_ENV: &str = "CORTEX_DAEMON_OWNER_LOCAL_SPAWN";
pub(crate) const APP_REQUIRED_ENV: &str = "CORTEX_APP_REQUIRED";
pub(crate) const APP_CLIENT_ENV: &str = "CORTEX_APP_CLIENT";
pub(crate) const APP_MANAGED_STARTUP_DELAY_ENV: &str = "CORTEX_APP_MANAGED_STARTUP_DELAY_SECS";
pub(crate) fn daemon_lock_wait_timeout() -> Duration {
    let secs = std::env::var("CORTEX_DAEMON_LOCK_WAIT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DAEMON_LOCK_WAIT_SECS);
    Duration::from_secs(secs.max(1))
}
#[derive(Debug)]
pub(crate) struct RuntimeLockGuards {
    _scoped: std::fs::File,
    _global: Option<std::fs::File>,
}
pub(crate) fn try_acquire_runtime_locks(paths: &auth::CortexPaths) -> Result<RuntimeLockGuards, String> {
    let scoped = auth::acquire_daemon_lock(paths)?;
    let global = if single_daemon_test_bypass_enabled() {
        None
    } else {
        match auth::acquire_global_daemon_lock() {
            Ok(lock) => Some(lock),
            Err(err) => {
                drop(scoped);
                return Err(err);
            }
        }
    };
    Ok(RuntimeLockGuards { _scoped: scoped, _global: global })
}
pub(crate) fn acquire_runtime_lock(paths: &auth::CortexPaths) -> Result<RuntimeLockGuards, String> {
    let _ = auth::cleanup_stale_pid_lock(paths);
    if std::env::var("CORTEX_WAIT_FOR_DAEMON_LOCK").ok().is_some_and(|value| value == "1") {
        let deadline = std::time::Instant::now() + daemon_lock_wait_timeout();
        let last_err = loop {
            match try_acquire_runtime_locks(paths) {
                Ok(lock) => return Ok(lock),
                Err(err) => {
                    let _ = auth::cleanup_stale_pid_lock(paths);
                    if std::time::Instant::now() >= deadline {
                        break err;
                    }
                    std::thread::sleep(Duration::from_millis(DAEMON_LOCK_RETRY_INTERVAL_MS));
                }
            }
        };
        let grace_deadline = std::time::Instant::now() + Duration::from_secs(DAEMON_LOCK_HANDOFF_GRACE_SECS);
        while std::time::Instant::now() < grace_deadline {
            let _ = auth::cleanup_stale_pid_lock(paths);
            if let Ok(lock) = try_acquire_runtime_locks(paths) {
                return Ok(lock);
            }
            std::thread::sleep(Duration::from_millis(DAEMON_LOCK_RETRY_INTERVAL_MS));
        }
        return Err(last_err);
    }
    try_acquire_runtime_locks(paths)
}
pub(crate) fn daemon_owner_tag_from_env() -> Option<String> {
    std::env::var("CORTEX_DAEMON_OWNER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
pub(crate) fn daemon_owner_token_from_env() -> Option<String> {
    std::env::var(DAEMON_OWNER_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
pub(crate) fn spawn_parent_pid_from_env() -> Option<u32> {
    std::env::var(SPAWN_PARENT_PID_ENV).ok().and_then(|value| value.trim().parse::<u32>().ok())
}
pub(crate) fn spawn_parent_start_time_from_env() -> Option<u64> {
    std::env::var(SPAWN_PARENT_START_TIME_ENV).ok().and_then(|value| value.trim().parse::<u64>().ok())
}
pub(crate) fn should_watch_spawn_parent(owner_tag: Option<&str>) -> bool {
    owner_tag.map(|owner| !owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG)).unwrap_or(true)
}
pub(crate) fn is_control_center_owner(owner_tag: Option<&str>) -> bool {
    owner_tag.map(|owner| owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG)).unwrap_or(false)
}
pub(crate) fn parse_env_u64_nonnegative(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|raw| raw.trim().parse::<u64>().ok()).unwrap_or(default)
}
pub(crate) fn app_managed_startup_heavy_delay(owner_tag: Option<&str>) -> Duration {
    if !is_control_center_owner(owner_tag) {
        return Duration::from_secs(0);
    }
    let secs = parse_env_u64_nonnegative(APP_MANAGED_STARTUP_DELAY_ENV, APP_MANAGED_STARTUP_HEAVY_DELAY_SECS)
        .min(APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS);
    Duration::from_secs(secs)
}
#[derive(Clone, Copy)]
pub(crate) struct StartupSchedule {
    pub(crate) index: Duration,
    pub(crate) aging: Duration,
    pub(crate) embed: Duration,
    pub(crate) crystallize: Duration,
    pub(crate) storage_governor_initial: Duration,
}
pub(crate) fn startup_delay_from_env(key: &str, default: u64) -> Duration {
    Duration::from_secs(parse_env_u64_nonnegative(key, default).min(3_600))
}
pub(crate) fn startup_schedule(owner_tag: Option<&str>) -> StartupSchedule {
    let zero = Duration::from_secs(0);
    let heavy = app_managed_startup_heavy_delay(owner_tag);
    let index = if heavy > zero { heavy } else { startup_delay_from_env(STARTUP_INDEX_DELAY_ENV, DEFAULT_STARTUP_INDEX_DELAY_SECS) };
    let aging = if heavy > zero {
        heavy + Duration::from_secs(APP_MANAGED_AGING_STARTUP_OFFSET_SECS)
    } else {
        startup_delay_from_env(STARTUP_AGING_DELAY_ENV, DEFAULT_STARTUP_AGING_DELAY_SECS)
    };
    let embed = if heavy > zero {
        heavy + Duration::from_secs(APP_MANAGED_EMBED_STARTUP_OFFSET_SECS)
    } else {
        startup_delay_from_env(STARTUP_EMBED_DELAY_ENV, DEFAULT_STARTUP_EMBED_DELAY_SECS)
    };
    let crystallize = if heavy > zero {
        heavy + Duration::from_secs(APP_MANAGED_CRYSTALLIZE_STARTUP_OFFSET_SECS)
    } else {
        startup_delay_from_env(STARTUP_CRYSTALLIZE_DELAY_ENV, DEFAULT_STARTUP_CRYSTALLIZE_DELAY_SECS)
    };
    let storage_governor_initial = startup_delay_from_env(STARTUP_STORAGE_GOVERNOR_DELAY_ENV, DEFAULT_STARTUP_STORAGE_GOVERNOR_DELAY_SECS);
    StartupSchedule { index, aging, embed, crystallize, storage_governor_initial }
}
pub(crate) fn background_db_lock_max_wait() -> Duration {
    let max_wait_ms =
        parse_env_u64_nonnegative(BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, BACKGROUND_DB_LOCK_DEFAULT_MAX_WAIT_MS).clamp(100, 60_000);
    Duration::from_millis(max_wait_ms)
}
pub(crate) async fn acquire_background_db_lock<'a>(
    db: &'a std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>, task_name: &str, max_wait: Duration,
) -> Option<tokio::sync::MutexGuard<'a, rusqlite::Connection>> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(conn) = db.try_lock() {
            return Some(conn);
        }
        if started.elapsed() >= max_wait {
            eprintln!("[cortex] Skipping {task_name}: DB lock busy for {}ms", started.elapsed().as_millis());
            return None;
        }
        tokio::time::sleep(Duration::from_millis(BACKGROUND_DB_LOCK_RETRY_MS)).await;
    }
}
pub(crate) fn process_pid_start_time(pid: u32) -> Option<u64> {
    let mut system = sysinfo::System::new_all();
    let target = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
    system.process(target).map(|process| process.start_time())
}
pub(crate) fn process_pid_identity_matches(pid: u32, expected_start_time: u64) -> bool {
    process_pid_start_time(pid)
        .map(|actual_start_time| actual_start_time == expected_start_time)
        .unwrap_or(false)
}
pub(crate) fn spawn_parent_orphan_watch_task<F>(
    shutdown_tx: std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>, parent_pid: u32, parent_start_time: u64,
    watch_interval: Duration, identity_matches: F,
) -> tokio::task::JoinHandle<()>
where
    F: Fn(u32, u64) -> bool + Send + Sync + 'static,
{
    let identity_matches = std::sync::Arc::new(identity_matches);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(watch_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            if !(identity_matches)(parent_pid, parent_start_time) {
                eprintln!("[cortex] Spawn parent process {parent_pid} exited or was recycled; shutting down daemon");
                if let Some(tx) = shutdown_tx.lock().await.take() {
                    let _ = tx.send(());
                }
                break;
            }
        }
    })
}
pub(crate) fn process_looks_like_cortex_daemon(process: &sysinfo::Process) -> bool {
    let cmd: Vec<String> = process.cmd().iter().map(|arg| arg.to_string_lossy().to_ascii_lowercase()).collect();
    if cmd.is_empty() {
        return false;
    }
    let has_daemon_role =
        cmd.iter().any(|arg| arg == "serve" || arg == "service-run") || cmd.windows(2).any(|pair| pair[0] == "service" && pair[1] == "run");
    if !has_daemon_role {
        return false;
    }
    let exe_is_cortex = process
        .exe()
        .and_then(|path| path.file_stem().or(path.file_name()))
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case("cortex"))
        .unwrap_or(false);
    let cmd_is_cortex = cmd.first().map(|first| first.contains("cortex")).unwrap_or(false);
    exe_is_cortex || cmd_is_cortex
}
pub(crate) fn detect_other_cortex_daemon_process() -> Option<(u32, String, String)> {
    let current_pid = std::process::id();
    let mut system = sysinfo::System::new_all();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    for (pid, process) in system.processes() {
        let pid_u32 = pid.as_u32();
        if pid_u32 == current_pid {
            continue;
        }
        if !process_looks_like_cortex_daemon(process) {
            continue;
        }
        let exe = process.exe().map(|path| path.display().to_string()).unwrap_or_else(|| "<unknown>".to_string());
        let cmd = process.cmd().iter().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ");
        return Some((pid_u32, exe, cmd));
    }
    None
}
pub(crate) fn spawned_owner_requires_parent_pid(owner_tag: Option<&str>) -> bool {
    owner_tag.map(|owner| should_watch_spawn_parent(Some(owner))).unwrap_or(false)
}
pub(crate) fn validate_spawned_owner_runtime_claim(
    paths: &auth::CortexPaths, owner_tag: Option<&str>, parent_pid: Option<u32>, parent_start_time: Option<u64>, owner_token: Option<&str>,
) -> Result<(), String> {
    if spawned_owner_requires_parent_pid(owner_tag) && parent_pid.is_none() {
        return Err(format!("owner '{}' requires {} linkage", owner_tag.unwrap_or("unknown"), SPAWN_PARENT_PID_ENV));
    }
    if spawned_owner_requires_parent_pid(owner_tag) && parent_start_time.is_none() {
        return Err(format!("owner '{}' requires {} linkage", owner_tag.unwrap_or("unknown"), SPAWN_PARENT_START_TIME_ENV));
    }
    if let (Some(parent_pid), Some(parent_start_time)) = (parent_pid, parent_start_time) {
        let Some(actual_start_time) = process_pid_start_time(parent_pid) else {
            return Err(format!("spawn parent process {parent_pid} is not running during ownership claim validation"));
        };
        if actual_start_time != parent_start_time {
            return Err(format!(
                "spawn parent start-time mismatch for pid {parent_pid} (env={parent_start_time}, actual={actual_start_time})"
            ));
        }
    }
    validate_spawned_owner_claim(paths, owner_tag, parent_pid, owner_token)
}
pub(crate) async fn startup_single_daemon_preflight(paths: &auth::CortexPaths) -> Result<(), String> {
    if let Some((pid, exe, cmd)) = detect_other_cortex_daemon_process() {
        if single_daemon_test_bypass_enabled() {
            eprintln!(
"[cortex] Warning: bypassing single-daemon process preflight for debug test run (detected pid={pid}, exe={exe}, cmd=\"{cmd}\")");
        } else {
            return Err(format!(
                "daemon startup denied: Cortex already has an active daemon process (pid={pid}, exe={exe}, cmd=\"{cmd}\")"
            ));
        }
    }
    let bind_addr = paths.bind.trim();
    let bind_error = match std::net::TcpListener::bind((bind_addr, paths.port)) {
        Ok(listener) => {
            drop(listener);
            return Ok(());
        }
        Err(err) => err,
    };
    let readiness_url = format!("{}/readiness", local_daemon_base_url(paths));
    let health_url = format!("{}/health", local_daemon_base_url(paths));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|err| format!("daemon startup preflight: build HTTP client: {err}"))?;
    let (mut status, mut body) = match transport::request_url_with_local_ipc_fallback(
        &client,
        "GET",
        &readiness_url,
        paths,
        &[],
        None,
        Duration::from_secs(2),
    )
    .await
    {
        Ok((status, body)) => (status.as_u16(), body),
        Err(readiness_err) => {
            match transport::request_url_with_local_ipc_fallback(&client, "GET", &health_url, paths, &[], None, Duration::from_secs(2))
                .await
            {
                Ok((status, body)) => (status.as_u16(), body),
                Err(health_err) => {
                    return Err(format!(
"daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}) and readiness probe at {readiness_url} failed ({readiness_err}); fallback health probe at {health_url} also failed ({health_err})"
,paths.port));
                }
            }
        }
    };
    if let Some(ready) = readiness_state_from_payload(status, &body, Some(paths.port), Some(paths)) {
        return if ready {
            Err(format!("daemon startup denied: canonical Cortex instance is already ready on port {}", paths.port))
        } else {
            Err(format!("daemon startup denied: canonical Cortex instance is already starting on port {}", paths.port))
        };
    }
    if readiness_state_from_payload(status, &body, Some(paths.port), None).is_some() {
        return Err(format!("daemon startup denied: port {} is served by a different Cortex runtime identity", paths.port));
    }
    if let Ok((health_status, health_body)) =
        transport::request_url_with_local_ipc_fallback(&client, "GET", &health_url, paths, &[], None, Duration::from_secs(2)).await
    {
        status = health_status.as_u16();
        body = health_body;
    }
    if is_cortex_health_payload(status, &body, Some(paths.port), Some(paths)) {
        return Err(format!("daemon startup denied: canonical Cortex instance is already healthy on port {}", paths.port));
    }
    if is_cortex_health_payload(status, &body, Some(paths.port), None) {
        return Err(format!("daemon startup denied: port {} is served by a different Cortex runtime identity", paths.port));
    }
    Err(format!(
"daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}); readiness probe at {readiness_url} returned non-canonical payload (HTTP {status})"
,paths.port))
}
pub(crate) fn app_init_required_client_name(agent: Option<&str>) -> String {
    env_trimmed(APP_CLIENT_ENV)
        .or_else(|| normalize_option(agent))
        .unwrap_or_else(|| "client".to_string())
}
pub(crate) fn app_init_required_error(paths: &auth::CortexPaths, agent: Option<&str>) -> String {
    let client = app_init_required_client_name(agent);
    format!(
"APP_INIT_REQUIRED: {client} is attach-only and cannot start the daemon automatically on port {}. Start Cortex Control Center and initialize the app-managed daemon, then retry."
,paths.port)
}
pub(crate) fn local_spawn_allowed_for_request(allow_service_ensure: bool) -> bool {
    if !allow_service_ensure {
        return false;
    }
    let app_client_marked = env_trimmed(APP_CLIENT_ENV).is_some();
    let local_spawn_raw = std::env::var(DAEMON_LOCAL_SPAWN_ENV).ok();
    let local_spawn_disabled = local_spawn_raw.as_ref().is_some_and(|value| !parse_truthy_flag(value));
    let app_required = std::env::var(APP_REQUIRED_ENV).ok().is_some_and(|value| parse_truthy_flag(&value));
    if app_client_marked && local_spawn_raw.is_none() {
        return false;
    }
    !(local_spawn_disabled || app_required)
}
pub(crate) fn control_center_lock_path(paths: &auth::CortexPaths) -> PathBuf {
    paths.home.join("runtime").join(CONTROL_CENTER_LOCK_FILE)
}
pub(crate) fn is_lock_contention_error(err: &std::io::Error) -> bool {
    if matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied) {
        return true;
    }
    if cfg!(windows) {
        return matches!(err.raw_os_error(), Some(32 | 33));
    }
    false
}
pub(crate) fn control_center_is_active(paths: &auth::CortexPaths) -> Result<bool, String> {
    let lock_path = control_center_lock_path(paths);
    let lock_file = match std::fs::OpenOptions::new().create(false).read(true).write(true).open(&lock_path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) if is_lock_contention_error(&err) => return Ok(true),
        Err(err) => {
            return Err(format!("open control-center lock {}: {err}", lock_path.display()));
        }
    };
    match lock_file.try_lock_exclusive() {
        Ok(()) => {
            let _ = lock_file.unlock();
            Ok(false)
        }
        Err(err) if is_lock_contention_error(&err) => Ok(true),
        Err(err) => Err(format!("probe control-center lock {}: {err}", lock_path.display())),
    }
}
pub(crate) async fn ensure_daemon(
    paths: &auth::CortexPaths, agent: Option<&str>, emit_port: bool, allow_service_ensure: bool,
) -> Result<(), String> {
    std::fs::create_dir_all(&paths.home).map_err(|e| format!("create home dir: {e}"))?;
    let local_spawn_allowed = local_spawn_allowed_for_request(allow_service_ensure);
    let control_center_active_snapshot = if local_spawn_allowed { control_center_is_active(paths).ok() } else { None };
    let lock = auth::acquire_daemon_lock(paths);
    match lock {
        Ok(_guard) => {
            if daemon_healthy(paths).await {
            } else if local_spawn_allowed {
                let _ = auth::migrate_legacy_db(paths)?;
                if control_center_active_snapshot == Some(true) {
                    return Err(app_init_required_error(paths, agent));
                }
                match control_center_is_active(paths) {
                    Ok(true) => return Err(app_init_required_error(paths, agent)),
                    Ok(false) => {}
                    Err(err) => {
                        return Err(format!("{} (control-center lock probe failed: {})", app_init_required_error(paths, agent), err));
                    }
                }
                #[cfg(windows)]
                {
                    if !ensure_service_ready_async().await {
                        return Err(format!(
                            "daemon is not healthy on port {} and Windows service ensure failed. Run `cortex service ensure` manually.",
                            paths.port
                        ));
                    }
                }
                #[cfg(not(windows))]
                {
                    ensure_local_plugin_spawn_async(paths, agent).await?;
                }
            } else {
                return Err(app_init_required_error(paths, agent));
            }
        }
        Err(_) => {
            if !wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
                if local_spawn_allowed {
                    if control_center_active_snapshot == Some(true) {
                        return Err(app_init_required_error(paths, agent));
                    }
                    match control_center_is_active(paths) {
                        Ok(true) => return Err(app_init_required_error(paths, agent)),
                        Ok(false) => {}
                        Err(err) => {
                            return Err(format!("{} (control-center lock probe failed: {})", app_init_required_error(paths, agent), err));
                        }
                    }
                    #[cfg(windows)]
                    {
                        if ensure_service_ready_async().await {
                        } else {
                            return Err(format!(
                                "daemon is not healthy on port {} and Windows service ensure failed while daemon lock was held.",
                                paths.port
                            ));
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        return Err(format!(
"daemon is not healthy on port {} and another process still holds the daemon lock. Retry after the in-flight startup finishes.",
paths.port));
                    }
                } else {
                    return Err(app_init_required_error(paths, agent));
                }
            }
        }
    }
    if let Some(agent) = agent {
        if let Err(e) = boot_agent(paths, agent).await {
            eprintln!("[cortex-plugin] Warning: boot call failed for agent '{agent}': {e}");
        }
    }
    if emit_port {
        println!("{}", paths.port);
    }
    Ok(())
}
#[cfg(windows)]
pub(crate) async fn ensure_service_ready_async() -> bool {
    tokio::task::spawn_blocking(service::ensure_ready).await.unwrap_or(false)
}
#[cfg(not(windows))]
pub(crate) fn plugin_owner_tag(agent: Option<&str>) -> String {
    let normalized = agent
        .unwrap_or("plugin")
        .trim()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        "plugin".to_string()
    } else {
        format!("plugin-{normalized}")
    }
}
pub(crate) fn normalized_path_for_guard(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
}
pub(crate) fn path_is_under_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalized_path_for_guard(path);
    let mut normalized_root = normalized_path_for_guard(root);
    if !normalized_root.ends_with('/') {
        normalized_root.push('/');
    }
    normalized_path == normalized_root.trim_end_matches('/') || normalized_path.starts_with(&normalized_root)
}
pub(crate) fn is_disallowed_startup_binary_path(path: &Path) -> bool {
    let normalized = normalized_path_for_guard(path);
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    if file_name.starts_with("cortex-daemon-run") {
        return true;
    }
    if normalized.contains("/daemon-lifecycle-runtime/") {
        return true;
    }
    let mut temp_roots = vec![std::env::temp_dir()];
    if let Ok(temp) = std::env::var("TEMP") {
        temp_roots.push(PathBuf::from(temp));
    }
    if let Ok(tmp) = std::env::var("TMP") {
        temp_roots.push(PathBuf::from(tmp));
    }
    temp_roots.iter().any(|root| !root.as_os_str().is_empty() && path_is_under_root(path, root))
}
#[cfg(not(windows))]
pub(crate) async fn ensure_local_plugin_spawn_async(paths: &auth::CortexPaths, agent: Option<&str>) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("resolve cortex binary: {e}"))?;
    if is_disallowed_startup_binary_path(&current_exe) {
        return Err(format!("refusing to launch daemon from disallowed runtime path: {}", current_exe.display()));
    }
    let parent_pid = std::process::id();
    let parent_start = process_pid_start_time(parent_pid).ok_or_else(|| format!("resolve spawn parent start time for pid {parent_pid}"))?;
    let owner_tag = plugin_owner_tag(agent);
    let owner_token = issue_owner_token_for_spawn(paths, &owner_tag, parent_pid).map_err(|e| format!("issue owner token: {e}"))?;
    let mut cmd = std::process::Command::new(current_exe);
    cmd.arg("serve")
        .arg("--home")
        .arg(paths.home.display().to_string())
        .arg("--db")
        .arg(paths.db.display().to_string())
        .arg("--port")
        .arg(paths.port.to_string())
        .arg("--bind")
        .arg(paths.bind.as_str())
        .env("CORTEX_DAEMON_OWNER", &owner_tag)
        .env("CORTEX_DAEMON_OWNER_SOURCE", "plugin-local")
        .env("CORTEX_DAEMON_OWNER_AGENT", agent.unwrap_or("plugin"))
        .env("CORTEX_DAEMON_OWNER_MODE", "local-plugin")
        .env(SPAWN_PARENT_PID_ENV, parent_pid.to_string())
        .env(SPAWN_PARENT_START_TIME_ENV, parent_start.to_string())
        .env(DAEMON_OWNER_TOKEN_ENV, owner_token)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn().map_err(|e| format!("spawn local daemon from plugin mode: {e}"))?;
    if wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
        Ok(())
    } else {
        Err(format!("daemon spawn started but health is still unavailable on port {}", paths.port))
    }
}
