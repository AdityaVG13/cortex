use super::verify_integrity;
use crate::db::{
    RepairError, RepairResult, SQLITE_BUSY_TIMEOUT_MS, configure, create_team_mode_tables,
    current_mode, initialize_schema, pragma_column_names, table_exists,
};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

fn sql_literal(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".to_string(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(f) => format!("{f}"),
        ValueRef::Text(t) => format!("'{}'", String::from_utf8_lossy(t).replace('\'', "''")),
        ValueRef::Blob(b) => format!(
            "X'{}'",
            b.iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>()
        ),
    }
}

fn salvage_rows(rows: &mut rusqlite::Rows<'_>, table: &str, columns: usize) -> Vec<Vec<String>> {
    const MAX_CONSECUTIVE_ROW_ERRORS: usize = 100;
    let mut values = Vec::new();
    let mut errors = 0;
    loop {
        match rows.next() {
            Ok(Some(row)) => {
                errors = 0;
                values.push(
                    (0..columns)
                        .map(|i| {
                            row.get_ref(i)
                                .map(sql_literal)
                                .unwrap_or_else(|_| "NULL".to_string())
                        })
                        .collect(),
                );
            }
            Ok(None) => break,
            Err(e) => {
                errors += 1;
                eprintln!("[cortex] auto_repair: row error in '{table}': {e} -- skipping row");
                if errors >= MAX_CONSECUTIVE_ROW_ERRORS {
                    eprintln!(
                        "[cortex] auto_repair: '{table}' hit {MAX_CONSECUTIVE_ROW_ERRORS} consecutive row errors -- abandoning remaining rows of this table"
                    );
                    break;
                }
            }
        }
    }
    values
}

fn salvage_table(
    corrupt: &Connection,
    fresh: &Connection,
    table: &str,
) -> Result<usize, RepairError> {
    if !table_exists(corrupt, table) {
        eprintln!("[cortex] auto_repair: table '{table}' not found in corrupt DB, skipping");
        return Ok(0);
    }
    let columns = pragma_column_names(corrupt, table).map_err(RepairError::Export)?;
    if columns.is_empty() {
        eprintln!("[cortex] auto_repair: table '{table}' has no columns, skipping");
        return Ok(0);
    }
    let fresh_columns = pragma_column_names(fresh, table).map_err(RepairError::Import)?;
    let common_columns: Vec<&str> = columns
        .iter()
        .filter(|c| fresh_columns.contains(c))
        .map(String::as_str)
        .collect();
    if common_columns.is_empty() {
        eprintln!(
            "[cortex] auto_repair: table '{table}' has no columns in common with the fresh schema, skipping"
        );
        return Ok(0);
    }
    let col_list = common_columns.join(", ");
    let mut stmt = match corrupt.prepare(&format!("SELECT {col_list} FROM {table}")) {
        Ok(stmt) => stmt,
        Err(e) => {
            eprintln!("[cortex] auto_repair: failed to prepare SELECT on '{table}': {e}");
            return Ok(0);
        }
    };
    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("[cortex] auto_repair: failed to query '{table}': {e}");
            return Ok(0);
        }
    };
    let values = salvage_rows(&mut rows, table, common_columns.len());
    // Count actual inserts, not exports: INSERT OR IGNORE can discard a row.
    let exported = values.len();
    let mut inserted = 0;
    for row in values {
        let sql = format!(
            "INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({});",
            row.join(", ")
        );
        if let Err(e) = fresh.execute_batch(&sql) {
            eprintln!("[cortex] auto_repair: insert skipped ({e}): {sql:.80}");
            continue;
        }
        inserted += usize::try_from(fresh.changes()).unwrap_or(0);
    }
    eprintln!("[cortex] auto_repair: {inserted}/{exported} rows recovered into '{table}'");
    Ok(inserted)
}

