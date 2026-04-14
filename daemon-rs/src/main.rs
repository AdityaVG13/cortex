// SPDX-License-Identifier: MIT
mod aging;
mod auth;
mod co_occurrence;
mod compaction;
mod compiler;
mod conflict;
mod crystallize;
mod daemon_lifecycle;
mod db;
mod embeddings;
mod export_data;
mod focus;
mod handlers;
mod hook_boot;
mod indexer;
mod logging;
mod mcp_proxy;
#[allow(dead_code)]
mod mcp_stdio;
mod prompt_inject;
mod rate_limit;
mod server;
mod service;
mod setup;
mod state;
mod tls;

use chrono::{self, Utc};
use std::path::Path;
use std::time::Duration;

const BACKUP_RETENTION_COUNT: usize = 3;
const BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION: i32 = 5;
const LOG_ROTATION_BYTES: u64 = 1024 * 1024;
const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
const SPAWN_PARENT_PID_ENV: &str = "CORTEX_SPAWN_PARENT_PID";
const ORPHAN_WATCH_INTERVAL_SECS: u64 = 2;
const STARTUP_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];

#[derive(Clone, Copy, Debug, Default)]
struct EnsureDaemonResult {
    spawned: bool,
}

// ── Backup rotation helpers ───────────────────────────────────────────────

/// Check if a backup should be created (>24h since last backup).
fn should_backup(backup_dir: &Path) -> bool {
    let last_backup_file = backup_dir.join(".last_backup");
    if !last_backup_file.exists() {
        return true;
    }
    match std::fs::read_to_string(&last_backup_file) {
        Ok(ts) => {
            if let Ok(last_backup) = chrono::DateTime::parse_from_rfc3339(&ts) {
                let now = Utc::now();
                // Convert FixedOffset to UTC for subtraction
                let last_utc = last_backup.with_timezone(&Utc);
                let hours_since_last = (now - last_utc).num_hours();
                hours_since_last >= 24
            } else {
                true
            }
        }
        Err(_) => true,
    }
}

/// Rotate backups to keep only the most recent N.
fn rotate_backups(backup_dir: &Path, keep: usize) -> Result<usize, std::io::Error> {
    let mut backups: Vec<_> = std::fs::read_dir(backup_dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry.file_name().to_string_lossy().starts_with("cortex-")
                        && entry.file_name().to_string_lossy().ends_with(".db")
                        && !entry.file_name().to_string_lossy().contains(".corrupt")
                })
                .collect()
        })
        .unwrap_or_default();

    if backups.len() <= keep {
        return Ok(0);
    }

    // Sort by modification time (oldest first)
    backups.sort_by_key(|entry| entry.metadata().ok().and_then(|m| m.modified().ok()));

    let mut removed = 0usize;
    for backup in backups.iter().take(backups.len() - keep) {
        std::fs::remove_file(backup.path())?;
        removed += 1;
    }

    Ok(removed)
}

fn cleanup_backup_retention(backup_dir: &Path) -> usize {
    match rotate_backups(backup_dir, BACKUP_RETENTION_COUNT) {
        Ok(removed) => removed,
        Err(e) => {
            eprintln!("[cortex] Warning: backup rotation failed: {e}");
            0
        }
    }
}

fn cleanup_bridge_backups(home: &Path, schema_version: i32) -> bool {
    if schema_version < BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION {
        return false;
    }

    let bridge_backup_dir = home.join("bridge-backups");
    if !bridge_backup_dir.exists() {
        return false;
    }

    match std::fs::remove_dir_all(&bridge_backup_dir) {
        Ok(()) => {
            eprintln!("[cortex] Removed legacy bridge-backups for schema version {schema_version}");
            true
        }
        Err(e) => {
            eprintln!("[cortex] Warning: failed to remove legacy bridge-backups: {e}");
            false
        }
    }
}

fn cleanup_expired_rows(conn: &rusqlite::Connection, label: &str) {
    match db::delete_expired_entries(conn) {
        Ok(counts) if counts.memories_deleted > 0 || counts.decisions_deleted > 0 => {
            eprintln!(
                "[cortex] {label}: deleted {} expired memories and {} expired decisions",
                counts.memories_deleted, counts.decisions_deleted
            );
        }
        Ok(_) => {}
        Err(e) => eprintln!("[cortex] Warning: expired-row cleanup failed: {e}"),
    }
}

fn rotate_log_file(home: &Path, file_name: &str) -> Result<bool, std::io::Error> {
    let log_path = home.join(file_name);
    let metadata = match std::fs::metadata(&log_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };

    if metadata.len() <= LOG_ROTATION_BYTES {
        return Ok(false);
    }

    let rotated_path = home.join(format!("{file_name}.1"));
    if rotated_path.exists() {
        std::fs::remove_file(&rotated_path)?;
    }
    std::fs::rename(&log_path, &rotated_path)?;
    std::fs::File::create(&log_path)?;
    Ok(true)
}

fn rotate_startup_logs(home: &Path) -> usize {
    let mut rotated = 0usize;
    for file_name in STARTUP_LOG_FILES {
        match rotate_log_file(home, file_name) {
            Ok(true) => {
                rotated += 1;
                eprintln!("[cortex] Rotated log file {file_name}");
            }
            Ok(false) => {}
            Err(e) => {
                eprintln!("[cortex] Warning: failed to rotate {file_name}: {e}");
            }
        }
    }
    rotated
}

fn collect_backup_cleanup_files(backup_dir: &Path, keep: usize) -> Vec<(std::path::PathBuf, u64)> {
    let mut backups: Vec<_> = std::fs::read_dir(backup_dir)
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry.file_name().to_string_lossy().starts_with("cortex-")
                        && entry.file_name().to_string_lossy().ends_with(".db")
                        && !entry.file_name().to_string_lossy().contains(".corrupt")
                })
                .collect()
        })
        .unwrap_or_default();

    if backups.len() <= keep {
        return Vec::new();
    }

    backups.sort_by_key(|entry| entry.metadata().ok().and_then(|m| m.modified().ok()));
    let remove_count = backups.len() - keep;
    backups
        .into_iter()
        .take(remove_count)
        .map(|entry| {
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            (entry.path(), size)
        })
        .collect()
}

fn format_cleanup_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes >= MB as u64 {
        format!("{:.1} MB", bytes as f64 / MB)
    } else if bytes >= KB as u64 {
        format!("{:.1} KB", bytes as f64 / KB)
    } else {
        format!("{bytes} B")
    }
}

fn path_size_bytes(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| path_size_bytes(&entry.path()))
                    .sum()
            })
            .unwrap_or(0),
        _ => 0,
    }
}

