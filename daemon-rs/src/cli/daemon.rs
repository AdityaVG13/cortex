// SPDX-License-Identifier: MIT

use chrono::Utc;
use fs2::FileExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::admin;
use crate::aging;
use crate::auth;
use crate::budgets;
use crate::compaction;
use crate::crystallize;
use crate::db;
use crate::daemon_lifecycle;
use crate::embeddings;
use crate::indexer;
use crate::server;
use crate::state;
use crate::transport;


use super::boot::boot_agent;
use super::cleanup::{
    cleanup_backup_retention, cleanup_bridge_backups, cleanup_expired_rows, create_backup,
    rotate_startup_logs, should_backup,
};
use super::common::{
    env_trimmed, local_daemon_base_url, normalize_option, parse_env_u64, parse_env_usize,
    parse_truthy_flag, single_daemon_test_bypass_enabled,
};

#[cfg(not(windows))]
use daemon_lifecycle::issue_owner_token_for_spawn;
use daemon_lifecycle::{
    daemon_healthy, is_cortex_health_payload, readiness_state_from_payload,
    validate_spawned_owner_claim, wait_for_health, DAEMON_OWNER_TOKEN_ENV,
    SPAWN_PARENT_START_TIME_ENV,
};

const CONTROL_CENTER_LOCK_FILE: &str = "control-center.lock";
const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
const SINGLE_DAEMON_TEST_BYPASS_ENV: &str = "CORTEX_SINGLE_DAEMON_TEST_BYPASS";
const SPAWN_PARENT_PID_ENV: &str = "CORTEX_SPAWN_PARENT_PID";
const ORPHAN_WATCH_INTERVAL_SECS: u64 = 2;
pub(crate) const DEFAULT_EMBED_BACKFILL_BATCH_SIZE: usize = 200;
pub(crate) const DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS: usize = 8;
const DEFAULT_EMBED_BACKFILL_INTERVAL_SECS: u64 = 120;
const DEFAULT_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES: usize = 64;
const DEFAULT_STARTUP_INDEX_DELAY_SECS: u64 = 5;
const DEFAULT_STARTUP_AGING_DELAY_SECS: u64 = 20;
const DEFAULT_STARTUP_EMBED_DELAY_SECS: u64 = 30;
const DEFAULT_STARTUP_CRYSTALLIZE_DELAY_SECS: u64 = 45;
const DEFAULT_STARTUP_STORAGE_GOVERNOR_DELAY_SECS: u64 = 12;
const STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES: usize = 3;
const STARTUP_STORAGE_GOVERNOR_CATCHUP_INTERVAL_SECS: u64 = 90;
const BACKGROUND_DB_LOCK_RETRY_MS: u64 = 50;
const BACKGROUND_DB_LOCK_DEFAULT_MAX_WAIT_MS: u64 = 2_000;
const APP_MANAGED_STARTUP_HEAVY_DELAY_SECS: u64 = 45;
const APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS: u64 = 120;
const APP_MANAGED_AGING_STARTUP_OFFSET_SECS: u64 = 15;
const APP_MANAGED_EMBED_STARTUP_OFFSET_SECS: u64 = 30;
const APP_MANAGED_CRYSTALLIZE_STARTUP_OFFSET_SECS: u64 = 45;
const DEFAULT_IDLE_SHUTDOWN_CHECK_INTERVAL_SECS: u64 = 5;
const DEFAULT_IDLE_SHUTDOWN_MIN_UPTIME_SECS: u64 = 120;
const STARTUP_INDEX_DELAY_ENV: &str = "CORTEX_STARTUP_INDEX_DELAY_SECS";
const STARTUP_AGING_DELAY_ENV: &str = "CORTEX_STARTUP_AGING_DELAY_SECS";
const STARTUP_EMBED_DELAY_ENV: &str = "CORTEX_STARTUP_EMBED_DELAY_SECS";
const STARTUP_CRYSTALLIZE_DELAY_ENV: &str = "CORTEX_STARTUP_CRYSTALLIZE_DELAY_SECS";
const STARTUP_STORAGE_GOVERNOR_DELAY_ENV: &str = "CORTEX_STARTUP_STORAGE_GOVERNOR_DELAY_SECS";
const BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV: &str = "CORTEX_BACKGROUND_DB_LOCK_MAX_WAIT_MS";
const EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV: &str = "CORTEX_EMBED_BACKFILL_DRAIN_ON_STARTUP";
const EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES_ENV: &str =
    "CORTEX_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES";
const IDLE_SHUTDOWN_SECS_ENV: &str = "CORTEX_IDLE_SHUTDOWN_SECS";
const IDLE_SHUTDOWN_MIN_UPTIME_SECS_ENV: &str = "CORTEX_IDLE_SHUTDOWN_MIN_UPTIME_SECS";
const STARTUP_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];

const DAEMON_STARTUP_WAIT_SECS: u64 = 90;
const DEFAULT_BOOT_BUDGET: usize = 600;
const DEFAULT_DAEMON_LOCK_WAIT_SECS: u64 = 15;
const DAEMON_LOCK_RETRY_INTERVAL_MS: u64 = 100;
const DAEMON_LOCK_HANDOFF_GRACE_SECS: u64 = 3;
const DAEMON_LOCAL_SPAWN_ENV: &str = "CORTEX_DAEMON_OWNER_LOCAL_SPAWN";
const APP_REQUIRED_ENV: &str = "CORTEX_APP_REQUIRED";
const APP_CLIENT_ENV: &str = "CORTEX_APP_CLIENT";
const APP_MANAGED_STARTUP_DELAY_ENV: &str = "CORTEX_APP_MANAGED_STARTUP_DELAY_SECS";

