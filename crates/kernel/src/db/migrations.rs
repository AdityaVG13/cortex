use super::*;
use rusqlite::{Connection, TransactionBehavior, params};
use std::collections::HashSet;
pub const SCHEMA_MIGRATIONS: [MigrationDef; 24] = [
    ("001_initial_schema", "initial_schema"),
    ("002_aging_columns", "aging_columns"),
    ("003_focus_table", "focus_table"),
    ("004_crystal_tables", "crystal_tables"),
    ("005_quality_dedup_columns", "quality_dedup_columns"),
    ("006", "ttl_expiration"),
    ("007", "semantic_store_quality_defaults"),
    ("008", "client_permissions"),
    ("009", "provenance_fields"),
    ("010", "decision_conflict_records"),
    ("011", "agent_feedback_telemetry"),
    ("012", "fts_tokenizer_porter_unicode61"),
    ("013", "embeddings_model_lookup_indexes"),
    ("014", "temporal_semantics_fields"),
    ("015", "boot_audits"),
    ("016", "retention_classes"),
    ("017", "recall_hot_path_indexes"),
    ("018", "owner_visibility_columns"),
    ("019", "history_tables"),
    ("020", "entity_tables"),
    ("021", "temporal_validity_windows"),
    ("022_clock_anchors", "clock_anchors"),
    ("023_authoritative_records", "authoritative_records"),
    ("024_boot_capsule_indexes", "boot_capsule_indexes"),
];
pub fn migration_definitions() -> &'static [MigrationDef] {
    &SCHEMA_MIGRATIONS
}
pub fn migration_user_version(version: &str) -> i32 {
    version
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>()
        .parse::<i32>()
        .unwrap_or(0)
}
#[cfg_attr(not(test), allow(dead_code))]
pub fn latest_schema_user_version() -> i32 {
    migration_definitions()
        .iter()
        .map(|(version, _)| migration_user_version(version))
        .max()
        .unwrap_or(0)
}
pub fn current_schema_user_version(conn: &Connection) -> rusqlite::Result<i32> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}
pub fn set_schema_user_version(conn: &Connection, version: i32) -> rusqlite::Result<()> {
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}
pub fn sync_schema_user_version(
    conn: &Connection,
    applied_versions: &HashSet<String>,
) -> rusqlite::Result<i32> {
    let version = applied_versions
        .iter()
        .map(|entry| migration_user_version(entry))
        .max()
        .unwrap_or(0);
    set_schema_user_version(conn, version)?;
    Ok(version)
}
pub fn ensure_schema_migrations_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations (id INTEGER PRIMARY KEY AUTOINCREMENT, version TEXT NOT NULL UNIQUE, name TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')));")?;
    Ok(())
}
pub fn migration_error(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(msg.into())
}

fn add_twin_columns(conn: &Connection, cols: &[(&str, &str)]) -> rusqlite::Result<()> {
    for (column, decl) in cols {
        for table in ["memories", "decisions"] {
            ensure_column(
                conn,
                table,
                &format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"),
            )?;
        }
    }
    Ok(())
}

fn fill_nulls(conn: &Connection, sql: &str) {
    let _ = conn.execute(sql, []);
}

fn fill_twins(conn: &Connection, sqls: &[&str]) {
    for table in ["memories", "decisions"] {
        for sql in sqls {
            fill_nulls(conn, &sql.replace("{table}", table));
        }
    }
}

fn ensure_quality_columns(conn: &Connection) -> rusqlite::Result<()> {
    add_twin_columns(
        conn,
        &[
            ("merged_count", "INTEGER DEFAULT 0"),
            ("quality", "INTEGER DEFAULT 50"),
        ],
    )?;
    fill_twins(
        conn,
        &[
            "UPDATE {table} SET merged_count = 0 WHERE merged_count IS NULL",
            "UPDATE {table} SET quality = 50 WHERE quality IS NULL",
        ],
    );
    Ok(())
}

fn require_columns(conn: &Connection, cols: &[(&str, &str)], err: &str) -> rusqlite::Result<()> {
    if cols
        .iter()
        .all(|(table, column)| table_has_column(conn, table, column))
    {
        Ok(())
    } else {
        Err(migration_error(err))
    }
}

fn require_tables(conn: &Connection, names: &[&str], err: &str) -> rusqlite::Result<()> {
    if names.iter().all(|name| table_exists(conn, name)) {
        Ok(())
    } else {
        Err(migration_error(err))
    }
}

fn ensure_temporal_columns(conn: &Connection) -> rusqlite::Result<()> {
    add_twin_columns(
        conn,
        &[
            ("observed_at", "TEXT"),
            ("valid_from", "TEXT"),
            ("valid_until", "TEXT"),
        ],
    )
}
mod apply;
pub use apply::apply_migration_with_logging;
pub fn applied_migration_versions(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    ensure_schema_migrations_table(conn)?;
    let mut stmt =
        conn.prepare("SELECT version FROM schema_migrations ORDER BY id ASC, version ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
pub fn pending_migration_versions(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let applied: HashSet<String> = applied_migration_versions(conn)?.into_iter().collect();
    let mut pending = Vec::new();
    for (version, _) in migration_definitions() {
        if !applied.contains(*version) {
            pending.push((*version).to_string());
        }
    }
    Ok(pending)
}
pub fn run_pending_migrations(conn: &mut Connection) -> usize {
    run_pending_migrations_with_logging(conn, true)
}
pub fn run_pending_migrations_quiet(conn: &mut Connection) -> usize {
    run_pending_migrations_with_logging(conn, false)
}
pub fn run_pending_migrations_with_logging(conn: &mut Connection, log_success: bool) -> usize {
    if let Err(e) = ensure_schema_migrations_table(conn) {
        eprintln!("[db] schema migration setup failed: {e}");
        return 0;
    }
    let mut applied_set: HashSet<String> = match applied_migration_versions(conn) {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            eprintln!("[db] failed to read applied migrations: {e}");
            return 0;
        }
    };
    let mut applied_count = 0usize;
    for (version, name) in migration_definitions() {
        if applied_set.contains(*version) {
            continue;
        }
        let tx = match conn.transaction_with_behavior(TransactionBehavior::Deferred) {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("[db] failed to start migration transaction for {version} ({name}): {e}");
                break;
            }
        };
        if let Err(e) = apply_migration_with_logging(&tx, version, log_success) {
            eprintln!("[db] migration {version} ({name}) failed: {e}");
            drop(tx);
            break;
        }
        if let Err(e) = tx.execute(
            "INSERT INTO schema_migrations (version, name) VALUES (?1, ?2)",
            params![version, name],
        ) {
            eprintln!("[db] failed to record migration {version} ({name}): {e}");
            drop(tx);
            break;
        }
        if let Err(e) = tx.commit() {
            eprintln!("[db] failed to commit migration {version} ({name}): {e}");
            break;
        }
        applied_set.insert((*version).to_string());
        applied_count += 1;
    }
    if let Err(e) = sync_schema_user_version(conn, &applied_set) {
        eprintln!("[db] failed to update PRAGMA user_version: {e}");
    }
    applied_count
}
