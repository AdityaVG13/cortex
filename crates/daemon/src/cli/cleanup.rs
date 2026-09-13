use super::common::{is_cli_option_token, validate_cli_options_or_exit};
use crate::{auth, db};
use chrono::{Local, Utc};
use std::path::Path;

pub(crate) const BACKUP_RETENTION_COUNT: usize = 3;
const LOG_ROTATION_BYTES: u64 = 1024 * 1024;
const STARTUP_LOG_FILES: &[&str] = &["daemon.log", "daemon.err.log", "daemon.out.log", "mcp-crash.log", "rust-daemon.err.log"];

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
    // Coherent snapshot through the online backup API (a raw copy of a live
    // .db ignores WAL frames) plus a manifest beside it.
    db::backup::backup_to(db_path, &dest)?;
    let _ = cleanup_backup_retention(backup_dir);
    let _ = std::fs::write(backup_dir.join(".last_backup"), Utc::now().to_rfc3339());
    Ok(dest.to_string_lossy().to_string())
}

pub(crate) fn event_type_count(conn: &rusqlite::Connection, event_type: &str) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM events WHERE type = ?1", rusqlite::params![event_type], |row| row.get(0))
        .unwrap_or(0)
}

pub(crate) fn top_event_type_counts(conn: &rusqlite::Connection, limit: usize) -> Vec<(String, i64)> {
    let Ok(mut stmt) = conn.prepare("SELECT type, COUNT(*) FROM events GROUP BY type ORDER BY COUNT(*) DESC LIMIT ?1") else {
        return Vec::new();
    };
    stmt.query_map([limit as i64], |row| Ok((row.get(0)?, row.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

pub fn run_cleanup_cli(paths: &auth::CortexPaths, dry_run: bool, include_events: bool, _max_event_passes: usize) {
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

pub fn run_backup_cli(paths: &auth::CortexPaths) {
    match create_backup(&paths.db, &paths.home.join("backups")) {
        Ok(path) => println!("Backup created: {path}"),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}

pub fn run_restore_cli(paths: &auth::CortexPaths, args: &[String]) {
    let restore_file = match args.get(2) {
        Some(path) if !is_cli_option_token(path) => path,
        _ => {
            eprintln!("Usage: cortex restore <backup-file.db>");
            std::process::exit(1);
        }
    };
    validate_cli_options_or_exit(&args[3..], &[], &["--skip-verification"]);
    // Serve excludes via flock on `paths.lock`. Take that lock first so a
    // worker cannot start in the window between the pid check and the copy.
    // A live pid after we hold the lock is an old daemon without flock, or
    // PID reuse of a non-daemon; refuse either rather than copy over it.
    // Do not call `cleanup_stale_pid_lock` here: it would try_lock a second
    // fd of the flock we already hold.
    let _lock = match auth::acquire_daemon_lock(paths) {
        Ok(lock) => lock,
        Err(err) => {
            eprintln!("[cortex] Error: daemon appears active ({err}).");
            eprintln!("[cortex] Stop the daemon before restoring; see `cortex paths --json` for the home it is using.");
            std::process::exit(1);
        }
    };
    if let Some(pid) = auth::pid_file_live_pid(paths) {
        eprintln!("[cortex] Error: daemon appears active (pid {pid} per {}).", paths.pid.display());
        eprintln!("[cortex] Stop the daemon before restoring; see `cortex paths --json` for the home it is using.");
        std::process::exit(1);
    }
    let _ = auth::cleanup_stale_pid_file(paths);
    match db::backup::restore_from(Path::new(restore_file), &paths.db, &paths.home) {
        Ok(report) => {
            let verified = report.integrity_ok && report.sample_reads_ok;
            println!(
                "Restore complete: epoch {} (was {}); integrity={} sample_reads={} records={} decisions={} memories={} aliases_expired={} projections_rebuilt={}",
                report.new_restore_epoch, report.previous_restore_epoch, report.integrity_ok, report.sample_reads_ok, report.records, report.decisions, report.memories, report.aliases_expired, report.projections_rebuilt
            );
            println!("Verification report: {}", report.report_path.display());
            if !verified {
                eprintln!("[cortex] WARNING: restore verification FAILED; the pre-restore copy in {} is intact", paths.home.display());
                std::process::exit(1);
            }
        }
        Err(err) => {
            eprintln!("[cortex] Error: restore failed: {err}");
            std::process::exit(1);
        }
    }
}