/// Hold the singleton daemon lock before startup so duplicate `serve`
/// invocations cannot rotate the shared auth token and then die on bind.
fn daemon_lock_wait_timeout() -> Duration {
    let secs = std::env::var("CORTEX_DAEMON_LOCK_WAIT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DAEMON_LOCK_WAIT_SECS);
    Duration::from_secs(secs.max(1))
}

#[derive(Debug)]
struct RuntimeLockGuards {
    _scoped: std::fs::File,
    _global: Option<std::fs::File>,
}

fn try_acquire_runtime_locks(paths: &auth::CortexPaths) -> Result<RuntimeLockGuards, String> {
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
    Ok(RuntimeLockGuards {
        _scoped: scoped,
        _global: global,
    })
}

fn acquire_runtime_lock(paths: &auth::CortexPaths) -> Result<RuntimeLockGuards, String> {
    let _ = auth::cleanup_stale_pid_lock(paths);
    if std::env::var("CORTEX_WAIT_FOR_DAEMON_LOCK")
        .ok()
        .is_some_and(|value| value == "1")
    {
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

        // Sleep/wake edge: lock ownership can hand off shortly after timeout due scheduler jitter.
        let grace_deadline =
            std::time::Instant::now() + Duration::from_secs(DAEMON_LOCK_HANDOFF_GRACE_SECS);
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

fn daemon_owner_tag_from_env() -> Option<String> {
    std::env::var("CORTEX_DAEMON_OWNER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn daemon_owner_token_from_env() -> Option<String> {
    std::env::var(DAEMON_OWNER_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn spawn_parent_pid_from_env() -> Option<u32> {
    std::env::var(SPAWN_PARENT_PID_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
}

fn spawn_parent_start_time_from_env() -> Option<u64> {
    std::env::var(SPAWN_PARENT_START_TIME_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn should_watch_spawn_parent(owner_tag: Option<&str>) -> bool {
    owner_tag
        .map(|owner| !owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG))
        .unwrap_or(true)
}

fn is_control_center_owner(owner_tag: Option<&str>) -> bool {
    owner_tag
        .map(|owner| owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG))
        .unwrap_or(false)
}

fn parse_env_u64_nonnegative(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn app_managed_startup_heavy_delay(owner_tag: Option<&str>) -> Duration {
    if !is_control_center_owner(owner_tag) {
        return Duration::from_secs(0);
    }
    let secs = parse_env_u64_nonnegative(
        APP_MANAGED_STARTUP_DELAY_ENV,
        APP_MANAGED_STARTUP_HEAVY_DELAY_SECS,
    )
    .min(APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS);
    Duration::from_secs(secs)
}

#[derive(Clone, Copy)]
struct StartupSchedule {
    index: Duration,
    aging: Duration,
    embed: Duration,
    crystallize: Duration,
    storage_governor_initial: Duration,
}

fn startup_delay_from_env(key: &str, default: u64) -> Duration {
    Duration::from_secs(parse_env_u64_nonnegative(key, default).min(3_600))
}

fn startup_schedule(owner_tag: Option<&str>) -> StartupSchedule {
    let zero = Duration::from_secs(0);
    let heavy = app_managed_startup_heavy_delay(owner_tag);
    let index = if heavy > zero {
        heavy
    } else {
        startup_delay_from_env(STARTUP_INDEX_DELAY_ENV, DEFAULT_STARTUP_INDEX_DELAY_SECS)
    };
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
        startup_delay_from_env(
            STARTUP_CRYSTALLIZE_DELAY_ENV,
            DEFAULT_STARTUP_CRYSTALLIZE_DELAY_SECS,
        )
    };
    let storage_governor_initial = startup_delay_from_env(
        STARTUP_STORAGE_GOVERNOR_DELAY_ENV,
        DEFAULT_STARTUP_STORAGE_GOVERNOR_DELAY_SECS,
    );
    StartupSchedule {
        index,
        aging,
        embed,
        crystallize,
        storage_governor_initial,
    }
}

pub(crate) fn background_db_lock_max_wait() -> Duration {
    let max_wait_ms = parse_env_u64_nonnegative(
        BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV,
        BACKGROUND_DB_LOCK_DEFAULT_MAX_WAIT_MS,
    )
    .clamp(100, 60_000);
    Duration::from_millis(max_wait_ms)
}

async fn acquire_background_db_lock<'a>(
    db: &'a std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    task_name: &str,
    max_wait: Duration,
) -> Option<tokio::sync::MutexGuard<'a, rusqlite::Connection>> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(conn) = db.try_lock() {
            return Some(conn);
        }
        if started.elapsed() >= max_wait {
            eprintln!(
                "[cortex] Skipping {task_name}: DB lock busy for {}ms",
                started.elapsed().as_millis()
            );
            return None;
        }
        tokio::time::sleep(Duration::from_millis(BACKGROUND_DB_LOCK_RETRY_MS)).await;
    }
}

fn process_pid_start_time(pid: u32) -> Option<u64> {
    let mut system = sysinfo::System::new_all();
    let target = sysinfo::Pid::from_u32(pid);
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
    system.process(target).map(|process| process.start_time())
}

fn process_pid_identity_matches(pid: u32, expected_start_time: u64) -> bool {
    process_pid_start_time(pid)
        .map(|actual_start_time| actual_start_time == expected_start_time)
        .unwrap_or(false)
}

fn spawn_parent_orphan_watch_task<F>(
    shutdown_tx: std::sync::Arc<tokio::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    parent_pid: u32,
    parent_start_time: u64,
    watch_interval: Duration,
    identity_matches: F,
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
                eprintln!(
                    "[cortex] Spawn parent process {parent_pid} exited or was recycled; shutting down daemon"
                );
                if let Some(tx) = shutdown_tx.lock().await.take() {
                    let _ = tx.send(());
                }
                break;
            }
        }
    })
}