pub fn auto_repair(db_path: &Path, timestamp: &str) -> Result<RepairResult, RepairError> {
    eprintln!(
        "[cortex] auto_repair: beginning dump-and-rebuild of {}",
        db_path.display()
    );
    let corrupt_conn = Connection::open(db_path).map_err(RepairError::OpenCorrupt)?;
    let busy_timeout_ms = SQLITE_BUSY_TIMEOUT_MS;
    let _ = corrupt_conn.execute_batch(&format!(
        "PRAGMA busy_timeout = {busy_timeout_ms}; PRAGMA query_only = ON;"
    ));
    pub const DATA_TABLES: &[&str] = &[
        "memories",
        "decisions",
        "embeddings",
        "co_occurrence",
        "events",
        "activities",
        "messages",
        "sessions",
        "tasks",
        "feed",
        "feed_acks",
        "context_cache",
        "focus_sessions",
        "recall_feedback",
        "memory_clusters",
        "cluster_members",
        "locks",
        // History and governance: losing these silently rewrites provenance.
        "traces",
        "versions",
        "head_state",
        "decision_conflicts",
        "boot_audits",
        "agent_feedback",
        "client_permissions",
        "promotion_policy",
        "decision_observation_evidence",
        "entities",
        "entity_aliases",
        "entity_mentions",
        // Authoritative records (additive schema); FK order matters.
        "brain_meta",
        "scopes",
        "commits",
        "sources",
        "records",
        "revisions",
        "revision_parents",
        "record_heads",
        "revision_sources",
        "relations",
        "digests",
        "addresses",
        "threads",
        "thread_members",
        "obligations",
        "change_items",
        "outbox",
        "guard_epochs",
        "erasures",
        "operation_ledger",
    ];
    // Team identity tables. initialize_schema does not create these, and a
    // team-mode DB that loses them during repair silently boots in solo mode
    // with every member's credential hash destroyed (all ctx_ keys die, no
    // warning). Detect team mode up front, create the team schema in the
    // fresh DB, and salvage the identity rows alongside the data tables.
    const TEAM_IDENTITY_TABLES: &[&str] = &["config", "users", "teams", "team_members"];
    // Team-ness detection must not rely solely on the config mode row: when
    // config itself is the damaged table (its rows unreadable/lost),
    // current_mode falls back to "solo" and the identity tables would be
    // silently excluded from salvage -- every credential hash dropped with no
    // warning, the same loss bed9809 fixed in the fully-readable-config case.
    // The `users` table only exists in team-mode DBs (initialize_schema does
    // not create it), so its presence disambiguates the fallback. Every writer
    // stores exactly 'team' or 'solo' (create_team_mode_tables seed,
    // migrate_to_team_mode flip, the post-salvage guard downgrade), so a
    // READABLE row whose value is neither is payload-level damage to that
    // cell, not a verdict -- it takes the same inference as the unreadable
    // arm. A plainly readable 'solo' row keeps the exact solo verdict (no
    // inference for a DB already downgraded to solo before the corruption --
    // no false warnings), and a plainly readable 'team' row stays exact.
    let corrupt_team_mode = match corrupt_conn.query_row(
        "SELECT value FROM config WHERE key = 'mode' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(mode) if mode == "team" => true,
        Ok(mode) if mode == "solo" => false,
        Ok(_) | Err(_) => table_exists(&corrupt_conn, "users"),
    };
    let tables: Vec<&str> = if corrupt_team_mode {
        DATA_TABLES
            .iter()
            .copied()
            .chain(TEAM_IDENTITY_TABLES.iter().copied())
            .collect()
    } else {
        DATA_TABLES.to_vec()
    };
    // The fresh schema must exist BEFORE salvage so each table's INSERT column
    // list can be intersected with the columns the fresh DB actually has.
    // Boot-time migrations add columns (e.g. compressed_text, age_tier) that
    // initialize_schema alone does not; naming them in the INSERT fails every
    // row ("no column named ...") while exports still counted as recovered.
    let tmp_path = db_path.with_extension("repair_tmp");
    let _ = std::fs::remove_file(&tmp_path);
    let mut fresh = Connection::open(&tmp_path).map_err(RepairError::OpenFresh)?;
    configure(&fresh).map_err(RepairError::Import)?;
    initialize_schema(&fresh).map_err(RepairError::Import)?;
    // Boot-time schema the salvage loop intersects columns against.
    cortex_logic::traces::migrate_history_tables(&fresh).map_err(RepairError::Import)?;
    crate::graph::migrate_entity_tables(&fresh).map_err(RepairError::Import)?;
    crate::db::run_pending_migrations_quiet(&mut fresh);
    crate::db::records::ensure_authoritative_schema(&fresh).map_err(RepairError::Import)?;
    if corrupt_team_mode {
        create_team_mode_tables(&fresh).map_err(RepairError::Import)?;
        // create_team_mode_tables seeds config.mode='solo'; the salvage loop
        // inserts with INSERT OR IGNORE, so the seeded row would outcompete
        // the corrupt DB's mode='team' row. Drop the seed before salvage; if
        // the corrupt DB's config cannot be salvaged, current_mode falls back
        // to solo below and the post-salvage guard warns loudly.
        fresh
            .execute("DELETE FROM config WHERE key = 'mode'", [])
            .map_err(RepairError::Import)?;
    }
    fresh
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(RepairError::Import)?;
    let mut memories_recovered = 0;
    let mut decisions_recovered = 0;
    for table in tables {
        let recovered = salvage_table(&corrupt_conn, &fresh, table)?;
        match table {
            "memories" => memories_recovered = recovered,
            "decisions" => decisions_recovered = recovered,
            _ => {}
        }
    }
    drop(corrupt_conn);
    fresh
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(RepairError::Import)?;
    fresh.execute_batch("INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags) SELECT id, text, source, tags FROM memories; INSERT OR IGNORE INTO decisions_fts(rowid, decision, context) SELECT id, decision, context FROM decisions;").map_err(RepairError::Import)?;
    fresh
        .execute_batch("VACUUM;")
        .map_err(RepairError::Import)?;
    // Team-mode safety net: booting in team mode with an empty users roster
    // bricks every credential (all ctx_ keys fail, the caller-less runtime
    // token is admin-locked), which is worse than an honest solo downgrade.
    // If team identity did not fully survive the salvage, fall back to solo
    // and say so loudly instead of flipping silently.
    if corrupt_team_mode {
        let users_survived: bool = fresh
            .query_row("SELECT 1 FROM users LIMIT 1", [], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
            .unwrap_or(false);
        if current_mode(&fresh) != "team" || !users_survived {
            let _ = fresh.execute("INSERT INTO config (key, value) VALUES ('mode', 'solo') ON CONFLICT(key) DO UPDATE SET value = 'solo'", []);
            eprintln!(
                "[cortex] auto_repair: WARNING -- corrupt DB was in TEAM MODE but team identity (config/users) could not be fully salvaged. The repaired DB starts in SOLO MODE; all team API keys are invalid. Re-run `cortex setup --team` to recreate team mode (it will re-assign salvaged rows to the new owner). The pre-repair file will be preserved as {}",
                db_path
                    .with_extension(format!("corrupt.{timestamp}"))
                    .display()
            );
        }
    }
    let integrity_ok = verify_integrity(&fresh).unwrap_or(false);
    drop(fresh);
    if !integrity_ok {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("[cortex] auto_repair: repaired DB failed integrity_check -- aborting");
        return Err(RepairError::RepairIntegrityFailed);
    }
    let corrupt_archive = db_path.with_extension(format!("corrupt.{timestamp}"));
    std::fs::rename(db_path, &corrupt_archive).map_err(RepairError::Io)?;
    std::fs::rename(&tmp_path, db_path).map_err(|e| {
        let _ = std::fs::rename(&corrupt_archive, db_path);
        RepairError::Io(e)
    })?;
    eprintln!(
        "[cortex] auto_repair: SUCCESS -- {} memories, {} decisions recovered. Corrupted DB archived at {}",
        memories_recovered,
        decisions_recovered,
        corrupt_archive.display()
    );
    Ok(RepairResult {
        memories_recovered,
        decisions_recovered,
        corrupt_db_path: corrupt_archive,
    })
}