fn run_backup_cleanup(backup_dir: &Path, dry_run: bool) -> Vec<String> {
    let candidates = collect_backup_cleanup_files(backup_dir, BACKUP_RETENTION_COUNT);
    let mut lines = Vec::new();
    for (path, size) in candidates {
        let target = format!(
            "backups/{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        );
        lines.push(format!("DELETE {target} ({})", format_cleanup_bytes(size)));
        if !dry_run {
            let _ = std::fs::remove_file(path);
        }
    }
    lines
}

fn run_log_cleanup(home: &Path, dry_run: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for file_name in STARTUP_LOG_FILES {
        let log_path = home.join(file_name);
        let metadata = match std::fs::metadata(&log_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if metadata.len() <= LOG_ROTATION_BYTES {
            continue;
        }

        lines.push(format!(
            "ROTATE {file_name} ({})",
            format_cleanup_bytes(metadata.len())
        ));

        if dry_run {
            continue;
        }

        let rotated_path = home.join(format!("{file_name}.1"));
        if rotated_path.exists() {
            let _ = std::fs::remove_file(&rotated_path);
        }
        if std::fs::rename(&log_path, &rotated_path).is_ok() {
            let _ = std::fs::File::create(&log_path);
        }
    }
    lines
}

fn run_bridge_backup_cleanup(home: &Path, schema_version: i32, dry_run: bool) -> Vec<String> {
    if schema_version < BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION {
        return Vec::new();
    }

    let bridge_dir = home.join("bridge-backups");
    if !bridge_dir.exists() {
        return Vec::new();
    }

    let size = path_size_bytes(&bridge_dir);
    let line = format!("DELETE bridge-backups/ ({})", format_cleanup_bytes(size));
    if !dry_run {
        let _ = std::fs::remove_dir_all(&bridge_dir);
    }
    vec![line]
}

fn run_stale_pid_cleanup(paths: &auth::CortexPaths, dry_run: bool) -> Vec<String> {
    let Some(pid) = auth::stale_pid_candidate(paths) else {
        return Vec::new();
    };

    let lines = vec![format!("DELETE cortex.pid (process {pid} not running)")];

    if !dry_run {
        let _ = auth::cleanup_stale_pid_lock(paths);
    }

    lines
}

/// Create a backup of the database file.
fn create_backup(db_path: &Path, backup_dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("create backup dir: {e}"))?;

    let timestamp = chrono::Local::now().format("%Y%m%d");
    let dest = backup_dir.join(format!("cortex-{timestamp}.db"));

    // Copy the DB file (not move - preserves original)
    std::fs::copy(db_path, &dest).map_err(|e| format!("copy db: {e}"))?;

    eprintln!("[cortex] Backup created: {}", dest.display());

    // Rotate old backups after creating a fresh backup.
    let _ = cleanup_backup_retention(backup_dir);

    // Update last backup timestamp
    let last_backup_file = backup_dir.join(".last_backup");
    let now_ts = chrono::Utc::now().to_rfc3339();
    if let Err(e) = std::fs::write(&last_backup_file, now_ts) {
        eprintln!("[cortex] Warning: failed to write last_backup timestamp: {e}");
    }

    Ok(dest.to_string_lossy().to_string())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let paths = auth::CortexPaths::resolve_from_args(&args);

    match mode {
        // ── HTTP daemon (standalone or via service) ─────────────────
        "serve" => {
            #[cfg(unix)]
            async fn sigterm_future() {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm =
                    signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            async fn sigterm_future() {
                std::future::pending::<()>().await;
            }

            run_daemon(paths.clone(), async {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        eprintln!("[cortex] Received Ctrl+C, shutting down...");
                    }
                    _ = sigterm_future() => {
                        eprintln!("[cortex] Received SIGTERM, shutting down...");
                    }
                }
            })
            .await;
        }

        // ── MCP stdio transport ─────────────────────────────────────
        "mcp" => {
            let remaining = &args[2..];
            let agent = parse_flag_value(remaining, "--agent");
            let (base_url, api_key, local_owner_mode) = resolve_client_target(remaining, &paths);
            // MCP clients are attach-only; daemon lifecycle is owned by
            // Control Center / service commands.
            let allow_spawn = false;
            if let Err(e) = ensure_remote_target_has_api_key(&base_url, api_key.as_deref(), &paths)
            {
                eprintln!("[cortex-mcp] {e}");
                std::process::exit(1);
            }
            let ensure = if local_owner_mode {
                match ensure_daemon(&paths, agent.as_deref(), false, allow_spawn, None).await {
                    Ok(result) => result,
                    Err(e) => {
                        eprintln!("[cortex-mcp] {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                EnsureDaemonResult::default()
            };
            if local_owner_mode {
                apply_path_env(&paths);
            }
            if let Err(e) = mcp_proxy::run(
                &base_url,
                api_key.as_deref(),
                agent.as_deref(),
                mcp_proxy::ProxyRuntimeOptions {
                    allow_respawn: allow_spawn,
                    shutdown_on_exit: ensure.spawned,
                    shutdown_on_idle_startup: ensure.spawned,
                    respawn_owner: None,
                },
            )
            .await
            {
                eprintln!("[cortex-mcp] {e}");
                std::process::exit(1);
            }
            if ensure.spawned {
                eprintln!("[cortex-mcp] Started a local daemon for this session");
            }
        }

        "paths" => {
            if args.iter().any(|a| a == "--json") {
                println!("{}", paths.to_json());
            } else {
                eprintln!("Usage: cortex paths --json");
                std::process::exit(1);
            }
        }

        "boot" => {
            let remaining: Vec<String> = args[2..].to_vec();
            if let Err(e) = run_boot_cli(&paths, &remaining).await {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }

        "plugin" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "ensure-daemon" => {
                    let agent = parse_flag_value(&args[3..], "--agent");
                    if let Err(e) = ensure_daemon(&paths, agent.as_deref(), true, false, None).await
                    {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
                "mcp" => {
                    let remaining = &args[3..];
                    let (base_url, api_key, local_owner_mode) =
                        resolve_client_target(remaining, &paths);
                    let agent = parse_flag_value(remaining, "--agent");
                    let allow_spawn = false;
                    if let Err(e) =
                        ensure_remote_target_has_api_key(&base_url, api_key.as_deref(), &paths)
                    {
                        eprintln!("[cortex-plugin] {e}");
                        std::process::exit(1);
                    }
                    let ensure = if local_owner_mode {
                        match ensure_daemon(&paths, agent.as_deref(), false, allow_spawn, None)
                            .await
                        {
                            Ok(result) => result,
                            Err(e) => {
                                eprintln!("[cortex-plugin] {e}");
                                std::process::exit(1);
                            }
                        }
                    } else {
                        EnsureDaemonResult::default()
                    };
                    if local_owner_mode {
                        apply_path_env(&paths);
                    }
                    if let Err(e) = mcp_proxy::run(
                        &base_url,
                        api_key.as_deref(),
                        agent.as_deref(),
                        mcp_proxy::ProxyRuntimeOptions {
                            allow_respawn: allow_spawn,
                            shutdown_on_exit: ensure.spawned,
                            shutdown_on_idle_startup: false,
                            respawn_owner: None,
                        },
                    )
                    .await
                    {
                        eprintln!("[cortex-plugin] {e}");
                        std::process::exit(1);
                    }
                }
                _ => {
                    eprintln!("Usage: cortex plugin <ensure-daemon|mcp>");
                    std::process::exit(1);
                }
            }
        }

        // ── Hook: SessionStart (replaces brain-boot.js) ─────────────
        "hook-boot" => {
            let agent = args
                .get(2)
                .and_then(|a| {
                    if a == "--agent" {
                        args.get(3).map(|s| s.as_str())
                    } else {
                        Some(a.as_str())
                    }
                })
                .unwrap_or("claude-opus");
            hook_boot::run_boot(agent).await;
        }

        // ── Hook: Statusline one-liner ──────────────────────────────
        "hook-status" => {
            hook_boot::run_status().await;
        }

        // ── Windows Service lifecycle ───────────────────────────────
        "service" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "install" => service::install(),
                "uninstall" => service::uninstall(),
                "start" => service::start(),
                "stop" => service::stop(),
                "status" => service::status(),
                "ensure" => service::ensure(),
                _ => {
                    eprintln!("Usage: cortex service <install|uninstall|start|stop|status|ensure>");
                }
            }
        }

        // ── Windows Service entry point (called by SCM) ─────────────
        "service-run" => {
            service::dispatch_service();
        }

        // ── System prompt injector CLI ──────────────────────────────
        "prompt-inject" => {
            let remaining: Vec<String> = args[2..].to_vec();
            prompt_inject::run(&remaining).await;
        }

        // ── Setup: detect AI tools, configure, verify ──────────────
        "setup" => {
            let remaining: Vec<String> = args[2..].to_vec();
            if remaining.iter().any(|a| a == "--team") {
                let dry_run = remaining.iter().any(|a| a == "--dry-run");
                setup::run_setup_team(&remaining, dry_run).await;
            } else {
                setup::run_setup().await;
            }
        }

        // ── Migrate: alias for setup --team with dry-run support ───
        "migrate" => {
            let remaining: Vec<String> = args[2..].to_vec();
            let dry_run = remaining.iter().any(|a| a == "--dry-run");
            setup::run_setup_team(&remaining, dry_run).await;
        }

        // ── Data export/import CLI ──────────────────────────────────
        "export" => {
            let remaining: Vec<String> = args[2..].to_vec();
            run_export_cli(&remaining);
        }
        "import" => {
            let remaining: Vec<String> = args[2..].to_vec();
            run_import_cli(&remaining);
        }
        "doctor" => {
            run_doctor_cli(&paths);
        }
        "cleanup" => {
            let dry_run = args.iter().any(|a| a == "--dry-run");
            run_cleanup_cli(&paths, dry_run);
        }

        // ── Backup/restore CLI ────────────────────────────────────
        "backup" => {
            let db_path = paths.db.clone();
            let home_dir = paths.home.clone();
            // Force checkpoint before backup for consistency
            let conn = match db::open(&db_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("Error: failed to open database: {e}");
                    std::process::exit(1);
                }
            };
            db::checkpoint_wal_best_effort(&conn);
            drop(conn);

            let backup_dir = home_dir.join("backups");
            match create_backup(&db_path, &backup_dir) {
                Ok(path) => {
                    println!("Backup created: {path}");
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "restore" => {
            let restore_file = match args.get(2) {
                Some(f) => f.clone(),
                None => {
                    eprintln!("Usage: cortex restore <backup-file.db>");
                    eprintln!("       cortex restore <backup-file.db> --skip-verification");
                    eprintln!();
                    eprintln!("Example: cortex restore ~/.cortex/backups/cortex-20260407.db");
                    std::process::exit(1);
                }
            };

            let skip_verification = args.iter().any(|a| a == "--skip-verification");

            // Check if daemon is running by checking PID file
            let paths_check = auth::CortexPaths::resolve();
            let daemon_running = paths_check.pid.exists();

            if daemon_running {
                eprintln!(
                    "[cortex] Warning: Daemon PID file exists at {}",
                    paths_check.pid.display()
                );
                eprintln!(
                    "[cortex] Please stop the daemon first with: Ctrl+C or kill the daemon process"
                );
                eprintln!("[cortex] Continuing restore anyway...");
                std::thread::sleep(Duration::from_millis(500));
            }

            let db_path = paths.db.clone();
            let home_dir = paths.home.clone();

            // Create a pre-restore backup
            let timestamp = chrono::Local::now().format("%Y%m%dT%H%M%S");
            let pre_backup = home_dir.join(format!("cortex.pre-restore.{}.db", timestamp));

            eprintln!(
                "[cortex] Creating pre-restore backup at: {}",
                pre_backup.display()
            );
            if let Err(e) = std::fs::copy(&db_path, &pre_backup) {
                eprintln!("[cortex] Error: failed to create pre-restore backup: {e}");
                eprintln!("[cortex] Restore cancelled for safety");
                std::process::exit(1);
            }

            // Restore from backup file
            eprintln!("[cortex] Restoring from: {}", restore_file);
            if let Err(e) = std::fs::copy(&restore_file, &db_path) {
                eprintln!("[cortex] Error: failed to restore backup: {e}");
                eprintln!(
                    "[cortex] Pre-restore backup preserved at: {}",
                    pre_backup.display()
                );
                std::process::exit(1);
            }

            // Verify integrity of restored DB
            if !skip_verification {
                eprintln!("[cortex] Verifying integrity of restored database...");
                match db::open(&db_path) {
                    Ok(conn) => {
                        if !db::verify_integrity(&conn).unwrap_or(false) {
                            eprintln!("[cortex] Error: restored database failed integrity check!");
                            eprintln!("[cortex] Rolling back to pre-restore backup...");
                            if let Err(e) = std::fs::copy(&pre_backup, &db_path) {
                                eprintln!(
                                    "[cortex] Critical: rollback failed! DB may be corrupted: {e}"
                                );
                            } else {
                                eprintln!("[cortex] Rollback complete");
                            }
                            std::process::exit(1);
                        }
                        eprintln!("[cortex] Integrity check passed");
                    }
                    Err(e) => {
                        eprintln!("[cortex] Error: failed to open restored database: {e}");
                        eprintln!("[cortex] Rolling back to pre-restore backup...");
                        if let Err(e) = std::fs::copy(&pre_backup, &db_path) {
                            eprintln!(
                                "[cortex] Critical: rollback failed! DB may be corrupted: {e}"
                            );
                        } else {
                            eprintln!("[cortex] Rollback complete");
                        }
                        std::process::exit(1);
                    }
                }
            }

            eprintln!(
                "[cortex] Restore complete. Pre-restore backup preserved at: {}",
                pre_backup.display()
            );
            eprintln!("[cortex] You can now restart the daemon with: cortex serve");
        }

        // ── User management CLI ────────────────────────────────────
        "user" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "add" => {
                    let username = match args.get(3) {
                        Some(u) => u.clone(),
                        None => {
                            eprintln!(
                                "Usage: cortex user add <username> [--role member|admin] [--display-name \"...\"]"
                            );
                            std::process::exit(1);
                        }
                    };
                    let mut role = "member".to_string();
                    let mut display_name: Option<String> = None;
                    let mut i = 4usize;
                    while i < args.len() {
                        match args[i].as_str() {
                            "--role" => {
                                if let Some(v) = args.get(i + 1) {
                                    role = v.clone();
                                    i += 1;
                                }
                            }
                            "--display-name" => {
                                if let Some(v) = args.get(i + 1) {
                                    display_name = Some(v.clone());
                                    i += 1;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let mut body = serde_json::json!({
                        "username": username,
                        "role": role,
                    });
                    if let Some(dn) = display_name {
                        body["display_name"] = serde_json::json!(dn);
                    }
                    match admin_request("POST", "/admin/user/add", Some(body)).await {
                        Ok(json) => {
                            println!("User created:");
                            println!("  Username:  {}", json_str(&json, "username"));
                            println!("  User ID:   {}", json_field(&json, "user_id"));
                            println!("  Role:      {}", json_str(&json, "role"));
                            println!("  API Key:   {}", json_str(&json, "api_key"));
                            println!();
                            println!("Save the API key -- it cannot be retrieved later.");
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "rotate-key" => {
                    let username = match args.get(3) {
                        Some(u) => u.clone(),
                        None => {
                            eprintln!("Usage: cortex user rotate-key <username>");
                            std::process::exit(1);
                        }
                    };
                    let body = serde_json::json!({ "username": username });
                    match admin_request("POST", "/admin/user/rotate-key", Some(body)).await {
                        Ok(json) => {
                            println!("API key rotated for '{}':", json_str(&json, "username"));
                            println!("  New API Key: {}", json_str(&json, "api_key"));
                            println!();
                            println!("Save the API key -- it cannot be retrieved later.");
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "remove" => {
                    let username = match args.get(3) {
                        Some(u) => u.clone(),
                        None => {
                            eprintln!("Usage: cortex user remove <username>");
                            std::process::exit(1);
                        }
                    };
                    if !confirm_action(&format!("Remove user '{username}'?")) {
                        eprintln!("Cancelled.");
                        std::process::exit(0);
                    }
                    let body = serde_json::json!({ "username": username });
                    match admin_request("POST", "/admin/user/remove", Some(body)).await {
                        Ok(json) => {
                            println!("Removed user '{}'", json_str(&json, "removed"));
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "list" => match admin_request("GET", "/admin/users", None).await {
                    Ok(json) => {
                        let users = json["users"].as_array();
                        match users {
                            Some(arr) if !arr.is_empty() => {
                                println!(
                                    "{:<6} {:<20} {:<20} {:<10} CREATED",
                                    "ID", "USERNAME", "DISPLAY NAME", "ROLE"
                                );
                                println!("{}", "-".repeat(80));
                                for u in arr {
                                    println!(
                                        "{:<6} {:<20} {:<20} {:<10} {}",
                                        json_field(u, "id"),
                                        json_str(u, "username"),
                                        json_str_or(u, "display_name", "-"),
                                        json_str(u, "role"),
                                        json_str_or(u, "created_at", "-"),
                                    );
                                }
                                println!();
                                println!("{} user(s)", arr.len());
                            }
                            _ => println!("No users found."),
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                _ => {
                    eprintln!("Usage: cortex user <add|rotate-key|remove|list>");
                    std::process::exit(1);
                }
            }
        }

        // ── Team management CLI ────────────────────────────────────
        "team" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "create" => {
                    let name = match args.get(3) {
                        Some(n) => n.clone(),
                        None => {
                            eprintln!("Usage: cortex team create <name>");
                            std::process::exit(1);
                        }
                    };
                    let body = serde_json::json!({ "name": name });
                    match admin_request("POST", "/admin/team/create", Some(body)).await {
                        Ok(json) => {
                            println!("Team created:");
                            println!("  Name:    {}", json_str(&json, "name"));
                            println!("  Team ID: {}", json_field(&json, "team_id"));
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "add" => {
                    let team_name = match args.get(3) {
                        Some(t) => t.clone(),
                        None => {
                            eprintln!(
                                "Usage: cortex team add <team> <username> [--role member|admin]"
                            );
                            std::process::exit(1);
                        }
                    };
                    let username = match args.get(4) {
                        Some(u) => u.clone(),
                        None => {
                            eprintln!(
                                "Usage: cortex team add <team> <username> [--role member|admin]"
                            );
                            std::process::exit(1);
                        }
                    };
                    let mut role = "member".to_string();
                    let mut i = 5usize;
                    while i < args.len() {
                        if args[i] == "--role" {
                            if let Some(v) = args.get(i + 1) {
                                role = v.clone();
                                i += 1;
                            }
                        }
                        i += 1;
                    }
                    let body = serde_json::json!({
                        "team_name": team_name,
                        "username": username,
                        "role": role,
                    });
                    match admin_request("POST", "/admin/team/add-member", Some(body)).await {
                        Ok(json) => {
                            println!(
                                "Added '{}' to team '{}' as {}",
                                json_str(&json, "username"),
                                json_str(&json, "team"),
                                json_str(&json, "role"),
                            );
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "remove" => {
                    let team_name = match args.get(3) {
                        Some(t) => t.clone(),
                        None => {
                            eprintln!("Usage: cortex team remove <team> <username>");
                            std::process::exit(1);
                        }
                    };
                    let username = match args.get(4) {
                        Some(u) => u.clone(),
                        None => {
                            eprintln!("Usage: cortex team remove <team> <username>");
                            std::process::exit(1);
                        }
                    };
                    if !confirm_action(&format!("Remove '{username}' from team '{team_name}'?")) {
                        eprintln!("Cancelled.");
                        std::process::exit(0);
                    }
                    let body = serde_json::json!({
                        "team_name": team_name,
                        "username": username,
                    });
                    match admin_request("POST", "/admin/team/remove-member", Some(body)).await {
                        Ok(json) => {
                            let removed = &json["removed"];
                            println!(
                                "Removed '{}' from team '{}'",
                                json_str(removed, "username"),
                                json_str(removed, "team"),
                            );
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "list" => match admin_request("GET", "/admin/teams", None).await {
                    Ok(json) => {
                        let teams = json["teams"].as_array();
                        match teams {
                            Some(arr) if !arr.is_empty() => {
                                println!("{:<6} {:<30} {:<10} CREATED", "ID", "NAME", "MEMBERS");
                                println!("{}", "-".repeat(70));
                                for t in arr {
                                    println!(
                                        "{:<6} {:<30} {:<10} {}",
                                        json_field(t, "id"),
                                        json_str(t, "name"),
                                        json_field(t, "member_count"),
                                        json_str_or(t, "created_at", "-"),
                                    );
                                }
                                println!();
                                println!("{} team(s)", arr.len());
                            }
                            _ => println!("No teams found."),
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                _ => {
                    eprintln!("Usage: cortex team <create|add|remove|list>");
                    std::process::exit(1);
                }
            }
        }

        // ── Admin management CLI ───────────────────────────────────
        "admin" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "list-unowned" => match admin_request("GET", "/admin/unowned", None).await {
                    Ok(json) => {
                        let unowned = json["unowned"].as_object();
                        match unowned {
                            Some(map) if !map.is_empty() => {
                                println!("{:<25} UNOWNED ROWS", "TABLE");
                                println!("{}", "-".repeat(40));
                                let mut total: i64 = 0;
                                for (table, count) in map {
                                    let n = count.as_i64().unwrap_or(0);
                                    total += n;
                                    println!("{:<25} {}", table, n);
                                }
                                println!("{}", "-".repeat(40));
                                println!("{:<25} {}", "TOTAL", total);
                            }
                            _ => println!("No unowned data found."),
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                "assign-owner" => {
                    let mut from_user: Option<String> = None;
                    let mut to_user: Option<String> = None;
                    let mut table: Option<String> = None;
                    let mut i = 3usize;
                    while i < args.len() {
                        match args[i].as_str() {
                            "--from" => {
                                if let Some(v) = args.get(i + 1) {
                                    from_user = Some(v.clone());
                                    i += 1;
                                }
                            }
                            "--to" => {
                                if let Some(v) = args.get(i + 1) {
                                    to_user = Some(v.clone());
                                    i += 1;
                                }
                            }
                            "--table" => {
                                if let Some(v) = args.get(i + 1) {
                                    table = Some(v.clone());
                                    i += 1;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let Some(to) = to_user else {
                        eprintln!(
                            "Usage: cortex admin assign-owner [--from <user>] --to <user> [--table <table>]"
                        );
                        std::process::exit(1);
                    };
                    let mut body = serde_json::json!({ "to_user": to });
                    if let Some(from) = from_user {
                        body["from_user"] = serde_json::json!(from);
                    }
                    if let Some(t) = table {
                        body["table"] = serde_json::json!(t);
                    }
                    match admin_request("POST", "/admin/assign-owner", Some(body)).await {
                        Ok(json) => {
                            let assigned = json["assigned"].as_object();
                            match assigned {
                                Some(map) if !map.is_empty() => {
                                    println!("{:<25} ROWS ASSIGNED", "TABLE");
                                    println!("{}", "-".repeat(40));
                                    let mut total: i64 = 0;
                                    for (tbl, count) in map {
                                        let n = count.as_i64().unwrap_or(0);
                                        total += n;
                                        println!("{:<25} {}", tbl, n);
                                    }
                                    println!("{}", "-".repeat(40));
                                    println!("{:<25} {}", "TOTAL", total);
                                }
                                _ => println!("No rows assigned."),
                            }
                        }
                        Err(e) => {
                            eprintln!("Error: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                "stats" => match admin_request("GET", "/admin/stats", None).await {
                    Ok(json) => {
                        println!("Cortex Admin Stats");
                        println!("{}", "=".repeat(50));
                        println!();
                        println!(
                            "Users: {}    Teams: {}    DB Size: {}",
                            json_field(&json, "user_count"),
                            json_field(&json, "team_count"),
                            json_str_or(&json, "db_size_mb", "?"),
                        );
                        println!();

                        if let Some(tables) = json["tables"].as_object() {
                            println!("{:<25} ROWS", "TABLE");
                            println!("{}", "-".repeat(40));
                            for (tbl, count) in tables {
                                println!("{:<25} {}", tbl, count);
                            }
                        }

                        if let Some(per_user) = json["per_user"].as_array() {
                            if !per_user.is_empty() {
                                println!();
                                println!("Per-User Breakdown:");
                                println!(
                                    "  {:<20} {:<10} {:<10} CRYSTALS",
                                    "USERNAME", "MEMORIES", "DECISIONS"
                                );
                                println!("  {}", "-".repeat(55));
                                for u in per_user {
                                    println!(
                                        "  {:<20} {:<10} {:<10} {}",
                                        json_str(u, "username"),
                                        json_field(u, "memories"),
                                        json_field(u, "decisions"),
                                        json_field(u, "crystals"),
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                },
                _ => {
                    eprintln!("Usage: cortex admin <list-unowned|assign-owner|stats>");
                    std::process::exit(1);
                }
            }
        }
        "--help" | "-h" | "help" => {
            print_usage_and_exit(0);
        }

        _ => {
            print_usage_and_exit(1);
        }
    }
}

fn print_usage_and_exit(code: i32) -> ! {
    eprintln!(
        "Cortex v{} -- Universal AI Memory Daemon",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!();
    eprintln!("Usage: cortex <command>");
    eprintln!();
    eprintln!("Setup:");
    eprintln!("  setup              First-run setup: detect AI tools, configure, verify");
    eprintln!("  setup --team       Team-mode setup + schema migration + owner API key");
    eprintln!("  migrate            Alias for setup --team (solo -> team migration)");
    eprintln!("  migrate --dry-run  Preview migration without modifying the database");
    eprintln!();
    eprintln!("Daemon:");
    eprintln!("  serve [--bind <addr>]  HTTP daemon on :7437 (default bind 127.0.0.1)");
    eprintln!(
        "  mcp [--url <base>] [--api-key <key>] [--agent <name>]  MCP stdio (attach-only when local)"
    );
    eprintln!("  paths --json       Print resolved Cortex paths + port + bind as JSON");
    eprintln!("  boot [--agent <name>] [--budget <n>] [--json] [--url <base>] [--api-key <key>]");
    eprintln!("  plugin ensure-daemon [--agent <name>]  Verify/attach to a healthy daemon only");
    eprintln!("  plugin mcp [--url <base>] [--api-key <key>] [--agent <name>]");
    eprintln!();
    eprintln!("Hooks:");
    eprintln!("  hook-boot [AGENT]  SessionStart hook (default: claude-opus)");
    eprintln!("  hook-status        Statusline one-liner");
    eprintln!();
    eprintln!("Tools:");
    eprintln!("  prompt-inject      Inject Cortex context into system prompt files");
    eprintln!("  export             Export data (--format json|sql, --out <file>)");
    eprintln!("  import             Import JSON data (--file <path>, optional --user <username>)");
    eprintln!("  doctor             Validate DB schema, migrations, integrity, and FTS health");
    eprintln!(
        "  cleanup [--dry-run]  Cleanup backups, logs, legacy bridge data, and stale PID files"
    );
    eprintln!("  backup             Create manual backup (stores in ~/.cortex/backups/)");
    eprintln!("  restore <file>     Restore from backup file (daemon must be stopped)");
    eprintln!();
    eprintln!("User Management (team mode):");
    eprintln!("  user add <name>    Add user [--role member|admin] [--display-name \"...\"]");
    eprintln!("  user rotate-key <name>  Rotate a user's API key");
    eprintln!("  user remove <name> Remove user (with confirmation)");
    eprintln!("  user list          List all users");
    eprintln!();
    eprintln!("Team Management (team mode):");
    eprintln!("  team create <name> Create a team");
    eprintln!("  team add <team> <user>  Add member [--role member|admin]");
    eprintln!("  team remove <team> <user>  Remove member (with confirmation)");
    eprintln!("  team list          List all teams");
    eprintln!();
    eprintln!("Admin (team mode):");
    eprintln!("  admin list-unowned List rows without an owner");
    eprintln!("  admin assign-owner [--from <user>] --to <user> [--table <t>]");
    eprintln!("  admin stats        Database and per-user statistics");
    eprintln!();
    eprintln!("Service:");
    eprintln!("  service install    Register as Windows Service (manual start by default)");
    eprintln!("  service uninstall  Remove Windows Service");
    eprintln!("  service start      Start the service");
    eprintln!("  service stop       Stop the service");
    eprintln!("  service status     Check service status");
    eprintln!("  service ensure     Ensure service is installed, running, and healthy");
    eprintln!();
    eprintln!("Troubleshooting:");
    eprintln!("  cortex doctor      Validate DB schema, migrations, integrity, and FTS state");
    eprintln!("  cortex boot        Preferred local boot path (auto-adds auth + SSRF headers)");
    eprintln!("  HTTP 403           Add header: X-Cortex-Request: true");
    eprintln!("  HTTP 401           Use Authorization: Bearer <token> from ~/.cortex/cortex.token");
    eprintln!(
        "  MCP not visible    Restart the client after `codex mcp add ...`; new MCP servers do not hot-attach mid-session"
    );
    eprintln!(
        "  App-hosted daemon  Restart the daemon from Cortex Control Center instead of stopping/starting it manually"
    );
    eprintln!("  More help          See Info/connecting.md for full connection and auth examples");
    std::process::exit(code);
}

fn run_cleanup_cli(paths: &auth::CortexPaths, dry_run: bool) {
    let schema_version = if paths.db.exists() {
        db::open(&paths.db)
            .and_then(|conn| db::current_schema_user_version(&conn))
            .unwrap_or_default()
    } else {
        0
    };

    let mut lines = Vec::new();
    lines.extend(run_backup_cleanup(&paths.home.join("backups"), dry_run));
    lines.extend(run_log_cleanup(&paths.home, dry_run));
    lines.extend(run_bridge_backup_cleanup(
        &paths.home,
        schema_version,
        dry_run,
    ));
    lines.extend(run_stale_pid_cleanup(paths, dry_run));

    if lines.is_empty() {
        println!("No cleanup actions needed");
        return;
    }

    for line in lines {
        println!("{line}");
    }
}

fn run_doctor_cli(paths: &auth::CortexPaths) {
    let db_path = paths.db.clone();
    println!("[doctor] db_path={}", db_path.display());

    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[doctor] FAIL open: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("[doctor] FAIL configure: {e}");
        std::process::exit(1);
    }

    let expected_tables = [
        "memories",
        "decisions",
        "embeddings",
        "events",
        "co_occurrence",
        "locks",
        "activities",
        "messages",
        "sessions",
        "tasks",
        "feed",
        "feed_acks",
        "context_cache",
        "recall_feedback",
        "schema_migrations",
        "focus_sessions",
        "memory_clusters",
        "cluster_members",
        "memories_fts",
        "decisions_fts",
    ];

    let missing_tables: Vec<&str> = expected_tables
        .iter()
        .copied()
        .filter(|table| !db::table_exists(&conn, table))
        .collect();
    if missing_tables.is_empty() {
        println!(
            "[doctor] OK tables: {}/{}",
            expected_tables.len(),
            expected_tables.len()
        );
    } else {
        println!(
            "[doctor] FAIL tables missing: {}",
            missing_tables.join(", ")
        );
    }

    let (schema_current, pending_versions) = match db::pending_migration_versions(&conn) {
        Ok(pending) => (pending.is_empty(), pending),
        Err(e) => {
            println!("[doctor] FAIL schema status: {e}");
            (false, vec![])
        }
    };
    if schema_current {
        let applied = db::applied_migration_versions(&conn)
            .map(|v| v.len())
            .unwrap_or(0);
        println!(
            "[doctor] OK schema current: {applied}/{} migrations applied",
            db::migration_definitions().len()
        );
    } else if !pending_versions.is_empty() {
        println!(
            "[doctor] FAIL schema pending: {}",
            pending_versions.join(", ")
        );
    }

    let integrity_ok = match db::verify_integrity(&conn) {
        Ok(true) => {
            println!("[doctor] OK integrity_check");
            true
        }
        Ok(false) => {
            println!("[doctor] FAIL integrity_check");
            false
        }
        Err(e) => {
            println!("[doctor] FAIL integrity_check error: {e}");
            false
        }
    };

    let fts_trigger_names = [
        "memories_fts_ai",
        "memories_fts_ad",
        "memories_fts_au",
        "decisions_fts_ai",
        "decisions_fts_ad",
        "decisions_fts_au",
    ];
    let fts_tables_ok =
        db::table_exists(&conn, "memories_fts") && db::table_exists(&conn, "decisions_fts");
    let fts_queries_ok = conn
        .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_ok()
        && conn
            .query_row("SELECT COUNT(*) FROM decisions_fts", [], |row| {
                row.get::<_, i64>(0)
            })
            .is_ok();
    let fts_triggers_ok = fts_trigger_names.iter().all(|name| {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1 LIMIT 1",
            rusqlite::params![name],
            |_| Ok(()),
        )
        .is_ok()
    });
    let fts_ok = fts_tables_ok && fts_queries_ok && fts_triggers_ok;
    if fts_ok {
        println!("[doctor] OK fts indexes");
    } else {
        println!("[doctor] FAIL fts indexes");
    }

    let all_ok = missing_tables.is_empty() && schema_current && integrity_ok && fts_ok;
    if all_ok {
        println!("[doctor] GREEN");
        return;
    }

    println!("[doctor] RED");
    std::process::exit(1);
}

fn run_export_cli(args: &[String]) {
    let mut format = "json".to_string();
    let mut out_path: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if let Some(v) = args.get(i + 1) {
                    format = v.to_string();
                    i += 1;
                }
            }
            "--out" => {
                if let Some(v) = args.get(i + 1) {
                    out_path = Some(v.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(export_format) = export_data::ExportFormat::parse(&format) else {
        eprintln!("Usage: cortex export --format json|sql [--out <path>]");
        std::process::exit(1);
    };

    let db_path = auth::db_path();
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to open database at {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("Failed to configure database: {e}");
        std::process::exit(1);
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("Failed to initialize schema: {e}");
        std::process::exit(1);
    }
    crystallize::migrate_crystal_tables(&conn);

    let output = match export_format {
        export_data::ExportFormat::Json => {
            let value = export_data::export_json_value(&conn);
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
        }
        export_data::ExportFormat::Sql => export_data::export_sql_text(&conn),
    };

    if let Some(path) = out_path {
        if let Err(e) = std::fs::write(&path, output) {
            eprintln!("Failed to write export file {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("Exported to {path}");
    } else {
        println!("{output}");
    }
}

fn run_import_cli(args: &[String]) {
    let mut file_path: Option<String> = None;
    let mut username: Option<String> = None;
    let mut visibility = "private".to_string();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                if let Some(v) = args.get(i + 1) {
                    file_path = Some(v.to_string());
                    i += 1;
                }
            }
            "--user" => {
                if let Some(v) = args.get(i + 1) {
                    username = Some(v.to_string());
                    i += 1;
                }
            }
            "--visibility" => {
                if let Some(v) = args.get(i + 1) {
                    visibility = v.to_string();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(file_path) = file_path else {
        eprintln!(
            "Usage: cortex import --file <path> [--user <username>] [--visibility private|team|shared]"
        );
        std::process::exit(1);
    };
    if !matches!(visibility.as_str(), "private" | "team" | "shared") {
        eprintln!("Invalid --visibility value '{visibility}'. Use private|team|shared.");
        std::process::exit(1);
    }

    let raw = match std::fs::read_to_string(&file_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Cannot read import file {file_path}: {e}");
            std::process::exit(1);
        }
    };
    let payload: export_data::ImportPayload = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Import file is not valid JSON: {e}");
            std::process::exit(1);
        }
    };

    let db_path = auth::db_path();
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to open database at {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("Failed to configure database: {e}");
        std::process::exit(1);
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("Failed to initialize schema: {e}");
        std::process::exit(1);
    }
    crystallize::migrate_crystal_tables(&conn);

    let team_mode = db::current_mode(&conn) == "team";
    if username.is_some() && !team_mode {
        eprintln!("--user import requires team mode. Run: cortex setup --team");
        std::process::exit(1);
    }
    let owner_id = if team_mode {
        if let Some(user) = username {
            match conn.query_row(
                "SELECT id FROM users WHERE username = ?1",
                rusqlite::params![user.clone()],
                |row| row.get::<_, i64>(0),
            ) {
                Ok(id) => Some(id),
                Err(_) => {
                    eprintln!("Unknown user '{user}'. Create the user before import.");
                    std::process::exit(1);
                }
            }
        } else {
            conn.query_row(
                "SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .or_else(|| {
                conn.query_row(
                    "SELECT id FROM users ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, id ASC LIMIT 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .ok()
            })
        }
    } else {
        None
    };
    if team_mode && owner_id.is_none() {
        eprintln!("Team mode import requires a target owner. Run `cortex setup --team` first.");
        std::process::exit(1);
    }

    let options = export_data::ImportOptions {
        owner_id,
        visibility: if team_mode { Some(visibility) } else { None },
        source_agent_fallback: "import-cli".to_string(),
    };
    let counts = export_data::import_payload(&conn, &payload, &options);
    println!(
        "{{\"imported\":{{\"memories\":{},\"decisions\":{}}}}}",
        counts.memories, counts.decisions
    );
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|idx| args.get(idx + 1))
        .cloned()
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
fn parse_truthy_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn normalize_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn local_daemon_base_url(paths: &auth::CortexPaths) -> String {
    let bind = paths.bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{}", paths.port)
}

fn normalized_host(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn is_local_client_base_url(base_url: &str, paths: &auth::CortexPaths) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if url.port_or_known_default() != Some(paths.port) {
        return false;
    }
    let host_norm = normalized_host(host);
    let bind_norm = normalized_host(&paths.bind);
    matches!(host_norm.as_str(), "127.0.0.1" | "localhost" | "::1")
        || (!bind_norm.is_empty()
            && !matches!(bind_norm.as_str(), "0.0.0.0" | "::")
            && host_norm == bind_norm)
}

fn resolve_client_target_inputs(
    override_url: Option<&str>,
    override_api_key: Option<&str>,
    env_base_url: Option<&str>,
    env_api_key: Option<&str>,
    default_base_url: &str,
) -> (String, Option<String>, bool) {
    let resolved_base_url =
        normalize_option(override_url).or_else(|| normalize_option(env_base_url));
    let resolved_api_key =
        normalize_option(override_api_key).or_else(|| normalize_option(env_api_key));
    let local_owner_mode = resolved_base_url.is_none() && resolved_api_key.is_none();
    let base_url = resolved_base_url.unwrap_or_else(|| default_base_url.to_string());
    (base_url, resolved_api_key, local_owner_mode)
}

fn resolve_client_target(
    args: &[String],
    paths: &auth::CortexPaths,
) -> (String, Option<String>, bool) {
    let override_url = parse_flag_value(args, "--url");
    let override_api_key = parse_flag_value(args, "--api-key");
    let env_base_url = env_trimmed("CORTEX_API_BASE").or_else(|| env_trimmed("CORTEX_BASE_URL"));
    let env_api_key = env_trimmed("CORTEX_API_KEY");
    resolve_client_target_inputs(
        override_url.as_deref(),
        override_api_key.as_deref(),
        env_base_url.as_deref(),
        env_api_key.as_deref(),
        &local_daemon_base_url(paths),
    )
}

fn ensure_remote_target_has_api_key(
    base_url: &str,
    api_key: Option<&str>,
    paths: &auth::CortexPaths,
) -> Result<(), String> {
    let parsed = reqwest::Url::parse(base_url).map_err(|_| {
        format!("Invalid Cortex target URL '{base_url}'. Use an absolute http:// or https:// URL.")
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported Cortex target URL scheme '{}' in '{base_url}'. Use http or https.",
            parsed.scheme()
        ));
    }
    if parsed.host_str().is_none() {
        return Err(format!(
            "Invalid Cortex target URL '{base_url}': missing host."
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "Cortex target URL must not include embedded credentials; pass --api-key instead."
                .to_string(),
        );
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(
            "Cortex target URL must not include query parameters or fragments.".to_string(),
        );
    }

    if api_key.is_none() && !is_local_client_base_url(base_url, paths) {
        return Err(format!(
            "Remote Cortex target '{base_url}' requires an API key. Pass --api-key <key> or set CORTEX_API_KEY."
        ));
    }
    Ok(())
}

fn apply_path_env(paths: &auth::CortexPaths) {
    std::env::set_var("CORTEX_HOME", &paths.home);
    std::env::set_var("CORTEX_DB", &paths.db);
    std::env::set_var("CORTEX_PORT", paths.port.to_string());
    std::env::set_var("CORTEX_BIND", &paths.bind);
}

fn parse_flag_usize(args: &[String], flag: &str) -> Result<Option<usize>, String> {
    let Some(idx) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };

    let raw = args
        .get(idx + 1)
        .ok_or_else(|| format!("missing value for {flag}"))?;
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("invalid value for {flag}: '{raw}'"))?;
    if value == 0 {
        return Err(format!("{flag} must be >= 1"));
    }
    Ok(Some(value))
}

use daemon_lifecycle::{
    daemon_healthy, is_cortex_health_payload, spawn_daemon, validate_spawned_owner_claim,
    wait_for_health, DAEMON_OWNER_TOKEN_ENV, SPAWN_PARENT_START_TIME_ENV,
};
const DAEMON_STARTUP_WAIT_SECS: u64 = 90;
const DEFAULT_BOOT_BUDGET: usize = 600;
const DEFAULT_DAEMON_LOCK_WAIT_SECS: u64 = 15;
const DAEMON_LOCK_RETRY_INTERVAL_MS: u64 = 100;
const DAEMON_LOCK_HANDOFF_GRACE_SECS: u64 = 3;

async fn recover_unhealthy_locked_daemon(
    paths: &auth::CortexPaths,
    result: &mut EnsureDaemonResult,
    owner_tag: Option<&str>,
) -> Result<bool, String> {
    let _ = auth::cleanup_stale_pid_lock(paths);

    let guard = match auth::acquire_daemon_lock(paths) {
        Ok(guard) => guard,
        Err(_) => return Ok(false),
    };

    if daemon_healthy(paths).await {
        drop(guard);
        return Ok(true);
    }

    apply_path_env(paths);
    spawn_daemon(paths, owner_tag)?;
    drop(guard);

    if !wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
        return Err(format!(
            "daemon lock was recovered but daemon did not become healthy on port {} within {}s",
            paths.port, DAEMON_STARTUP_WAIT_SECS
        ));
    }

    result.spawned = true;
    Ok(true)
}

fn read_auth_token_from_path(token_path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(token_path).ok().and_then(|token| {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn resolve_boot_auth_header(
    token_path: &std::path::Path,
    api_key: Option<&str>,
    allow_local_token_fallback: bool,
) -> Option<String> {
    if let Some(api_key) = api_key {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            return Some(format!("Bearer {trimmed}"));
        }
    }
    if allow_local_token_fallback {
        return read_auth_token_from_path(token_path).map(|token| format!("Bearer {token}"));
    }
    None
}

async fn request_boot_payload(
    base_url: &str,
    token_path: &std::path::Path,
    api_key: Option<&str>,
    allow_local_token_fallback: bool,
    agent: &str,
    budget: usize,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("create boot client: {e}"))?;

    let mut req = client
        .get(format!("{}/boot", base_url.trim_end_matches('/')))
        .query(&[
            ("agent".to_string(), agent.to_string()),
            ("budget".to_string(), budget.to_string()),
        ])
        .header("x-cortex-request", "true")
        .header("x-source-agent", agent);

    if let Some(auth) = resolve_boot_auth_header(token_path, api_key, allow_local_token_fallback) {
        req = req.header("Authorization", auth);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("boot request failed: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("read boot response failed: {e}"))?;
    if !status.is_success() {
        let detail = body.trim();
        return if detail.is_empty() {
            Err(format!("boot returned {status}"))
        } else {
            Err(format!("boot returned {status}: {detail}"))
        };
    }

    serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| format!("parse boot response failed: {e}"))
}

async fn run_boot_cli(paths: &auth::CortexPaths, args: &[String]) -> Result<(), String> {
    let agent = parse_flag_value(args, "--agent").unwrap_or_else(|| "cli".to_string());
    let agent = agent.trim();
    if agent.is_empty() {
        return Err("agent cannot be empty".to_string());
    }

    let budget = parse_flag_usize(args, "--budget")?.unwrap_or(DEFAULT_BOOT_BUDGET);
    let json_output = args.iter().any(|arg| arg == "--json");
    let (base_url, api_key, local_owner_mode) = resolve_client_target(args, paths);
    ensure_remote_target_has_api_key(&base_url, api_key.as_deref(), paths)?;

    if local_owner_mode {
        // Boot CLI does not own daemon lifecycle and must not auto-spawn.
        let _ = ensure_daemon(paths, None, false, false, None).await?;
    }

    let local_target_identity_valid = if local_owner_mode {
        false
    } else if is_local_client_base_url(&base_url, paths) {
        daemon_healthy(paths).await
    } else {
        false
    };
    let allow_local_token_fallback = local_owner_mode || local_target_identity_valid;
    let payload = request_boot_payload(
        &base_url,
        &paths.token,
        api_key.as_deref(),
        allow_local_token_fallback,
        agent,
        budget,
    )
    .await?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|e| format!("serialize boot response failed: {e}"))?
        );
    } else {
        let boot_prompt = payload
            .get("bootPrompt")
            .and_then(|value| value.as_str())
            .ok_or_else(|| "boot response missing bootPrompt".to_string())?;
        println!("{boot_prompt}");
    }
    Ok(())
}

async fn boot_agent(paths: &auth::CortexPaths, agent: &str) -> Result<(), String> {
    let base_url = local_daemon_base_url(paths);
    request_boot_payload(&base_url, &paths.token, None, true, agent, 200)
        .await
        .map(|_| ())
}

/// Hold the singleton daemon lock before startup so duplicate `serve`
/// invocations cannot rotate the shared auth token and then die on bind.
fn daemon_lock_wait_timeout() -> Duration {
    let secs = std::env::var("CORTEX_DAEMON_LOCK_WAIT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_DAEMON_LOCK_WAIT_SECS);
    Duration::from_secs(secs.max(1))
}

fn acquire_runtime_lock(paths: &auth::CortexPaths) -> Result<std::fs::File, String> {
    let _ = auth::cleanup_stale_pid_lock(paths);
    if std::env::var("CORTEX_WAIT_FOR_DAEMON_LOCK")
        .ok()
        .is_some_and(|value| value == "1")
    {
        let deadline = std::time::Instant::now() + daemon_lock_wait_timeout();
        let last_err = loop {
            match auth::acquire_daemon_lock(paths) {
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
            if let Ok(lock) = auth::acquire_daemon_lock(paths) {
                return Ok(lock);
            }
            std::thread::sleep(Duration::from_millis(DAEMON_LOCK_RETRY_INTERVAL_MS));
        }
        return Err(last_err);
    }
    auth::acquire_daemon_lock(paths)
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

async fn startup_single_daemon_preflight(paths: &auth::CortexPaths) -> Result<(), String> {
    let bind_addr = paths.bind.trim();
    let bind_error = match std::net::TcpListener::bind((bind_addr, paths.port)) {
        Ok(listener) => {
            drop(listener);
            return Ok(());
        }
        Err(err) => err,
    };

    let health_url = format!("{}/health", local_daemon_base_url(paths));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .map_err(|err| format!("daemon startup preflight: build HTTP client: {err}"))?;
    let response = match client.get(&health_url).send().await {
        Ok(response) => response,
        Err(err) => {
            return Err(format!(
                "daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}) and health probe at {health_url} failed ({err})",
                paths.port
            ));
        }
    };
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .unwrap_or_else(|_| String::from("<unreadable>"));

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
        "daemon startup denied: cannot bind {bind_addr}:{} ({bind_error}); /health at {health_url} returned non-canonical payload (HTTP {status})",
        paths.port
    ))
}

async fn ensure_daemon(
    paths: &auth::CortexPaths,
    agent: Option<&str>,
    emit_port: bool,
    allow_spawn: bool,
    owner_tag: Option<&str>,
) -> Result<EnsureDaemonResult, String> {
    std::fs::create_dir_all(&paths.home).map_err(|e| format!("create home dir: {e}"))?;
    let _ = auth::migrate_legacy_db(paths)?;

    let lock = auth::acquire_daemon_lock(paths);
    let mut result = EnsureDaemonResult::default();

    match lock {
        Ok(guard) => {
            if daemon_healthy(paths).await {
                // already healthy
            } else if allow_spawn {
                apply_path_env(paths);
                spawn_daemon(paths, owner_tag)?;
                drop(guard);
                if !wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
                    return Err(format!(
                        "daemon did not become healthy on port {} within {}s",
                        paths.port, DAEMON_STARTUP_WAIT_SECS
                    ));
                }
                result.spawned = true;
            } else {
                return Err(format!(
                    "daemon is not healthy on port {} and this client cannot start it automatically. Start Cortex from Control Center or rerun `cortex mcp --agent <name>` after the daemon is available.",
                    paths.port
                ));
            }
        }
        Err(_) => {
            if !wait_for_health(paths, Duration::from_secs(DAEMON_STARTUP_WAIT_SECS)).await {
                if allow_spawn {
                    if recover_unhealthy_locked_daemon(paths, &mut result, owner_tag).await? {
                        if let Some(agent) = agent {
                            if let Err(e) = boot_agent(paths, agent).await {
                                eprintln!(
                                    "[cortex-plugin] Warning: boot call failed for agent '{agent}': {e}"
                                );
                            }
                        }
                        if emit_port {
                            println!("{}", paths.port);
                        }
                        return Ok(result);
                    }
                    return Err(format!(
                        "daemon lock is held and daemon is not healthy on port {}",
                        paths.port
                    ));
                }
                return Err(format!(
                    "daemon is not healthy on port {} and another process still holds the daemon lock. Start Cortex from Control Center or rerun `cortex mcp --agent <name>` after the daemon is available.",
                    paths.port
                ));
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
    Ok(result)
}

// ── Admin CLI helpers ───────────────────────────────────────────────────────

fn read_auth_token() -> Result<String, String> {
    let token_path = auth::CortexPaths::resolve().token;
    std::fs::read_to_string(&token_path)
        .map(|v| v.trim().to_string())
        .map_err(|_| {
            format!(
                "Cannot read auth token at {}. Is the daemon running?",
                token_path.display()
            )
        })
}

async fn admin_request(
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let port = auth::CortexPaths::resolve().port;
    let token = read_auth_token()?;
    let client = reqwest::Client::new();
    let url = format!("http://localhost:{port}{path}");
    let req = match method {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        _ => return Err("Invalid method".into()),
    };
    let req = req
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Cortex-Request", "true");
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let resp = req.send().await.map_err(|e| {
        if e.is_connect() {
            "Cortex daemon not running. Start with: cortex serve".to_string()
        } else {
            format!("Request failed: {e}")
        }
    })?;
    let status = resp.status();
    let body_text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {e}"))?;
    if status.as_u16() == 403 {
        return Err("Admin commands require team mode. Run: cortex setup --team".to_string());
    }
    if status.as_u16() == 404 {
        return Err("Endpoint not found. Is the daemon up to date?".to_string());
    }
    let json: serde_json::Value = serde_json::from_str(&body_text).map_err(|_| {
        if body_text.is_empty() {
            format!("Empty response from daemon (HTTP {status})")
        } else {
            format!("Unexpected response (HTTP {status}): {body_text}")
        }
    })?;
    if !status.is_success() {
        let msg = json
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(msg.to_string());
    }
    Ok(json)
}

fn confirm_action(prompt: &str) -> bool {
    eprint!("{prompt} [y/N] ");
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

fn json_str(val: &serde_json::Value, key: &str) -> String {
    val.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn json_str_or(val: &serde_json::Value, key: &str, default: &str) -> String {
    val.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        .to_string()
}

fn json_field(val: &serde_json::Value, key: &str) -> String {
    match val.get(key) {
        Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
        Some(v) => v.to_string(),
        None => "-".to_string(),
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

    let (state, shutdown_rx) = state::initialize(&paths, true).expect("Failed to initialize state");

    if should_watch_spawn_parent(daemon_owner.as_deref()) {
        if let (Some(parent_pid), Some(parent_start_time)) = (parent_pid, parent_start_time) {
            let shutdown_tx = state.shutdown_tx.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(Duration::from_secs(ORPHAN_WATCH_INTERVAL_SECS));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if !process_pid_identity_matches(parent_pid, parent_start_time) {
                        eprintln!(
                            "[cortex] Spawn parent process {parent_pid} exited or was recycled; shutting down daemon"
                        );
                        if let Some(tx) = shutdown_tx.lock().await.take() {
                            let _ = tx.send(());
                        }
                        break;
                    }
                }
            });
        }
    }

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
    eprintln!("[cortex] Cleaned {cleaned_backups} old backups, kept {BACKUP_RETENTION_COUNT}");
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
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let (indexed, decayed) = {
                let conn = db_index.lock().await;
                let indexed = indexer::index_all(&conn, &home, owner_id);
                let decayed = indexer::decay_pass(&conn);
                (indexed, decayed)
            };
            eprintln!(
                "[cortex] Startup indexing complete: indexed {indexed}, decayed {decayed} scores in {}ms",
                started.elapsed().as_millis()
            );
        });
    }

    // ── Background embedding builder ────────────────────────────────
    if let Some(engine) = state.embedding_engine.clone() {
        let db = state.db.clone();
        tokio::spawn(async move {
            build_embeddings_async(&engine, &db).await;
        });
    } else {
        tokio::spawn(async {
            if let Some(dir) = embeddings::ensure_model_downloaded().await {
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
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;

                // Checkpoint WAL first to ensure consistency
                {
                    let conn = db_wal.lock().await;
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
        tokio::spawn(async move {
            // Run initial aging pass on startup
            {
                let conn = db_aging.lock().await;
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
                let conn = db_aging.lock().await;
                aging::run_aging_pass(&conn);
                compaction::run_compaction(&conn);
                cleanup_expired_rows(&conn, "Expired cleanup");
            }
        });
    }

    // ── Background crystallization pass every 2 hours ─────────────
    {
        let db_crystal = state.db.clone();
        let engine_crystal = state.embedding_engine.clone();
        let crystal_owner_id = state.default_owner_id;
        tokio::spawn(async move {
            // Initial pass on startup (after embeddings are built)
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            {
                let conn = db_crystal.lock().await;
                let result = crystallize::run_crystallize_pass(
                    &conn,
                    engine_crystal.as_deref(),
                    crystal_owner_id,
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
                let conn = db_crystal.lock().await;
                crystallize::run_crystallize_pass(
                    &conn,
                    engine_crystal.as_deref(),
                    crystal_owner_id,
                );
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

    let db_for_shutdown = state.db.clone();
    let router = server::build_router(state, paths.port);

    // Combine shutdown sources: HTTP /shutdown, extra (Ctrl+C or SCM stop)
    let shutdown_future = async {
        tokio::select! {
            _ = shutdown_rx => {
                eprintln!("[cortex] Shutdown requested via HTTP");
            }
            _ = extra_shutdown => {}
        }
    };

    server::run(router, &paths.bind, paths.port, &db_path, shutdown_future).await;

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
async fn build_embeddings_async(
    engine: &embeddings::EmbeddingEngine,
    db: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
) {
    let (unembedded_mem, unembedded_dec) = {
        let conn = db.lock().await;

        let mem: Vec<(i64, String)> = conn
            .prepare(
                "SELECT m.id, m.text FROM memories m \
                 WHERE m.status = 'active' \
                   AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'memory' AND e.target_id = m.id)",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        let dec: Vec<(i64, String)> = conn
            .prepare(
                "SELECT d.id, d.decision FROM decisions d \
                 WHERE d.status = 'active' \
                   AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'decision' AND e.target_id = d.id)",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        (mem, dec)
    };

    let total = unembedded_mem.len() + unembedded_dec.len();
    if total == 0 {
        return;
    }

    eprintln!("[embeddings] Building embeddings for {total} entries...");
    let mut computed = 0;

    let mut mem_results: Vec<(i64, Vec<u8>)> = Vec::new();
    for (id, text) in &unembedded_mem {
        if let Some(vec) = engine.embed(text) {
            mem_results.push((*id, embeddings::vector_to_blob(&vec)));
            computed += 1;
        }
    }

    let mut dec_results: Vec<(i64, Vec<u8>)> = Vec::new();
    for (id, text) in &unembedded_dec {
        if let Some(vec) = engine.embed(text) {
            dec_results.push((*id, embeddings::vector_to_blob(&vec)));
            computed += 1;
        }
    }

    {
        let conn = db.lock().await;
        for (id, blob) in &mem_results {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                 VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
                rusqlite::params![id, blob],
            );
        }
        for (id, blob) in &dec_results {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                 VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
                rusqlite::params![id, blob],
            );
        }
    }

    eprintln!("[embeddings] Built {computed}/{total} embeddings");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn spawn_single_response_server(
        listener: TcpListener,
        status_line: &str,
        content_type: &str,
        body: String,
    ) -> std::thread::JoinHandle<()> {
        let status_line = status_line.to_string();
        let content_type = content_type.to_string();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request_buffer = [0_u8; 2048];
                let _ = stream.read(&mut request_buffer);
                let response = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        })
    }

    #[test]
    fn rotate_backups_keeps_three_most_recent_files() {
        let backup_dir = temp_test_dir("backup_rotation");
        fs::create_dir_all(&backup_dir).unwrap();

        for idx in 0..5 {
            let path = backup_dir.join(format!("cortex-2026040{}.db", idx + 1));
            fs::write(&path, format!("backup-{idx}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let removed = rotate_backups(&backup_dir, BACKUP_RETENTION_COUNT).unwrap();
        assert_eq!(removed, 2);

        let mut remaining: Vec<String> = fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        remaining.sort();

        assert_eq!(
            remaining,
            vec![
                "cortex-20260403.db".to_string(),
                "cortex-20260404.db".to_string(),
                "cortex-20260405.db".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&backup_dir);
    }

    #[test]
    fn cleanup_bridge_backups_requires_schema_version_five_or_higher() {
        let home_dir = temp_test_dir("bridge_backups");
        let bridge_dir = home_dir.join("bridge-backups");
        fs::create_dir_all(&bridge_dir).unwrap();
        fs::write(bridge_dir.join("legacy.txt"), "legacy").unwrap();

        assert!(!cleanup_bridge_backups(&home_dir, 4));
        assert!(bridge_dir.exists());

        assert!(cleanup_bridge_backups(&home_dir, 5));
        assert!(!bridge_dir.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn rotate_log_file_replaces_existing_rotation_and_creates_fresh_log() {
        let home_dir = temp_test_dir("log_rotation");
        fs::create_dir_all(&home_dir).unwrap();

        let log_path = home_dir.join("daemon.log");
        let rotated_path = home_dir.join("daemon.log.1");
        fs::write(&rotated_path, "old-rotation").unwrap();
        fs::write(&log_path, vec![b'x'; (LOG_ROTATION_BYTES as usize) + 1]).unwrap();

        assert!(rotate_log_file(&home_dir, "daemon.log").unwrap());
        assert!(log_path.exists());
        assert_eq!(fs::metadata(&log_path).unwrap().len(), 0);
        assert_eq!(
            fs::metadata(&rotated_path).unwrap().len(),
            LOG_ROTATION_BYTES + 1
        );

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn run_backup_cleanup_dry_run_reports_files_without_deleting_them() {
        let backup_dir = temp_test_dir("backup_cleanup_dry_run");
        fs::create_dir_all(&backup_dir).unwrap();

        for idx in 0..4 {
            let path = backup_dir.join(format!("cortex-2026040{}.db", idx + 1));
            fs::write(&path, format!("backup-{idx}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let lines = run_backup_cleanup(&backup_dir, true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("DELETE backups/"));
        assert_eq!(fs::read_dir(&backup_dir).unwrap().count(), 4);

        let _ = fs::remove_dir_all(&backup_dir);
    }

    #[test]
    fn acquire_runtime_lock_rejects_duplicate_serve_startup() {
        let home_dir = temp_test_dir("runtime_lock");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let first_lock = acquire_runtime_lock(&paths).unwrap();
        let err = acquire_runtime_lock(&paths).unwrap_err();

        assert!(err.contains("another cortex instance"));

        drop(first_lock);
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn acquire_runtime_lock_waits_for_handoff_when_enabled() {
        let home_dir = temp_test_dir("runtime_lock_handoff");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let first_lock = acquire_runtime_lock(&paths).unwrap();
        std::env::set_var("CORTEX_WAIT_FOR_DAEMON_LOCK", "1");
        std::env::set_var("CORTEX_DAEMON_LOCK_WAIT_SECS", "1");

        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first_lock);
        });

        let second_lock = acquire_runtime_lock(&paths).expect("lock handoff should succeed");
        drop(second_lock);
        releaser.join().unwrap();

        std::env::remove_var("CORTEX_WAIT_FOR_DAEMON_LOCK");
        std::env::remove_var("CORTEX_DAEMON_LOCK_WAIT_SECS");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn run_stale_pid_cleanup_keeps_lock_file() {
        let home_dir = temp_test_dir("stale_pid_cleanup");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        fs::write(&paths.pid, "999999").unwrap();
        fs::write(&paths.lock, "lock-held").unwrap();

        let dry_run = run_stale_pid_cleanup(&paths, true);
        assert_eq!(
            dry_run,
            vec!["DELETE cortex.pid (process 999999 not running)"]
        );
        assert!(paths.pid.exists());
        assert!(paths.lock.exists());

        let apply = run_stale_pid_cleanup(&paths, false);
        assert_eq!(
            apply,
            vec!["DELETE cortex.pid (process 999999 not running)"]
        );
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_requires_parent_pid_only_for_non_control_center_owner() {
        assert!(spawned_owner_requires_parent_pid(Some("cli-mcp")));
        assert!(spawned_owner_requires_parent_pid(Some("plugin-claude")));
        assert!(!spawned_owner_requires_parent_pid(Some("control-center")));
        assert!(!spawned_owner_requires_parent_pid(None));
    }

    #[test]
    fn spawned_owner_runtime_claim_requires_parent_linkage_for_plugin_owner() {
        let home_dir = temp_test_dir("owner_runtime_parent");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let err =
            validate_spawned_owner_runtime_claim(&paths, Some("plugin-claude"), None, None, None)
                .unwrap_err();
        assert!(err.contains(SPAWN_PARENT_PID_ENV));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_missing_owner_token_when_parent_set() {
        let home_dir = temp_test_dir("owner_runtime_token");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let parent_start_time =
            process_pid_start_time(parent_pid).expect("current process start time should resolve");
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            Some(parent_start_time),
            None,
        )
        .unwrap_err();
        assert!(err.contains("missing ownership token"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_allows_unspawned_control_center_mode() {
        let home_dir = temp_test_dir("owner_runtime_unspawned");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        validate_spawned_owner_runtime_claim(&paths, Some("control-center"), None, None, None)
            .expect("direct control-center owner mode should remain compatible");

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_missing_parent_start_time_when_parent_set() {
        let home_dir = temp_test_dir("owner_runtime_parent_start");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains(SPAWN_PARENT_START_TIME_ENV));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_parent_start_time_mismatch() {
        let home_dir = temp_test_dir("owner_runtime_parent_start_mismatch");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            Some(0),
            Some("invalid-token"),
        )
        .unwrap_err();
        assert!(err.contains("start-time mismatch"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn parse_flag_usize_validates_and_parses_values() {
        let args = vec![
            "--agent".to_string(),
            "codex".to_string(),
            "--budget".to_string(),
            "900".to_string(),
        ];
        assert_eq!(parse_flag_usize(&args, "--budget").unwrap(), Some(900));

        let missing_value = vec!["--budget".to_string()];
        assert!(parse_flag_usize(&missing_value, "--budget")
            .unwrap_err()
            .contains("missing value"));

        let invalid_value = vec!["--budget".to_string(), "abc".to_string()];
        assert!(parse_flag_usize(&invalid_value, "--budget")
            .unwrap_err()
            .contains("invalid value"));

        let zero_value = vec!["--budget".to_string(), "0".to_string()];
        assert!(parse_flag_usize(&zero_value, "--budget")
            .unwrap_err()
            .contains("must be >= 1"));
    }

    #[test]
    fn resolve_boot_auth_header_prefers_api_key_and_falls_back_to_token_file() {
        let home_dir = temp_test_dir("boot_auth");
        fs::create_dir_all(&home_dir).unwrap();
        let token_path = home_dir.join("cortex.token");
        fs::write(&token_path, "local-token").unwrap();

        let explicit = resolve_boot_auth_header(&token_path, Some("ctx_remote"), true);
        assert_eq!(explicit, Some("Bearer ctx_remote".to_string()));

        let fallback = resolve_boot_auth_header(&token_path, None, true);
        assert_eq!(fallback, Some("Bearer local-token".to_string()));

        fs::write(&token_path, "   ").unwrap();
        assert_eq!(resolve_boot_auth_header(&token_path, None, true), None);
        assert_eq!(resolve_boot_auth_header(&token_path, None, false), None);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn parse_truthy_flag_accepts_expected_values() {
        assert!(parse_truthy_flag("1"));
        assert!(parse_truthy_flag("true"));
        assert!(parse_truthy_flag("YES"));
        assert!(parse_truthy_flag(" on "));
        assert!(!parse_truthy_flag("0"));
        assert!(!parse_truthy_flag("false"));
        assert!(!parse_truthy_flag(""));
    }

    #[test]
    fn resolve_client_target_inputs_prefers_cli_over_env_values() {
        let (base_url, api_key, local_owner_mode) = resolve_client_target_inputs(
            Some("https://cli.example"),
            Some("ctx_cli"),
            Some("https://env.example"),
            Some("ctx_env"),
            "http://127.0.0.1:7437",
        );
        assert_eq!(base_url, "https://cli.example");
        assert_eq!(api_key.as_deref(), Some("ctx_cli"));
        assert!(!local_owner_mode);
    }

    #[test]
    fn resolve_client_target_inputs_uses_env_and_disables_local_owner_mode() {
        let (base_url, api_key, local_owner_mode) = resolve_client_target_inputs(
            None,
            None,
            Some("https://100.101.102.103:7437"),
            Some("ctx_remote"),
            "http://127.0.0.1:7437",
        );
        assert_eq!(base_url, "https://100.101.102.103:7437");
        assert_eq!(api_key.as_deref(), Some("ctx_remote"));
        assert!(!local_owner_mode);
    }

    #[test]
    fn local_daemon_base_url_uses_loopback_for_wildcard_bind() {
        let home_dir = temp_test_dir("bind_wildcard");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "0.0.0.0".to_string();
        assert_eq!(local_daemon_base_url(&paths), "http://127.0.0.1:7437");

        paths.bind = "100.64.0.12".to_string();
        assert_eq!(local_daemon_base_url(&paths), "http://100.64.0.12:7437");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn startup_preflight_rejects_non_canonical_health_payload() {
        let home_dir = temp_test_dir("startup_preflight_noncanonical");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "127.0.0.1".to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("resolve listener addr").port();
        paths.port = port;
        let server =
            spawn_single_response_server(listener, "404 Not Found", "text/plain", "nope".into());

        let err = run_preflight(&paths).unwrap_err();
        assert!(err.contains("non-canonical payload"));

        server.join().expect("join response server");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn startup_preflight_rejects_different_cortex_runtime_identity() {
        let home_dir = temp_test_dir("startup_preflight_wrong_runtime");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "127.0.0.1".to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("resolve listener addr").port();
        paths.port = port;

        let payload = serde_json::json!({
            "status": "ok",
            "runtime": {
                "port": port,
                "token_path": "C:/other/cortex.token",
                "pid_path": "C:/other/cortex.pid",
                "db_path": "C:/other/cortex.db"
            },
            "stats": {
                "home": "C:/other"
            }
        })
        .to_string();
        let server = spawn_single_response_server(listener, "200 OK", "application/json", payload);

        let err = run_preflight(&paths).unwrap_err();
        assert!(err.contains("different Cortex runtime identity"));

        server.join().expect("join response server");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn is_local_client_base_url_accepts_configured_bind_host() {
        let home_dir = temp_test_dir("local_client_base");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        paths.bind = "100.64.0.12".to_string();
        assert!(is_local_client_base_url("http://100.64.0.12:7437", &paths));
        assert!(!is_local_client_base_url("http://100.64.0.12:9999", &paths));
        assert!(!is_local_client_base_url(
            "https://example.com:7437",
            &paths
        ));

        paths.bind = "0.0.0.0".to_string();
        assert!(is_local_client_base_url("http://127.0.0.1:7437", &paths));

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn remote_target_without_api_key_is_rejected() {
        let home_dir = temp_test_dir("remote_target_auth_required");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let err =
            ensure_remote_target_has_api_key("https://100.64.0.12:7437", None, &paths).unwrap_err();
        assert!(err.contains("requires an API key"));

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn local_target_without_api_key_is_allowed() {
        let home_dir = temp_test_dir("local_target_no_key");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        assert!(ensure_remote_target_has_api_key("http://127.0.0.1:7437", None, &paths).is_ok());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn invalid_client_target_url_is_rejected_cleanly() {
        let home_dir = temp_test_dir("invalid_client_target_url");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let invalid_scheme =
            ensure_remote_target_has_api_key("ftp://example.com", Some("ctx_key"), &paths)
                .unwrap_err();
        assert!(invalid_scheme.contains("Unsupported Cortex target URL scheme"));

        let embedded_creds = ensure_remote_target_has_api_key(
            "https://user:pass@example.com",
            Some("ctx_key"),
            &paths,
        )
        .unwrap_err();
        assert!(embedded_creds.contains("must not include embedded credentials"));

        let query_url = ensure_remote_target_has_api_key(
            "https://example.com?debug=1",
            Some("ctx_key"),
            &paths,
        )
        .unwrap_err();
        assert!(query_url.contains("must not include query parameters"));

        let _ = fs::remove_dir_all(&home_dir);
    }
}