fn process_looks_like_cortex_daemon(process: &sysinfo::Process) -> bool {
    let cmd: Vec<String> = process
        .cmd()
        .iter()
        .map(|arg| arg.to_string_lossy().to_ascii_lowercase())
        .collect();
    if cmd.is_empty() {
        return false;
    }
    let has_daemon_role = cmd.iter().any(|arg| arg == "serve" || arg == "service-run")
        || cmd
            .windows(2)
            .any(|pair| pair[0] == "service" && pair[1] == "run");
    if !has_daemon_role {
        return false;
    }

    let exe_is_cortex = process
        .exe()
        .and_then(|path| path.file_stem().or(path.file_name()))
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case("cortex"))
        .unwrap_or(false);
    let cmd_is_cortex = cmd
        .first()
        .map(|first| first.contains("cortex"))
        .unwrap_or(false);
    exe_is_cortex || cmd_is_cortex
}

fn detect_other_cortex_daemon_process() -> Option<(u32, String, String)> {
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
        let exe = process
            .exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        let cmd = process
            .cmd()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        return Some((pid_u32, exe, cmd));
    }
    None
}

fn spawned_owner_requires_parent_pid(owner_tag: Option<&str>) -> bool {
    owner_tag
        .map(|owner| should_watch_spawn_parent(Some(owner)))
        .unwrap_or(false)
}

fn validate_spawned_owner_runtime_claim(
    paths: &auth::CortexPaths,
    owner_tag: Option<&str>,
    parent_pid: Option<u32>,
    parent_start_time: Option<u64>,
    owner_token: Option<&str>,
) -> Result<(), String> {
    if spawned_owner_requires_parent_pid(owner_tag) && parent_pid.is_none() {
        return Err(format!(
            "owner '{}' requires {} linkage",
            owner_tag.unwrap_or("unknown"),
            SPAWN_PARENT_PID_ENV
        ));
    }
    if spawned_owner_requires_parent_pid(owner_tag) && parent_start_time.is_none() {
        return Err(format!(
            "owner '{}' requires {} linkage",
            owner_tag.unwrap_or("unknown"),
            SPAWN_PARENT_START_TIME_ENV
        ));
    }

    if let (Some(parent_pid), Some(parent_start_time)) = (parent_pid, parent_start_time) {
        let Some(actual_start_time) = process_pid_start_time(parent_pid) else {
            return Err(format!(
                "spawn parent process {parent_pid} is not running during ownership claim validation"
            ));
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
                "[cortex] Warning: bypassing single-daemon process preflight for debug test run (detected pid={pid}, exe={exe}, cmd=\"{cmd}\")"
            );
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
            // Backward compatibility for daemons that do not expose /readiness yet.
            match transport::request_url_with_local_ipc_fallback(
                &client,
                "GET",
                &health_url,
                paths,
                &[],
                None,
                Duration::from_secs(2),
            )
            .await
            {
                Ok((status, body)) => (status.as_u16(), body),
                Err(health_err) => {
                    return Err(format!(
                        "daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}) and readiness probe at {readiness_url} failed ({readiness_err}); fallback health probe at {health_url} also failed ({health_err})",
                        paths.port
                    ));
                }
            }
        }
    };

    if let Some(ready) = readiness_state_from_payload(status, &body, Some(paths.port), Some(paths))
    {
        return if ready {
            Err(format!(
                "daemon startup denied: canonical Cortex instance is already ready on port {}",
                paths.port
            ))
        } else {
            Err(format!(
                "daemon startup denied: canonical Cortex instance is already starting on port {}",
                paths.port
            ))
        };
    }
    if readiness_state_from_payload(status, &body, Some(paths.port), None).is_some() {
        return Err(format!(
            "daemon startup denied: port {} is served by a different Cortex runtime identity",
            paths.port
        ));
    }

    // Fallback for legacy daemons (or intermediaries that do not proxy readiness):
    // probe /health and apply canonical identity checks there.
    if let Ok((health_status, health_body)) = transport::request_url_with_local_ipc_fallback(
        &client,
        "GET",
        &health_url,
        paths,
        &[],
        None,
        Duration::from_secs(2),
    )
    .await
    {
        status = health_status.as_u16();
        body = health_body;
    }

    if is_cortex_health_payload(status, &body, Some(paths.port), Some(paths)) {
        return Err(format!(
            "daemon startup denied: canonical Cortex instance is already healthy on port {}",
            paths.port
        ));
    }
    if is_cortex_health_payload(status, &body, Some(paths.port), None) {
        return Err(format!(
            "daemon startup denied: port {} is served by a different Cortex runtime identity",
            paths.port
        ));
    }

    Err(format!(
        "daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}); readiness probe at {readiness_url} returned non-canonical payload (HTTP {status})",
        paths.port
    ))
}

