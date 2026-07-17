use super::common::{is_cli_option_token, validate_cli_options_or_exit};
use crate::auth;
use crate::compaction;
use crate::db;
use chrono::Utc;
use std::path::Path;
use std::time::Duration;
pub(crate) const BACKUP_RETENTION_COUNT: usize = 3;
const BRIDGE_BACKUP_CLEANUP_SCHEMA_VERSION: i32 = 5;
const LOG_ROTATION_BYTES: u64 = 1024 * 1024;
const STARTUP_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];
pub(crate) fn should_backup(backup_dir: &Path) -> bool {
    let last_backup_file = backup_dir.join(".last_backup");
    if !last_backup_file.exists() {
        return true;
    }
    match std::fs::read_to_string(&last_backup_file) {
        Ok(ts) => {
            if let Ok(last_backup) = chrono::DateTime::parse_from_rfc3339(&ts) {
                let now = Utc::now();
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
    backups.sort_by_key(|entry| entry.metadata().ok().and_then(|m| m.modified().ok()));
    let mut removed = 0usize;
    for backup in backups.iter().take(backups.len() - keep) {
        std::fs::remove_file(backup.path())?;
        removed += 1;
    }
    Ok(removed)
}
pub(crate) fn cleanup_backup_retention(backup_dir: &Path) -> usize {
    match rotate_backups(backup_dir, BACKUP_RETENTION_COUNT) {
        Ok(removed) => removed,
        Err(e) => {
            eprintln!("[cortex] Warning: backup rotation failed: {e}");
            0
        }
    }
}
pub(crate) fn cleanup_bridge_backups(home: &Path, schema_version: i32) -> bool {
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
pub(crate) fn cleanup_expired_rows(conn: &rusqlite::Connection, label: &str) {
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
pub(crate) fn rotate_startup_logs(home: &Path) -> usize {
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
pub(crate) fn run_stale_pid_cleanup(paths: &auth::CortexPaths, dry_run: bool) -> Vec<String> {
    let Some(pid) = auth::stale_pid_candidate(paths) else {
        return Vec::new();
    };
    let lines = vec![format!("DELETE cortex.pid (process {pid} not running)")];
    if !dry_run {
        let _ = auth::cleanup_stale_pid_lock(paths);
    }
    lines
}
pub(crate) fn create_backup(db_path: &Path, backup_dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(backup_dir).map_err(|e| format!("create backup dir: {e}"))?;
    let timestamp = chrono::Local::now().format("%Y%m%d");
    let dest = backup_dir.join(format!("cortex-{timestamp}.db"));
    std::fs::copy(db_path, &dest).map_err(|e| format!("copy db: {e}"))?;
    eprintln!("[cortex] Backup created: {}", dest.display());
    let _ = cleanup_backup_retention(backup_dir);
    let last_backup_file = backup_dir.join(".last_backup");
    let now_ts = chrono::Utc::now().to_rfc3339();
    if let Err(e) = std::fs::write(&last_backup_file, now_ts) {
        eprintln!("[cortex] Warning: failed to write last_backup timestamp: {e}");
    }
    Ok(dest.to_string_lossy().to_string())
}
pub(crate) fn event_type_count(conn: &rusqlite::Connection, event_type: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM events WHERE type = ?1",
        rusqlite::params![event_type],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
}
pub(crate) fn top_event_type_counts(
    conn: &rusqlite::Connection,
    limit: usize,
) -> Vec<(String, i64)> {
    let mut statement = match conn.prepare(
        "SELECT type, COUNT(*) AS cnt FROM events GROUP BY type ORDER BY cnt DESC LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    let rows = match statement.query_map([limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    rows.filter_map(Result::ok).collect()
}
pub(crate) fn run_event_compaction_cleanup(
    db_path: &Path,
    dry_run: bool,
    max_passes: usize,
) -> Result<Vec<String>, String> {
    if !db_path.exists() {
        return Ok(vec![
            "EVENTS skip: database file is missing; nothing to compact".to_string(),
        ]);
    }
    let conn = db::open(db_path).map_err(|e| format!("events cleanup open db: {e}"))?;
    db::configure(&conn).map_err(|e| format!("events cleanup configure db: {e}"))?;
    let before_nonboot = compaction::non_boot_event_count(&conn);
    let before_decision_stored = event_type_count(&conn, "decision_stored");
    let mut lines = vec![format!(
        "EVENTS before: pressure={} nonboot_rows={} decision_stored_rows={} (soft={} hard={})",
        compaction::classify_event_pressure(before_nonboot),
        before_nonboot,
        before_decision_stored,
        compaction::EVENT_NONBOOT_SOFT_LIMIT_ROWS,
        compaction::EVENT_NONBOOT_HARD_LIMIT_ROWS,
    )];
    let top_before = top_event_type_counts(&conn, 5);
    if !top_before.is_empty() {
        lines.push("EVENTS top types before:".to_string());
        for (event_type, count) in top_before {
            lines.push(format!("  {event_type:<24} {count}"));
        }
    }
    if dry_run {
        lines.push(format!(
"EVENTS dry-run only: rerun with `cortex cleanup --events --max-passes {max_passes}` to apply compaction"));
        return Ok(lines);
    }
    for pass in 1..=max_passes.max(1) {
        let nonboot_before_pass = compaction::non_boot_event_count(&conn);
        let maybe_result = compaction::run_compaction_governor(&conn);
        let Some(result) = maybe_result else {
            lines.push(format!(
                "EVENTS pass {pass}: no additional compaction needed (pressure={})",
                compaction::classify_event_pressure(nonboot_before_pass)
            ));
            break;
        };
        let nonboot_after_pass = compaction::non_boot_event_count(&conn);
        let pressure_after_pass = compaction::classify_event_pressure(nonboot_after_pass);
        lines.push(format!(
"EVENTS pass {pass}: pruned events={} benchmark={} archived={} expired={} feedback={} | nonboot {} -> {} ({pressure_after_pass})",
result.events_pruned,result.benchmark_pruned,result.archived_text_stripped,result.expired_pruned,result.feedback_aggregated,
nonboot_before_pass,nonboot_after_pass,));
        if nonboot_after_pass >= nonboot_before_pass || pressure_after_pass == "normal" {
            break;
        }
    }
    let after_nonboot = compaction::non_boot_event_count(&conn);
    let after_decision_stored = event_type_count(&conn, "decision_stored");
    lines.push(format!(
        "EVENTS after: pressure={} nonboot_rows={} decision_stored_rows={}",
        compaction::classify_event_pressure(after_nonboot),
        after_nonboot,
        after_decision_stored,
    ));
    let top_after = top_event_type_counts(&conn, 5);
    if !top_after.is_empty() {
        lines.push("EVENTS top types after:".to_string());
        for (event_type, count) in top_after {
            lines.push(format!("  {event_type:<24} {count}"));
        }
    }
    Ok(lines)
}
pub(crate) fn run_cleanup_cli(
    paths: &auth::CortexPaths,
    dry_run: bool,
    include_events: bool,
    max_event_passes: usize,
) {
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
    if include_events {
        match run_event_compaction_cleanup(&paths.db, dry_run, max_event_passes) {
            Ok(event_lines) => lines.extend(event_lines),
            Err(err) => lines.push(format!("EVENTS cleanup failed: {err}")),
        }
    }
    if lines.is_empty() {
        println!("No cleanup actions needed");
        return;
    }
    for line in lines {
        println!("{line}");
    }
}
pub(crate) fn run_backup_cli(paths: &auth::CortexPaths) {
    let db_path = paths.db.clone();
    let home_dir = paths.home.clone();
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
        Ok(path) => println!("Backup created: {path}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
pub(crate) fn run_restore_cli(paths: &auth::CortexPaths, args: &[String]) {
    let restore_file = match args.get(2) {
        Some(f) if !is_cli_option_token(f) => f.clone(),
        None => {
            eprintln!("Usage: cortex restore <backup-file.db>");
            eprintln!("       cortex restore <backup-file.db> --skip-verification");
            eprintln!();
            eprintln!("Example: cortex restore ~/.cortex/backups/cortex-20260407.db");
            std::process::exit(1);
        }
        Some(_) => {
            eprintln!("Usage: cortex restore <backup-file.db>");
            eprintln!("       cortex restore <backup-file.db> --skip-verification");
            eprintln!();
            eprintln!("Example: cortex restore ~/.cortex/backups/cortex-20260407.db");
            std::process::exit(1);
        }
    };
    validate_cli_options_or_exit(&args[3..], &[], &["--skip-verification"]);
    let skip_verification = args.iter().any(|a| a == "--skip-verification");
    let paths_check = auth::CortexPaths::resolve();
    let daemon_running = paths_check.pid.exists();
    if daemon_running {
        eprintln!(
            "[cortex] Warning: Daemon PID file exists at {}",
            paths_check.pid.display()
        );
        eprintln!("[cortex] Please stop the daemon first with: Ctrl+C or kill the daemon process");
        eprintln!("[cortex] Continuing restore anyway...");
        std::thread::sleep(Duration::from_millis(500));
    }
    let db_path = paths.db.clone();
    let home_dir = paths.home.clone();
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
    eprintln!("[cortex] Restoring from: {}", restore_file);
    if let Err(e) = std::fs::copy(&restore_file, &db_path) {
        eprintln!("[cortex] Error: failed to restore backup: {e}");
        eprintln!(
            "[cortex] Pre-restore backup preserved at: {}",
            pre_backup.display()
        );
        std::process::exit(1);
    }
    if !skip_verification {
        eprintln!("[cortex] Verifying integrity of restored database...");
        match db::open(&db_path) {
            Ok(conn) => {
                if !db::verify_integrity(&conn).unwrap_or(false) {
                    eprintln!("[cortex] Error: restored database failed integrity check!");
                    eprintln!("[cortex] Rolling back to pre-restore backup...");
                    if let Err(e) = std::fs::copy(&pre_backup, &db_path) {
                        eprintln!("[cortex] Critical: rollback failed! DB may be corrupted: {e}");
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
                    eprintln!("[cortex] Critical: rollback failed! DB may be corrupted: {e}");
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
