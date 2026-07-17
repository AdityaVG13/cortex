use super::common::{is_cli_option_token, validate_cli_options_or_exit};
use crate::{auth, db};
use chrono::{Local, Utc};
use std::path::Path;

pub(crate) const BACKUP_RETENTION_COUNT: usize = 3;
const BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION: i32 = 5;
const LOG_ROTATION_BYTES: u64 = 1024 * 1024;
const STARTUP_LOG_FILES: &[&str] = &["daemon.log", "daemon.err.log", "daemon.out.log", "mcp-crash.log", "rust-daemon.err.log"];

pub(crate) fn should_backup(backup_dir: &Path) -> bool {
    let last_backup_file = backup_dir.join(".last_backup");
    let Ok(ts) = std::fs::read_to_string(last_backup_file) else {
        return true;
    };
    chrono::DateTime::parse_from_rfc3339(&ts)
        .map(|last_backup| (Utc::now() - last_backup.with_timezone(&Utc)).num_hours() >= 24)
        .unwrap_or(true)
}

pub(crate) fn cleanup_backup_retention(backup_dir: &Path) -> usize {
    let mut backups = std::fs::read_dir(backup_dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    name.starts_with("cortex-") && name.ends_with(".db") && !name.contains(".corrupt")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    backups.sort_by_key(|entry| entry.metadata().ok().and_then(|meta| meta.modified().ok()));
    let remove_count = backups.len().saturating_sub(BACKUP_RETENTION_COUNT);
    for entry in backups.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(entry.path());
    }
    remove_count
}

pub(crate) fn cleanup_bridge_backups(home: &Path, schema_version: i32) -> bool {
    if schema_version < BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION {
        return false;
    }
    std::fs::remove_dir_all(home.join("bridge-backups")).is_ok()
}

pub(crate) fn cleanup_expired_rows(conn: &rusqlite::Connection, label: &str) {
    match db::delete_expired_entries(conn) {
        Ok(counts) if counts.memories_deleted > 0 || counts.decisions_deleted > 0 => {
            eprintln!("[cortex] {label}: deleted {} expired memories and {} expired decisions", counts.memories_deleted, counts.decisions_deleted);
        }
        Ok(_) => {}
        Err(err) => eprintln!("[cortex] Warning: expired-row cleanup failed: {err}"),
    }
}

pub(crate) fn run_stale_pid_cleanup(paths: &auth::CortexPaths, dry_run: bool) -> Vec<String> {
    let Some(pid) = std::fs::read_to_string(&paths.pid).ok().and_then(|value| value.trim().parse::<u32>().ok()) else {
        return Vec::new();
    };
    if pid == std::process::id() || std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return Vec::new();
    }
    if !dry_run {
        let _ = std::fs::remove_file(&paths.pid);
    }
    vec![format!("DELETE cortex.pid (process {pid} not running)")]
}

pub(crate) fn rotate_startup_logs(home: &Path) -> usize {
    let mut rotated = 0;
    for file_name in STARTUP_LOG_FILES {
        let log_path = home.join(file_name);
        let Ok(metadata) = std::fs::metadata(&log_path) else {
            continue;
        };
        if metadata.len() <= LOG_ROTATION_BYTES {
            continue;
        }
        let rotated_path = home.join(format!("{file_name}.1"));
        let _ = std::fs::remove_file(&rotated_path);
        if std::fs::rename(&log_path, &rotated_path).is_ok() {
            let _ = std::fs::File::create(&log_path);
            rotated += 1;
        }
    }
    rotated
}

pub(crate) fn create_backup(db_path: &Path, backup_dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(backup_dir).map_err(|err| format!("create backup dir: {err}"))?;
    let dest = backup_dir.join(format!("cortex-{}.db", Local::now().format("%Y%m%d")));
    std::fs::copy(db_path, &dest).map_err(|err| format!("copy db: {err}"))?;
    let _ = cleanup_backup_retention(backup_dir);
    let _ = std::fs::write(backup_dir.join(".last_backup"), Utc::now().to_rfc3339());
    Ok(dest.to_string_lossy().to_string())
}

pub(crate) fn event_type_count(conn: &rusqlite::Connection, event_type: &str) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM events WHERE type = ?1", rusqlite::params![event_type], |row| row.get(0)).unwrap_or(0)
}

pub(crate) fn top_event_type_counts(conn: &rusqlite::Connection, limit: usize) -> Vec<(String, i64)> {
    let Ok(mut stmt) = conn.prepare("SELECT type, COUNT(*) FROM events GROUP BY type ORDER BY COUNT(*) DESC LIMIT ?1") else {
        return Vec::new();
    };
    stmt.query_map([limit as i64], |row| Ok((row.get(0)?, row.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub(crate) fn run_cleanup_cli(paths: &auth::CortexPaths, dry_run: bool, include_events: bool, _max_event_passes: usize) {
    let mut actions = Vec::new();
    actions.push(format!("{} old backups", if dry_run { "Would prune" } else { "Pruned" }));
    if !dry_run {
        let removed = cleanup_backup_retention(&paths.home.join("backups"));
        actions[0] = format!("Pruned {removed} old backups");
        let rotated = rotate_startup_logs(&paths.home);
        actions.push(format!("Rotated {rotated} startup logs"));
        let _ = auth::cleanup_stale_pid_lock(paths);
    }
    if include_events {
        actions.push("EVENTS cleanup is handled by the storage governor".to_string());
    }
    for action in actions {
        println!("{action}");
    }
}

pub(crate) fn run_backup_cli(paths: &auth::CortexPaths) {
    match create_backup(&paths.db, &paths.home.join("backups")) {
        Ok(path) => println!("Backup created: {path}"),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn run_restore_cli(paths: &auth::CortexPaths, args: &[String]) {
    let restore_file = match args.get(2) {
        Some(path) if !is_cli_option_token(path) => path,
        _ => {
            eprintln!("Usage: cortex restore <backup-file.db>");
            std::process::exit(1);
        }
    };
    validate_cli_options_or_exit(&args[3..], &[], &["--skip-verification"]);
    let pre_backup = paths.home.join(format!("cortex.pre-restore.{}.db", Local::now().format("%Y%m%dT%H%M%S")));
    if let Err(err) = std::fs::copy(&paths.db, &pre_backup) {
        eprintln!("[cortex] Error: failed to create pre-restore backup: {err}");
        std::process::exit(1);
    }
    if let Err(err) = std::fs::copy(restore_file, &paths.db) {
        eprintln!("[cortex] Error: failed to restore backup: {err}");
        eprintln!("[cortex] Pre-restore backup preserved at: {}", pre_backup.display());
        std::process::exit(1);
    }
    println!("Restore complete. Pre-restore backup preserved at: {}", pre_backup.display());
}