fn app_init_required_client_name(agent: Option<&str>) -> String {
    env_trimmed(APP_CLIENT_ENV)
        .or_else(|| normalize_option(agent))
        .unwrap_or_else(|| "client".to_string())
}

fn app_init_required_error(paths: &auth::CortexPaths, agent: Option<&str>) -> String {
    let client = app_init_required_client_name(agent);
    format!(
        "APP_INIT_REQUIRED: {client} is attach-only and cannot start the daemon automatically on port {}. Start Cortex Control Center and initialize the app-managed daemon, then retry.",
        paths.port
    )
}

fn local_spawn_allowed_for_request(allow_service_ensure: bool) -> bool {
    if !allow_service_ensure {
        return false;
    }
    let app_client_marked = env_trimmed(APP_CLIENT_ENV).is_some();
    let local_spawn_raw = std::env::var(DAEMON_LOCAL_SPAWN_ENV).ok();
    let local_spawn_disabled = local_spawn_raw
        .as_ref()
        .is_some_and(|value| !parse_truthy_flag(value));
    let app_required = std::env::var(APP_REQUIRED_ENV)
        .ok()
        .is_some_and(|value| parse_truthy_flag(&value));
    // Fail closed for app-marked clients when no explicit local spawn policy exists.
    // This prevents partial registration env contracts from silently re-enabling local spawn.
    if app_client_marked && local_spawn_raw.is_none() {
        return false;
    }
    !(local_spawn_disabled || app_required)
}

fn control_center_lock_path(paths: &auth::CortexPaths) -> PathBuf {
    paths.home.join("runtime").join(CONTROL_CENTER_LOCK_FILE)
}

fn is_lock_contention_error(err: &std::io::Error) -> bool {
    if matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
    ) {
        return true;
    }
    if cfg!(windows) {
        return matches!(err.raw_os_error(), Some(32 | 33));
    }
    false
}

fn control_center_is_active(paths: &auth::CortexPaths) -> Result<bool, String> {
    let lock_path = control_center_lock_path(paths);
    let lock_file = match std::fs::OpenOptions::new()
        .create(false)
        .read(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) if is_lock_contention_error(&err) => return Ok(true),
        Err(err) => {
            return Err(format!(
                "open control-center lock {}: {err}",
                lock_path.display()
            ));
        }
    };

    match lock_file.try_lock_exclusive() {
        Ok(()) => {
            let _ = lock_file.unlock();
            Ok(false)
        }
        Err(err) if is_lock_contention_error(&err) => Ok(true),
        Err(err) => Err(format!(
            "probe control-center lock {}: {err}",
            lock_path.display()
        )),
    }
}

