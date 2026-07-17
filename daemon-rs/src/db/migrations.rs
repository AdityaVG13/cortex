use super::*;
use rusqlite::{params, Connection};
use std::collections::HashSet;
pub(crate) const SCHEMA_MIGRATIONS: [MigrationDef; 17] = [
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
];
pub fn migration_definitions() -> &'static [MigrationDef] {
    &SCHEMA_MIGRATIONS
}
pub(crate) fn migration_user_version(version: &str) -> i32 {
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
pub(crate) fn sync_schema_user_version(
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
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            version TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        "#,
    )?;
    Ok(())
}
pub(crate) fn migration_error(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(msg.into())
}
pub(crate) fn apply_migration_with_logging(
    conn: &Connection,
    version: &str,
    log_success: bool,
) -> rusqlite::Result<()> {
    match version {
        "001_initial_schema" => Ok(()),
        "002_aging_columns" => {
            migrate_aging_columns_with_logging(conn, log_success);
            if table_has_column(conn, "memories", "compressed_text")
                && table_has_column(conn, "memories", "age_tier")
                && table_has_column(conn, "decisions", "compressed_text")
                && table_has_column(conn, "decisions", "age_tier")
            {
                Ok(())
            } else {
                Err(migration_error(
                    "aging migration did not create expected columns",
                ))
            }
        }
        "003_focus_table" => {
            migrate_focus_table(conn);
            if table_exists(conn, "focus_sessions") {
                Ok(())
            } else {
                Err(migration_error(
                    "focus table migration did not create focus_sessions",
                ))
            }
        }
        "004_crystal_tables" => {
            crate::crystallize::migrate_crystal_tables(conn);
            if table_exists(conn, "memory_clusters") && table_exists(conn, "cluster_members") {
                Ok(())
            } else {
                Err(migration_error(
                    "crystal migration did not create memory_clusters/cluster_members",
                ))
            }
        }
        "005_quality_dedup_columns" => {
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN merged_count INTEGER DEFAULT 0",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN quality INTEGER DEFAULT 50",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN merged_count INTEGER DEFAULT 0",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN quality INTEGER DEFAULT 50",
            )?;
            let _ = conn.execute(
                "UPDATE memories SET merged_count = 0 WHERE merged_count IS NULL",
                [],
            );
            let _ = conn.execute("UPDATE memories SET quality = 50 WHERE quality IS NULL", []);
            let _ = conn.execute(
                "UPDATE decisions SET merged_count = 0 WHERE merged_count IS NULL",
                [],
            );
            let _ = conn.execute(
                "UPDATE decisions SET quality = 50 WHERE quality IS NULL",
                [],
            );
            Ok(())
        }
        "006" => {
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN expires_at TEXT",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN expires_at TEXT",
            )?;
            Ok(())
        }
        "007" => {
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN merged_count INTEGER DEFAULT 0",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN quality INTEGER DEFAULT 50",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN merged_count INTEGER DEFAULT 0",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN quality INTEGER DEFAULT 50",
            )?;
            let _ = conn.execute(
                "UPDATE memories SET merged_count = 0 WHERE merged_count IS NULL",
                [],
            );
            let _ = conn.execute("UPDATE memories SET quality = 50 WHERE quality IS NULL", []);
            let _ = conn.execute(
                "UPDATE decisions SET merged_count = 0 WHERE merged_count IS NULL",
                [],
            );
            let _ = conn.execute(
                "UPDATE decisions SET quality = 50 WHERE quality IS NULL",
                [],
            );
            Ok(())
        }
        "008" => {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS client_permissions (
                  owner_id INTEGER NOT NULL DEFAULT 0,
                  client_id TEXT NOT NULL,
                  permission TEXT NOT NULL,
                  scope TEXT NOT NULL DEFAULT '*',
                  granted_by TEXT NOT NULL DEFAULT 'system',
                  granted_at TEXT NOT NULL DEFAULT (datetime('now')),
                  PRIMARY KEY (owner_id, client_id, permission, scope)
                );
                CREATE INDEX IF NOT EXISTS idx_client_permissions_client
                  ON client_permissions(owner_id, client_id);
                "#,
            )?;
            Ok(())
        }
        "009" => {
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN source_client TEXT DEFAULT 'unknown'",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN source_model TEXT",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN reasoning_depth TEXT DEFAULT 'single-shot'",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN trust_score REAL DEFAULT 0.8",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN source_client TEXT DEFAULT 'unknown'",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN source_model TEXT",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN reasoning_depth TEXT DEFAULT 'single-shot'",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN trust_score REAL DEFAULT 0.8",
            )?;
            let _ = conn.execute(
                "UPDATE memories
                 SET source_client = COALESCE(NULLIF(lower(source_agent), ''), 'unknown')
                 WHERE source_client IS NULL OR source_client = ''",
                [],
            );
            let _ = conn.execute(
                "UPDATE memories SET trust_score = COALESCE(confidence, 0.8)
                 WHERE trust_score IS NULL",
                [],
            );
            let _ = conn.execute(
                "UPDATE decisions
                 SET source_client = COALESCE(NULLIF(lower(source_agent), ''), 'unknown')
                 WHERE source_client IS NULL OR source_client = ''",
                [],
            );
            let _ = conn.execute(
                "UPDATE decisions SET trust_score = COALESCE(confidence, 0.8)
                 WHERE trust_score IS NULL",
                [],
            );
            Ok(())
        }
        "010" => {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS decision_conflicts (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  source_decision_id INTEGER REFERENCES decisions(id) ON DELETE SET NULL,
                  target_decision_id INTEGER NOT NULL REFERENCES decisions(id) ON DELETE CASCADE,
                  classification TEXT NOT NULL
                    CHECK (classification IN ('AGREES', 'CONTRADICTS', 'REFINES', 'UNRELATED')),
                  similarity_jaccard REAL,
                  similarity_cosine REAL,
                  status TEXT NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'auto_resolved', 'user_resolved')),
                  resolution_strategy TEXT,
                  resolved_by TEXT,
                  resolved_at TEXT,
                  created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX IF NOT EXISTS idx_decision_conflicts_source
                  ON decision_conflicts(source_decision_id);
                CREATE INDEX IF NOT EXISTS idx_decision_conflicts_target
                  ON decision_conflicts(target_decision_id);
                CREATE INDEX IF NOT EXISTS idx_decision_conflicts_status_created
                  ON decision_conflicts(status, created_at);
                "#,
            )?;
            Ok(())
        }
        "011" => {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS agent_feedback (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  owner_id INTEGER NOT NULL DEFAULT 0,
                  agent TEXT NOT NULL,
                  task_class TEXT NOT NULL DEFAULT 'general',
                  outcome TEXT NOT NULL
                    CHECK (outcome IN ('success', 'partial', 'failure')),
                  outcome_score REAL NOT NULL,
                  quality_score REAL NOT NULL DEFAULT 0.7,
                  latency_ms INTEGER,
                  retries INTEGER,
                  tokens_used INTEGER,
                  memory_sources_json TEXT NOT NULL DEFAULT '[]',
                  notes TEXT,
                  created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX IF NOT EXISTS idx_agent_feedback_agent_created
                  ON agent_feedback(owner_id, agent, created_at);
                CREATE INDEX IF NOT EXISTS idx_agent_feedback_task_created
                  ON agent_feedback(owner_id, task_class, created_at);
                "#,
            )?;
            Ok(())
        }
        "012" => {
            conn.execute_batch(
r#"
                DROP TRIGGER IF EXISTS memories_fts_ai;
                DROP TRIGGER IF EXISTS memories_fts_ad;
                DROP TRIGGER IF EXISTS memories_fts_au;
                DROP TRIGGER IF EXISTS decisions_fts_ai;
                DROP TRIGGER IF EXISTS decisions_fts_ad;
                DROP TRIGGER IF EXISTS decisions_fts_au;
                DROP TABLE IF EXISTS memories_fts;
                DROP TABLE IF EXISTS decisions_fts;
                CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                  text, source, tags,
                  content=memories,
                  content_rowid=id,
                  tokenize='porter unicode61'
                );
                CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
                  decision, context,
                  content=decisions,
                  content_rowid=id,
                  tokenize='porter unicode61'
                );
                CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
                  INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
                END;
                CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
                  INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
                END;
                CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
                  INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
                  INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
                END;
                CREATE TRIGGER IF NOT EXISTS decisions_fts_ai AFTER INSERT ON decisions BEGIN
                  INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
                END;
                CREATE TRIGGER IF NOT EXISTS decisions_fts_ad AFTER DELETE ON decisions BEGIN
                  INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
                END;
                CREATE TRIGGER IF NOT EXISTS decisions_fts_au AFTER UPDATE ON decisions BEGIN
                  INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
                  INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
                END;
                "#
