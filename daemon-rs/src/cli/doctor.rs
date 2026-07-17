use super::cleanup::{event_type_count, top_event_type_counts};
use crate::auth;
use crate::compaction;
use crate::db;
use std::collections::HashSet;
pub(crate) fn run_doctor_cli(paths: &auth::CortexPaths) {
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
    let missing_tables: Vec<&str> = expected_tables.iter().copied().filter(|table| !db::table_exists(&conn, table)).collect();
    if missing_tables.is_empty() {
        println!("[doctor] OK tables: {}/{}", expected_tables.len(), expected_tables.len());
    } else {
        println!("[doctor] FAIL tables missing: {}", missing_tables.join(", "));
    }
    let (schema_current, pending_versions) = match db::pending_migration_versions(&conn) {
        Ok(pending) => (pending.is_empty(), pending),
        Err(e) => {
            println!("[doctor] FAIL schema status: {e}");
            (false, vec![])
        }
    };
    if schema_current {
        let expected_versions: HashSet<&'static str> = db::migration_definitions().iter().map(|(version, _)| *version).collect();
        let (applied, marker_rows) = db::applied_migration_versions(&conn)
            .map(|versions| {
                let schema_applied = versions.iter().filter(|version| expected_versions.contains(version.as_str())).count();
                let non_schema_markers = versions.len().saturating_sub(schema_applied);
                (schema_applied, non_schema_markers)
            })
            .unwrap_or((0, 0));
        println!("[doctor] OK schema current: {applied}/{} migrations applied", db::migration_definitions().len());
        if marker_rows > 0 {
            println!("[doctor] INFO schema markers: {marker_rows} non-schema row(s) ignored");
        }
    } else if !pending_versions.is_empty() {
        println!("[doctor] FAIL schema pending: {}", pending_versions.join(", "));
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
    let fts_tables_ok = db::table_exists(&conn, "memories_fts") && db::table_exists(&conn, "decisions_fts");
    let fts_queries_ok = conn.query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get::<_, i64>(0)).is_ok()
        && conn.query_row("SELECT COUNT(*) FROM decisions_fts", [], |row| row.get::<_, i64>(0)).is_ok();
    let fts_triggers_ok = fts_trigger_names.iter().all(|name| {
        conn.query_row("SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1 LIMIT 1", rusqlite::params![name], |_| Ok(()))
            .is_ok()
    });
    let fts_ok = fts_tables_ok && fts_queries_ok && fts_triggers_ok;
    if fts_ok {
        println!("[doctor] OK fts indexes");
    } else {
        println!("[doctor] FAIL fts indexes");
    }
    let nonboot_event_rows = compaction::non_boot_event_count(&conn);
    let decision_stored_rows = event_type_count(&conn, "decision_stored");
    let event_pressure = compaction::classify_event_pressure(nonboot_event_rows);
    println!(
        "[doctor] EVENT pressure={} nonboot_rows={} decision_stored_rows={} (soft={} hard={})",
        event_pressure,
        nonboot_event_rows,
        decision_stored_rows,
        compaction::EVENT_NONBOOT_SOFT_LIMIT_ROWS,
        compaction::EVENT_NONBOOT_HARD_LIMIT_ROWS,
    );
    let top_event_types = top_event_type_counts(&conn, 5);
    if !top_event_types.is_empty() {
        println!("[doctor] EVENT top types:");
        for (event_type, count) in top_event_types {
            println!("  {:<24} {}", event_type, count);
        }
    }
    if event_pressure != "normal" {
        println!(
            "[doctor] WARN elevated event pressure detected; run `cortex cleanup --events --dry-run` to preview one-time remediation."
        );
    }
    let all_ok = missing_tables.is_empty() && schema_current && integrity_ok && fts_ok;
    if all_ok {
        println!("[doctor] GREEN");
        return;
    }
    println!("[doctor] RED");
    std::process::exit(1);
}