pub(crate) async fn ensure_daemon(
    paths: &auth::CortexPaths,
    agent: Option<&str>,
    emit_port: bool,
    allow_service_ensure: bool,
) -> Result<(), String> {
    std::fs::create_dir_all(&paths.home).map_err(|e| format!("create home dir: {e}"))?;
    let local_spawn_allowed = local_spawn_allowed_for_request(allow_service_ensure);
    let control_center_active_snapshot = if local_spawn_allowed {
        control_center_is_active(paths).ok()
    } else {
        None
    };

    let lock = auth::acquire_daemon_lock(paths);

    match lock {
        Ok(_guard) => {
            if daemon_healthy(paths).await {
                // already healthy
            } else if local_spawn_allowed {
                let _ = auth::migrate_legacy_db(paths)?;
                if control_center_active_snapshot == Some(true) {
                    return Err(app_init_required_error(paths, agent));
                }
                match control_center_is_active(paths) {
                    Ok(true) => return Err(app_init_required_error(paths, agent)),
                    Ok(false) => {}
                    Err(err) => {
                        return Err(format!(
                            "{} (control-center lock probe failed: {})",
                            app_init_required_error(paths, agent),
                            err
                        ));
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
                            return Err(format!(
                                "{} (control-center lock probe failed: {})",
                                app_init_required_error(paths, agent),
                                err
                            ));
                        }
                    }
                    #[cfg(windows)]
                    {
                        if ensure_service_ready_async().await {
                            // proceed
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
                            paths.port
                        ));
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
async fn ensure_service_ready_async() -> bool {
    tokio::task::spawn_blocking(service::ensure_ready)
        .await
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn plugin_owner_tag(agent: Option<&str>) -> String {
    let normalized = agent
        .unwrap_or("plugin")
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        "plugin".to_string()
    } else {
        format!("plugin-{normalized}")
    }
}

fn normalized_path_for_guard(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalized_path_for_guard(path);
    let mut normalized_root = normalized_path_for_guard(root);
    if !normalized_root.ends_with('/') {
        normalized_root.push('/');
    }
    normalized_path == normalized_root.trim_end_matches('/')
        || normalized_path.starts_with(&normalized_root)
}

pub(crate) fn is_disallowed_startup_binary_path(path: &Path) -> bool {
    let normalized = normalized_path_for_guard(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

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
    temp_roots
        .iter()
        .any(|root| !root.as_os_str().is_empty() && path_is_under_root(path, root))
}

#[cfg(not(windows))]
async fn ensure_local_plugin_spawn_async(
    paths: &auth::CortexPaths,
    agent: Option<&str>,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("resolve cortex binary: {e}"))?;
    if is_disallowed_startup_binary_path(&current_exe) {
        return Err(format!(
            "refusing to launch daemon from disallowed runtime path: {}",
            current_exe.display()
        ));
    }
    let parent_pid = std::process::id();
    let parent_start = process_pid_start_time(parent_pid)
        .ok_or_else(|| format!("resolve spawn parent start time for pid {parent_pid}"))?;
    let owner_tag = plugin_owner_tag(agent);
    let owner_token = issue_owner_token_for_spawn(paths, &owner_tag, parent_pid)
        .map_err(|e| format!("issue owner token: {e}"))?;

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

    cmd.spawn()
        .map_err(|e| format!("spawn local daemon from plugin mode: {e}"))?;

    if wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
        Ok(())
    } else {
        Err(format!(
            "daemon spawn started but health is still unavailable on port {}",
            paths.port
        ))
    }
}

// ── Shared daemon logic (used by `serve` and `service-run`) ─────────────────

/// Run the full Cortex daemon. The `extra_shutdown` future is an additional
/// shutdown trigger beyond the HTTP /shutdown endpoint:
/// - `serve` passes Ctrl+C / SIGTERM
/// - `service-run` passes the SCM stop signal
pub(crate) async fn run_daemon(
    paths: auth::CortexPaths,
    extra_shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let _daemon_lock = match acquire_runtime_lock(&paths) {
        Ok(lock) => lock,
        Err(err) => {
            if daemon_healthy(&paths).await {
                eprintln!(
                    "[cortex] Daemon already healthy on port {}; exiting cleanly.",
                    paths.port
                );
                return;
            }
            eprintln!("[cortex] FATAL: {err}");
            eprintln!(
                "[cortex] Reuse the existing daemon instead of launching a second `cortex serve`."
            );
            std::process::exit(1);
        }
    };

    let db_path = paths.db.clone();
    eprintln!(
        "[cortex] Starting Cortex v{} (Rust)...",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("[cortex] DB: {}", db_path.display());

    crate::install_daemon_panic_hook(&paths);

    let daemon_owner = daemon_owner_tag_from_env();
    let parent_pid = spawn_parent_pid_from_env();
    let parent_start_time = spawn_parent_start_time_from_env();
    let owner_token = daemon_owner_token_from_env();
    if let Err(reason) = validate_spawned_owner_runtime_claim(
        &paths,
        daemon_owner.as_deref(),
        parent_pid,
        parent_start_time,
        owner_token.as_deref(),
    ) {
        eprintln!("[cortex] FATAL: invalid spawned owner claim ({reason}); refusing startup");
        std::process::exit(1);
    }
    if let Err(reason) = startup_single_daemon_preflight(&paths).await {
        eprintln!("[cortex] FATAL: {reason}");
        std::process::exit(1);
    }

    let (state, shutdown_rx) = match state::initialize(&paths, true) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("[cortex] FATAL: failed to initialize state: {err}");
            std::process::exit(1);
        }
    };

    if should_watch_spawn_parent(daemon_owner.as_deref()) {
        if let (Some(parent_pid), Some(parent_start_time)) = (parent_pid, parent_start_time) {
            let _watcher = spawn_parent_orphan_watch_task(
                state.shutdown_tx.clone(),
                parent_pid,
                parent_start_time,
                Duration::from_secs(ORPHAN_WATCH_INTERVAL_SECS),
                process_pid_identity_matches,
            );
        }
    }

    let startup_schedule = startup_schedule(daemon_owner.as_deref());
    let background_lock_wait = background_db_lock_max_wait();
    eprintln!(
        "[cortex] Startup scheduling: index={}s, aging={}s, embeddings={}s, crystallize={}s, storage_governor={}s",
        startup_schedule.index.as_secs(),
        startup_schedule.aging.as_secs(),
        startup_schedule.embed.as_secs(),
        startup_schedule.crystallize.as_secs(),
        startup_schedule.storage_governor_initial.as_secs()
    );

    if let Some(parent) = paths.pid.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&paths.pid, std::process::id().to_string()).ok();

    let token_path = paths.token.clone();
    let pid_path = paths.pid.clone();
    let backup_dir = paths.home.join("backups");
    eprintln!("[cortex] Auth token at {}", token_path.display());
    eprintln!(
        "[cortex] PID {} written to {}",
        std::process::id(),
        pid_path.display()
    );
    let cleaned_backups = cleanup_backup_retention(&backup_dir);
    eprintln!(
        "[cortex] Cleaned {cleaned_backups} old backups, kept {}",
        super::cleanup::BACKUP_RETENTION_COUNT
    );
    let rotated_logs = rotate_startup_logs(&paths.home);
    if rotated_logs > 0 {
        eprintln!("[cortex] Rotated {rotated_logs} oversized log files");
    }

    // ── Recover WAL on startup ──────────────────────────────────────
    // Run WAL checkpoint to recover any pending writes from a previous crash.
    // This ensures committed transactions are flushed to the main DB file.
    {
        let conn = state.db.lock().await;
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            eprintln!("[cortex] WAL recovery warning: {e}");
        } else {
            eprintln!("[cortex] WAL recovery complete");
        }
    }

    // ── Schema migrations (idempotent) ──────────────────────────────
    let schema_version = {
        let conn = state.db.lock().await;
        let applied = db::run_pending_migrations(&conn);
        if applied > 0 {
            eprintln!("[cortex] Applied {applied} schema migrations");
        }
        db::current_schema_user_version(&conn).unwrap_or(0)
    };
    let _ = cleanup_bridge_backups(&paths.home, schema_version);

    // ── Startup indexing + decay (non-blocking) ─────────────────────
    // This used to run inline before the server bound its port, which could
    // delay startup significantly on large source trees.
    {
        let db_index = state.db.clone();
        let home = state.home.clone();
        let owner_id = state.default_owner_id;
        let startup_delay = startup_schedule.index;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            let started = std::time::Instant::now();
            if let Some(conn) =
                acquire_background_db_lock(&db_index, "startup indexing", lock_wait).await
            {
                let indexed = indexer::index_all(&conn, &home, owner_id);
                let decayed = indexer::decay_pass(&conn);
                eprintln!(
                    "[cortex] Startup indexing complete: indexed {indexed}, decayed {decayed} scores in {}ms",
                    started.elapsed().as_millis()
                );
            }
        });
    }

    // ── Background embedding builder ────────────────────────────────
    if let Some(engine) = state.embedding_engine.clone() {
        let db = state.db.clone();
        let batch_size = parse_env_usize(
            "CORTEX_EMBED_BACKFILL_BATCH_SIZE",
            DEFAULT_EMBED_BACKFILL_BATCH_SIZE,
        )
        .clamp(1, 10_000);
        let max_batches_per_pass = parse_env_usize(
            "CORTEX_EMBED_BACKFILL_MAX_BATCHES_PER_PASS",
            DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS,
        )
        .clamp(1, 1000);
        let interval_secs = parse_env_u64(
            "CORTEX_EMBED_BACKFILL_INTERVAL_SECS",
            DEFAULT_EMBED_BACKFILL_INTERVAL_SECS,
        )
        .clamp(5, 86_400);
        let drain_on_startup = std::env::var(EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV)
            .ok()
            .map(|value| parse_truthy_flag(&value))
            .unwrap_or(false);
        let startup_drain_max_batches = parse_env_usize(
            EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES_ENV,
            DEFAULT_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES,
        )
        .clamp(1, 10_000);
        let startup_delay = startup_schedule.embed;
        let lock_wait = background_lock_wait;
        let startup_max_batches_per_pass = if startup_delay > Duration::from_secs(0) {
            max_batches_per_pass.min(2)
        } else {
            max_batches_per_pass
        };
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            let startup_pass = build_embeddings_async(
                engine.clone(),
                &db,
                batch_size,
                startup_max_batches_per_pass,
                lock_wait,
            )
            .await;
            if startup_pass.queued_total > 0 && !startup_pass.exhausted {
                if drain_on_startup {
                    let drain_pass = build_embeddings_async(
                        engine.clone(),
                        &db,
                        batch_size,
                        startup_drain_max_batches,
                        lock_wait,
                    )
                    .await;
                    if drain_pass.exhausted {
                        eprintln!(
                            "[embeddings] Startup drain completed backlog in {} batches",
                            drain_pass.passes_ran
                        );
                    } else if drain_pass.queued_total > 0 {
                        eprintln!(
                            "[embeddings] Startup drain reached cap with backlog still pending (passes={}, queued={})",
                            drain_pass.passes_ran, drain_pass.queued_total
                        );
                    }
                } else {
                    eprintln!(
                        "[embeddings] Startup pass left backlog pending; set {}=1 to run a one-time extended drain",
                        EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV
                    );
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                build_embeddings_async(
                    engine.clone(),
                    &db,
                    batch_size,
                    max_batches_per_pass,
                    lock_wait,
                )
                .await;
            }
        });
    } else {
        let models_dir = paths.models.clone();
        tokio::spawn(async move {
            if let Some(dir) = embeddings::ensure_model_downloaded_in(&models_dir).await {
                eprintln!(
                    "[embeddings] Model ready at {} -- restart to activate",
                    dir.display()
                );
            }
        });
    }

    // ── Background WAL checkpoint every 10s (crash-safe) ──────────────
    {
        let db_wal = state.db.clone();
        let db_path = db_path.clone();
        let home_dir = paths.home.clone();
        let lock_wait = background_lock_wait.min(Duration::from_millis(750));
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;

                // Checkpoint WAL first to ensure consistency
                let Some(conn) =
                    acquire_background_db_lock(&db_wal, "wal checkpoint", lock_wait).await
                else {
                    continue;
                };
                {
                    db::checkpoint_wal_best_effort(&conn);
                }

                // Check if daily backup is needed
                let backup_dir = home_dir.join("backups");
                if should_backup(&backup_dir) {
                    if let Err(e) = create_backup(&db_path, &backup_dir) {
                        eprintln!("[cortex] Backup failed: {e}");
                    }
                }
            }
        });
    }

    // ── Background quick_check every 30 minutes ────────────────────────
    // Runs PRAGMA quick_check (B-tree only) to catch corruption that develops
    // during runtime.  On failure, sets db_corrupted so /health reflects it.
    {
        let db_qc = state.db_read.clone();
        let db_corrupted_flag = state.db_corrupted.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
            interval.tick().await; // skip first tick -- startup integrity_check already ran
            loop {
                interval.tick().await;
                let conn = db_qc.lock().await;
                if db::quick_check(&conn) {
                    // Clear the flag if a previous check had set it (e.g. after manual repair).
                    db_corrupted_flag.store(false, std::sync::atomic::Ordering::SeqCst);
                } else {
                    eprintln!(
                        "[cortex] WARNING: runtime PRAGMA quick_check FAILED -- \
                         database may be corrupted. Restart the daemon to trigger auto-repair. \
                         /health endpoint now shows degraded=true, db_corrupted=true."
                    );
                    db_corrupted_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
    }

    // ── Background aging pass every 6 hours ──────────────────────────
    {
        let db_aging = state.db.clone();
        let startup_delay = startup_schedule.aging;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            // Run initial aging pass on startup
            if let Some(conn) =
                acquire_background_db_lock(&db_aging, "initial aging pass", lock_wait).await
            {
                let (compressed, archived) = aging::run_aging_pass(&conn);
                if compressed > 0 || archived > 0 {
                    eprintln!(
                        "[cortex] Initial aging: {compressed} compressed, {archived} archived"
                    );
                }
                cleanup_expired_rows(&conn, "Initial expired cleanup");
            }
            // Then run every 6 hours
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) =
                    acquire_background_db_lock(&db_aging, "aging pass", lock_wait).await
                {
                    aging::run_aging_pass(&conn);
                    cleanup_expired_rows(&conn, "Expired cleanup");
                }
            }
        });
    }

    // ── Background storage governor ────────────────────────────────────
    {
        let db_compaction = state.db.clone();
        let startup_delay = startup_schedule.storage_governor_initial;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            // Catch-up passes soon after startup to relieve event pressure early.
            for pass in 0..STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES {
                if let Some(conn) = acquire_background_db_lock(
                    &db_compaction,
                    "startup storage governor",
                    lock_wait,
                )
                .await
                {
                    let ran = compaction::run_compaction_governor_startup(&conn).is_some();
                    if !ran {
                        break;
                    }
                }
                if pass + 1 < STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES {
                    tokio::time::sleep(Duration::from_secs(
                        STARTUP_STORAGE_GOVERNOR_CATCHUP_INTERVAL_SECS,
                    ))
                    .await;
                }
            }

            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) =
                    acquire_background_db_lock(&db_compaction, "storage governor", lock_wait).await
                {
                    let _ = compaction::run_compaction_governor(&conn);
                }
            }
        });
    }

    // ── Background crystallization pass every 2 hours ─────────────
    {
        let db_crystal = state.db.clone();
        let engine_crystal = state.embedding_engine.clone();
        let crystal_owner_id = state.default_owner_id;
        let brain_crystal: crystallize::BrainFiringSender = Some(state.brain_firing.clone());
        let initial_delay = startup_schedule.crystallize;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            // Initial pass on startup (after embeddings are built, with app-managed delay if needed)
            tokio::time::sleep(initial_delay).await;
            if let Some(conn) =
                acquire_background_db_lock(&db_crystal, "initial crystallization", lock_wait).await
            {
                let result = crystallize::run_crystallize_pass_with_brain(
                    &conn,
                    engine_crystal.as_deref(),
                    crystal_owner_id,
                    &brain_crystal,
                );
                if result.crystals_created > 0 || result.crystals_updated > 0 {
                    eprintln!(
                        "[cortex] Initial crystallization: {} created, {} updated",
                        result.crystals_created, result.crystals_updated
                    );
                }
            }
            // Then run every 2 hours
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) =
                    acquire_background_db_lock(&db_crystal, "crystallization pass", lock_wait).await
                {
                    crystallize::run_crystallize_pass_with_brain(
                        &conn,
                        engine_crystal.as_deref(),
                        crystal_owner_id,
                        &brain_crystal,
                    );
                }
            }
        });
    }

    // ── Background rate limiter cleanup every 5 minutes ────────────
    {
        let rl = state.rate_limiter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await;
            loop {
                interval.tick().await;
                rl.cleanup().await;
            }
        });
    }

    let idle_shutdown_secs = std::env::var(IDLE_SHUTDOWN_SECS_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let idle_min_uptime_secs = parse_env_u64(
        IDLE_SHUTDOWN_MIN_UPTIME_SECS_ENV,
        DEFAULT_IDLE_SHUTDOWN_MIN_UPTIME_SECS,
    )
    .clamp(1, 86_400);
    if idle_shutdown_secs > 0 {
        eprintln!(
            "[cortex] Idle shutdown enabled (timeout={}s, min_uptime={}s)",
            idle_shutdown_secs, idle_min_uptime_secs
        );
    }

    let readiness_signal = state.readiness.clone();
    let db_for_shutdown = state.db.clone();
    let state_for_idle_shutdown = state.clone();
    let router = server::build_router(state, paths.port);

    // Combine shutdown sources: HTTP /shutdown, extra (Ctrl+C or SCM stop)
    let shutdown_future = async move {
        let idle_shutdown_future = async move {
            if idle_shutdown_secs == 0 {
                std::future::pending::<()>().await;
                return;
            }
            tokio::time::sleep(Duration::from_secs(idle_min_uptime_secs)).await;
            let mut interval = tokio::time::interval(Duration::from_secs(
                DEFAULT_IDLE_SHUTDOWN_CHECK_INTERVAL_SECS,
            ));
            interval.tick().await;
            loop {
                interval.tick().await;
                let idle_for = state_for_idle_shutdown.idle_for_secs();
                if idle_for >= idle_shutdown_secs {
                    eprintln!(
                        "[cortex] Idle shutdown threshold reached (idle={}s >= {}s)",
                        idle_for, idle_shutdown_secs
                    );
                    break;
                }
            }
        };

        tokio::select! {
            _ = shutdown_rx => {
                eprintln!("[cortex] Shutdown requested via HTTP");
            }
            _ = extra_shutdown => {}
            _ = idle_shutdown_future => {}
        }
    };

    server::run(
        router,
        &paths.bind,
        paths.port,
        paths.ipc_endpoint.clone(),
        &db_path,
        Some(readiness_signal),
        shutdown_future,
    )
    .await;

    // WAL checkpoint + cleanup
    eprintln!("[cortex] Flushing database...");
    {
        let conn = db_for_shutdown.lock().await;
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            eprintln!("[cortex] Warning: WAL checkpoint failed: {e}");
        }
    }

    let _ = std::fs::remove_file(&pid_path);
    eprintln!("[cortex] Shutdown complete.");
}