,)?;
            rebuild_fts(conn)?;
            Ok(())
        }
        "013" => {
            conn.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_embeddings_model_norm
                  ON embeddings(LOWER(COALESCE(model, '')));
                CREATE INDEX IF NOT EXISTS idx_embeddings_target_model_norm
                  ON embeddings(target_type, target_id, LOWER(COALESCE(model, '')));
                "#,
            )?;
            Ok(())
        }
        "015" => {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS boot_audits (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  agent TEXT NOT NULL,
                  profile TEXT NOT NULL,
                  budget_tokens INTEGER NOT NULL,
                  token_estimate INTEGER NOT NULL,
                  token_savings INTEGER NOT NULL DEFAULT 0,
                  capsules_count INTEGER NOT NULL DEFAULT 0,
                  capsules_json TEXT,
                  latency_ms INTEGER,
                  created_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE INDEX IF NOT EXISTS idx_boot_audits_created_at
                  ON boot_audits(created_at);
                CREATE INDEX IF NOT EXISTS idx_boot_audits_agent_created
                  ON boot_audits(agent, created_at);
                "#,
            )?;
            Ok(())
        }
        "014" => {
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN observed_at TEXT",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN valid_from TEXT",
            )?;
            ensure_column(
                conn,
                "memories",
                "ALTER TABLE memories ADD COLUMN valid_until TEXT",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN observed_at TEXT",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN valid_from TEXT",
            )?;
            ensure_column(
                conn,
                "decisions",
                "ALTER TABLE decisions ADD COLUMN valid_until TEXT",
            )?;
            Ok(())
        }
        "016" => {
            ensure_column(conn,"memories",
"ALTER TABLE memories ADD COLUMN retention_class TEXT NOT NULL DEFAULT 'operational'")?;
            ensure_column(conn,"decisions",
"ALTER TABLE decisions ADD COLUMN retention_class TEXT NOT NULL DEFAULT 'operational'")?;
            let _ = conn.execute(
                "UPDATE memories SET retention_class = 'operational'
                 WHERE retention_class IS NULL
                    OR retention_class = ''
                    OR retention_class NOT IN ('durable', 'operational', 'audit', 'ephemeral')",
                [],
            );
            let _ = conn.execute(
                "UPDATE decisions SET retention_class = 'operational'
                 WHERE retention_class IS NULL
                    OR retention_class = ''
                    OR retention_class NOT IN ('durable', 'operational', 'audit', 'ephemeral')",
                [],
            );
            conn.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_memories_retention_class
                  ON memories(retention_class);
                CREATE INDEX IF NOT EXISTS idx_decisions_retention_class
                  ON decisions(retention_class);
                "#,
            )?;
            Ok(())
        }
        "017" => {
            conn.execute_batch(
                r#"
                CREATE INDEX IF NOT EXISTS idx_memories_active_source_recent
                  ON memories(source, COALESCE(last_accessed, created_at) DESC)
                  WHERE status = 'active';
                CREATE INDEX IF NOT EXISTS idx_decisions_context_status
                  ON decisions(context, status);
                CREATE INDEX IF NOT EXISTS idx_decisions_active_context_recent
                  ON decisions(context, COALESCE(last_accessed, created_at) DESC)
                  WHERE status = 'active';
                CREATE INDEX IF NOT EXISTS idx_embeddings_model_type_target_norm
                  ON embeddings(LOWER(COALESCE(model, '')), target_type, target_id);
                "#,
            )?;
            Ok(())
        }
        other => Err(migration_error(format!(
            "unknown schema migration: {other}"
        ))),
    }
}
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
pub fn run_pending_migrations(conn: &Connection) -> usize {
    run_pending_migrations_with_logging(conn, true)
}
pub fn run_pending_migrations_quiet(conn: &Connection) -> usize {
    run_pending_migrations_with_logging(conn, false)
}
pub(crate) fn run_pending_migrations_with_logging(conn: &Connection, log_success: bool) -> usize {
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
        let tx = match conn.unchecked_transaction() {
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