/// Build embeddings for all un-embedded memories and decisions.
/// IMPORTANT: Does NOT hold the DB lock during ONNX inference.
/// Reads IDs/text in a short lock, embeds in memory (no lock), then writes in batches.
type EmbeddingBackfillRows = Vec<(i64, String)>;
type EmbeddingBackfillTargets = (EmbeddingBackfillRows, EmbeddingBackfillRows);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EmbeddingBackfillPassResult {
    pub(crate) queued_total: usize,
    pub(crate) computed_total: usize,
    pub(crate) passes_ran: usize,
    pub(crate) exhausted: bool,
}

fn backfill_batch_may_have_more(
    memory_count: usize,
    decision_count: usize,
    batch_size: usize,
) -> bool {
    memory_count >= batch_size || decision_count >= batch_size
}

fn collect_unembedded_targets_for_model(
    conn: &rusqlite::Connection,
    model_key: &str,
    limit: usize,
) -> EmbeddingBackfillTargets {
    let mem: EmbeddingBackfillRows = conn
        .prepare(
            "SELECT m.id, m.text FROM memories m \
             WHERE m.status = 'active' \
                AND NOT EXISTS (\
                    SELECT 1 FROM embeddings e \
                    WHERE e.target_type = 'memory' \
                      AND e.target_id = m.id \
                      AND LOWER(COALESCE(e.model, '')) = ?1\
                ) \
             ORDER BY m.id ASC \
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![model_key, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let dec: EmbeddingBackfillRows = conn
        .prepare(
            "SELECT d.id, d.decision FROM decisions d \
             WHERE d.status = 'active' \
                AND NOT EXISTS (\
                    SELECT 1 FROM embeddings e \
                    WHERE e.target_type = 'decision' \
                      AND e.target_id = d.id \
                      AND LOWER(COALESCE(e.model, '')) = ?1\
                ) \
             ORDER BY d.id ASC \
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![model_key, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    (mem, dec)
}

pub(crate) fn count_unembedded_targets_for_model(
    conn: &rusqlite::Connection,
    model_key: &str,
) -> (usize, usize) {
    let memory_count = conn
        .query_row(
            "SELECT COUNT(*) FROM memories m \
             WHERE m.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'memory' \
                     AND e.target_id = m.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            rusqlite::params![model_key],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;

    let decision_count = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions d \
             WHERE d.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'decision' \
                     AND e.target_id = d.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            rusqlite::params![model_key],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;

    (memory_count, decision_count)
}

pub(crate) async fn build_embeddings_async(
    engine: std::sync::Arc<embeddings::EmbeddingEngine>,
    db: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    batch_size: usize,
    max_batches_per_pass: usize,
    lock_wait: Duration,
) -> EmbeddingBackfillPassResult {
    let model_key = engine.model_key();
    let mut result = EmbeddingBackfillPassResult::default();

    for _ in 0..max_batches_per_pass {
        let (unembedded_mem, unembedded_dec) = {
            let Some(conn) =
                acquire_background_db_lock(db, "embedding backfill scan", lock_wait).await
            else {
                break;
            };
            collect_unembedded_targets_for_model(&conn, model_key, batch_size)
        };

        let memory_count = unembedded_mem.len();
        let decision_count = unembedded_dec.len();
        let total = memory_count + decision_count;
        if total == 0 {
            result.exhausted = true;
            break;
        }
        result.passes_ran += 1;
        result.queued_total += total;

        let mut computed_batch = 0usize;
        let mut mem_results: Vec<(i64, Vec<u8>)> = Vec::new();
        for (id, text) in &unembedded_mem {
            if let Some(vec) = engine.clone().embed_async(text.clone()).await {
                mem_results.push((*id, embeddings::vector_to_blob(&vec)));
                computed_batch += 1;
            }
        }

        let mut dec_results: Vec<(i64, Vec<u8>)> = Vec::new();
        for (id, text) in &unembedded_dec {
            if let Some(vec) = engine.clone().embed_async(text.clone()).await {
                dec_results.push((*id, embeddings::vector_to_blob(&vec)));
                computed_batch += 1;
            }
        }

        {
            let Some(conn) =
                acquire_background_db_lock(db, "embedding backfill persist", lock_wait).await
            else {
                break;
            };
            for (id, blob) in &mem_results {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                     VALUES ('memory', ?1, ?2, ?3)",
                    rusqlite::params![id, blob, model_key],
                );
            }
            for (id, blob) in &dec_results {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                     VALUES ('decision', ?1, ?2, ?3)",
                    rusqlite::params![id, blob, model_key],
                );
            }
        }

        result.computed_total += computed_batch;
        if !backfill_batch_may_have_more(memory_count, decision_count, batch_size) {
            result.exhausted = true;
            break;
        }
    }

    if result.queued_total > 0 {
        eprintln!(
            "[embeddings] Built {}/{} embeddings this pass (passes={}, batch_size={}, max_batches={}, exhausted={})",
            result.computed_total,
            result.queued_total,
            result.passes_ran,
            batch_size,
            max_batches_per_pass,
            result.exhausted
        );
    }
    result
}
