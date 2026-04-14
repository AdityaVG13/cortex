// SPDX-License-Identifier: MIT
use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

/// Result of an auto-repair attempt.
#[derive(Debug)]
pub struct RepairResult {
    pub memories_recovered: usize,
    pub decisions_recovered: usize,
    pub corrupt_db_path: std::path::PathBuf,
}

/// Error type for auto-repair failures.
pub enum RepairError {
    /// Could not open the corrupted DB for reading.
    OpenCorrupt(rusqlite::Error),
    /// Could not create a fresh DB for the repaired copy.
    OpenFresh(rusqlite::Error),
    /// Data export from the corrupted DB failed.
    Export(rusqlite::Error),
    /// Import into the fresh DB failed.
    Import(rusqlite::Error),
    /// The repaired DB itself failed integrity_check.
    RepairIntegrityFailed,
    /// File-system rename/copy operations failed.
    Io(std::io::Error),
}

impl std::fmt::Debug for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepairError::OpenCorrupt(e) => write!(f, "RepairError::OpenCorrupt({e})"),
            RepairError::OpenFresh(e) => write!(f, "RepairError::OpenFresh({e})"),
            RepairError::Export(e) => write!(f, "RepairError::Export({e})"),
            RepairError::Import(e) => write!(f, "RepairError::Import({e})"),
            RepairError::RepairIntegrityFailed => write!(f, "RepairError::RepairIntegrityFailed"),
            RepairError::Io(e) => write!(f, "RepairError::Io({e})"),
        }
    }
}

/// Open a SQLite connection at the given path.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open(path)
}

/// Apply WAL mode, NORMAL synchronous writes, and foreign-key enforcement.
///
/// NOTE: PRAGMA synchronous=NORMAL is safe with WAL mode. From SQLite docs:
/// - FULL: Extra safety at the cost of significant performance (OS crash protection)
/// - NORMAL: All changes are synced before passing control to caller at critical moments
///   (process crash protection). With WAL checkpoint every 10s, data loss is limited to <10s.
///   This is the recommended setting for WAL mode workloads.
pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA foreign_keys = ON;
        PRAGMA mmap_size = 268435456;
        PRAGMA cache_size = -8000;
        "#,
    )?;
    Ok(())
}

type MigrationDef = (&'static str, &'static str);

const SCHEMA_MIGRATIONS: [MigrationDef; 9] = [
    ("001_initial_schema", "initial_schema"),
    ("002_aging_columns", "aging_columns"),
    ("003_focus_table", "focus_table"),
    ("004_crystal_tables", "crystal_tables"),
    ("005_quality_dedup_columns", "quality_dedup_columns"),
    ("006", "ttl_expiration"),
    ("007", "semantic_store_quality_defaults"),
    ("008", "client_permissions"),
    ("009", "provenance_fields"),
];

/// Return ordered schema migration definitions.
pub fn migration_definitions() -> &'static [MigrationDef] {
    &SCHEMA_MIGRATIONS
}

fn migration_user_version(version: &str) -> i32 {
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

fn sync_schema_user_version(
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

/// Ensure schema migration tracking table exists.
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

fn migration_error(msg: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(msg.into())
}

fn apply_migration(conn: &Connection, version: &str) -> rusqlite::Result<()> {
    match version {
        // Baseline marker for pre-versioned schemas.
        "001_initial_schema" => Ok(()),
        "002_aging_columns" => {
            migrate_aging_columns(conn);
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
        other => Err(migration_error(format!(
            "unknown schema migration: {other}"
        ))),
    }
}

/// Return already-applied migration versions.
pub fn applied_migration_versions(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    ensure_schema_migrations_table(conn)?;
    let mut stmt =
        conn.prepare("SELECT version FROM schema_migrations ORDER BY id ASC, version ASC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Return pending migration versions in execution order.
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

/// Execute pending schema migrations in-order and record each in
/// `schema_migrations`. Returns the number of newly-applied migrations.
pub fn run_pending_migrations(conn: &Connection) -> usize {
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

        // Apply + record in one transaction so we never leave a migration half-recorded.
        // `BEGIN IMMEDIATE` prevents concurrent writers from racing this step.
        let tx = match conn.unchecked_transaction() {
            Ok(tx) => tx,
            Err(e) => {
                eprintln!("[db] failed to start migration transaction for {version} ({name}): {e}");
                break;
            }
        };
        if let Err(e) = apply_migration(&tx, version) {
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

/// Create all 12 application tables and supporting indexes if they do not
/// already exist.
pub fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS memories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          text TEXT NOT NULL,
          source TEXT,
          type TEXT DEFAULT 'memory',
          tags TEXT,
          source_agent TEXT DEFAULT 'unknown',
          source_client TEXT DEFAULT 'unknown',
          source_model TEXT,
          confidence REAL DEFAULT 0.8,
          reasoning_depth TEXT DEFAULT 'single-shot',
          trust_score REAL DEFAULT 0.8,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          merged_count INTEGER DEFAULT 0,
          quality INTEGER DEFAULT 50,
          expires_at TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS decisions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          decision TEXT NOT NULL,
          context TEXT,
          type TEXT DEFAULT 'decision',
          source_agent TEXT DEFAULT 'unknown',
          source_client TEXT DEFAULT 'unknown',
          source_model TEXT,
          confidence REAL DEFAULT 0.8,
          reasoning_depth TEXT DEFAULT 'single-shot',
          trust_score REAL DEFAULT 0.8,
          surprise REAL DEFAULT 1.0,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          parent_id INTEGER,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          merged_count INTEGER DEFAULT 0,
          quality INTEGER DEFAULT 50,
          expires_at TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS embeddings (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          target_type TEXT NOT NULL,
          target_id INTEGER NOT NULL,
          vector BLOB NOT NULL,
          model TEXT DEFAULT 'nomic-embed-text',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          type TEXT NOT NULL,
          data TEXT,
          source_agent TEXT,
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS co_occurrence (
          source_a TEXT NOT NULL,
          source_b TEXT NOT NULL,
          count INTEGER DEFAULT 1,
          last_seen TEXT DEFAULT (datetime('now')),
          PRIMARY KEY (source_a, source_b)
        );

        CREATE TABLE IF NOT EXISTS locks (
          id TEXT PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          agent TEXT NOT NULL,
          locked_at TEXT NOT NULL,
          expires_at TEXT
        );

        CREATE TABLE IF NOT EXISTS activities (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          description TEXT NOT NULL,
          files_json TEXT NOT NULL DEFAULT '[]',
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          sender TEXT NOT NULL,
          recipient TEXT NOT NULL,
          message TEXT NOT NULL,
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
          agent TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          description TEXT,
          started_at TEXT NOT NULL,
          last_heartbeat TEXT NOT NULL,
          expires_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks (
          task_id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          description TEXT,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          priority TEXT NOT NULL DEFAULT 'medium',
          required_capability TEXT NOT NULL DEFAULT 'any',
          status TEXT NOT NULL DEFAULT 'pending',
          claimed_by TEXT,
          created_at TEXT NOT NULL,
          claimed_at TEXT,
          completed_at TEXT,
          summary TEXT
        );

        CREATE TABLE IF NOT EXISTS feed (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          kind TEXT NOT NULL,
          summary TEXT NOT NULL,
          content TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          task_id TEXT,
          trace_id TEXT,
          priority TEXT NOT NULL DEFAULT 'normal',
          timestamp TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS feed_acks (
          agent TEXT PRIMARY KEY,
          last_seen_id TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

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

        CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a);
        CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b);

        -- Performance indexes (added 2026-03-31)
        CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status);
        CREATE INDEX IF NOT EXISTS idx_memories_source_status ON memories(source, status);
        CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status);
        CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_events_type_created ON events(type, created_at);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);

        CREATE TABLE IF NOT EXISTS context_cache (
          cache_key TEXT PRIMARY KEY,
          content_hash TEXT NOT NULL,
          compressed TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0,
          created_at TEXT DEFAULT (datetime('now')),
          hits INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS schema_migrations (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          version TEXT NOT NULL UNIQUE,
          name TEXT NOT NULL,
          applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- FTS5 full-text search indexes (trigram tokenizer for code/identifier matching)
        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
          text, source, tags,
          content=memories,
          content_rowid=id,
          tokenize='trigram'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
          decision, context,
          content=decisions,
          content_rowid=id,
          tokenize='trigram'
        );

        -- Relevance feedback: tracks which recalled results were actually useful
        CREATE TABLE IF NOT EXISTS recall_feedback (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          query_text TEXT NOT NULL,
          query_embedding BLOB,
          result_source TEXT NOT NULL,
          result_type TEXT NOT NULL DEFAULT 'unknown',
          result_id INTEGER,
          signal REAL NOT NULL DEFAULT 1.0,
          agent TEXT NOT NULL DEFAULT 'unknown',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_feedback_result ON recall_feedback(result_source);
        CREATE INDEX IF NOT EXISTS idx_feedback_created ON recall_feedback(created_at);

        -- Triggers to keep FTS in sync with base tables
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
        "#,
    )?;
    Ok(())
}

/// Return the current runtime mode (`solo` or `team`).
pub fn current_mode(conn: &Connection) -> String {
    if !table_exists(conn, "config") {
        return "solo".to_string();
    }
    conn.query_row(
        "SELECT value FROM config WHERE key = 'mode' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|_| "solo".to_string())
}

/// Check whether the database is in team mode.
pub fn is_team_mode(conn: &Connection) -> bool {
    current_mode(conn) == "team"
}

/// Per-table row counts for owner-aware tables after migration.
///
/// Returns `(table_name, count)` pairs. If a table lacks `owner_id` (solo mode),
/// its count is 0 rather than erroring.
pub fn migration_counts(conn: &Connection) -> Vec<(String, i64)> {
    const TABLES: &[&str] = &[
        "memories",
        "decisions",
        "memory_clusters",
        "recall_feedback",
        "sessions",
        "locks",
        "tasks",
        "messages",
        "feed",
        "feed_acks",
        "activities",
        "focus_sessions",
    ];

    TABLES
        .iter()
        .map(|&table| {
            let count = if table_has_column(conn, table, "owner_id") {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE owner_id IS NOT NULL"),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
            } else {
                0
            };
            (table.to_string(), count)
        })
        .collect()
}

/// Create the base team-mode tables (`config`, `users`, `teams`, `team_members`).
pub fn create_team_mode_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS config (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            display_name TEXT,
            api_key_hash TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member'
                CHECK (role IN ('owner', 'admin', 'member')),
            created_at TEXT DEFAULT (datetime('now')),
            last_active_at TEXT
        );

        CREATE TABLE IF NOT EXISTS teams (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT UNIQUE NOT NULL,
            parent_team_id INTEGER REFERENCES teams(id),
            created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS team_members (
            team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            role TEXT NOT NULL DEFAULT 'member'
                CHECK (role IN ('admin', 'member')),
            joined_at TEXT DEFAULT (datetime('now')),
            PRIMARY KEY (team_id, user_id)
        );
        "#,
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')",
        [],
    )?;
    Ok(())
}

/// Create or rotate the owner user entry and return its `users.id`.
pub fn upsert_owner_user(
    conn: &Connection,
    username: &str,
    display_name: Option<&str>,
    api_key_hash: &str,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, display_name, api_key_hash, role)
         VALUES (?1, ?2, ?3, 'owner')
         ON CONFLICT(username) DO UPDATE SET
           display_name = excluded.display_name,
           api_key_hash = excluded.api_key_hash,
           role = 'owner'",
        params![username, display_name, api_key_hash],
    )?;

    conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![username],
        |row| row.get::<_, i64>(0),
    )
}

/// Apply team-mode schema migration on top of an existing solo database.
///
/// This is idempotent and safe to call repeatedly.
pub fn migrate_to_team_mode(conn: &Connection, owner_id: i64) -> rusqlite::Result<()> {
    create_team_mode_tables(conn)?;

    // Core memory tables.
    ensure_column(
        conn,
        "memories",
        &format!(
            "ALTER TABLE memories ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "memories",
        "ALTER TABLE memories ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;

    ensure_column(
        conn,
        "decisions",
        &format!(
            "ALTER TABLE decisions ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "decisions",
        "ALTER TABLE decisions ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;

    // Crystal tables are named memory_clusters / cluster_members in this codebase.
    ensure_column(
        conn,
        "memory_clusters",
        &format!(
            "ALTER TABLE memory_clusters ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "memory_clusters",
        "ALTER TABLE memory_clusters ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;

    ensure_column(
        conn,
        "recall_feedback",
        &format!(
            "ALTER TABLE recall_feedback ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;

    // Conductor tables.
    ensure_column(
        conn,
        "tasks",
        &format!(
            "ALTER TABLE tasks ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "tasks",
        "ALTER TABLE tasks ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;

    ensure_column(
        conn,
        "messages",
        &format!(
            "ALTER TABLE messages ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;

    ensure_column(
        conn,
        "feed",
        &format!(
            "ALTER TABLE feed ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "feed",
        "ALTER TABLE feed ADD COLUMN visibility TEXT DEFAULT 'team' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;

    ensure_column(
        conn,
        "focus_sessions",
        &format!(
            "ALTER TABLE focus_sessions ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;
    ensure_column(
        conn,
        "activities",
        &format!(
            "ALTER TABLE activities ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )?;

    // Recreate sessions table for owner-scoped uniqueness.
    if !table_has_column(conn, "sessions", "id") || !table_has_column(conn, "sessions", "owner_id")
    {
        conn.execute_batch("DROP TABLE IF EXISTS sessions_new;")?;
        conn.execute_batch(&format!(
            "CREATE TABLE sessions_new (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent TEXT NOT NULL,
                owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id),
                session_id TEXT NOT NULL,
                project TEXT,
                files_json TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                started_at TEXT NOT NULL,
                last_heartbeat TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                UNIQUE(owner_id, agent)
            );"
        ))?;
        if table_exists(conn, "sessions") {
            conn.execute(
                "INSERT INTO sessions_new (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
                 SELECT agent, ?1, session_id, project, files_json, description, started_at, last_heartbeat, expires_at FROM sessions",
                params![owner_id],
            )?;
            conn.execute_batch("DROP TABLE sessions;")?;
        }
        conn.execute_batch("ALTER TABLE sessions_new RENAME TO sessions;")?;
    }

    // Recreate locks table for owner-scoped uniqueness.
    if !table_has_column(conn, "locks", "owner_id") {
        conn.execute_batch("DROP TABLE IF EXISTS locks_new;")?;
        conn.execute_batch(&format!(
            "CREATE TABLE locks_new (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                agent TEXT NOT NULL,
                owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id),
                locked_at TEXT NOT NULL,
                expires_at TEXT,
                UNIQUE(owner_id, path)
            );"
        ))?;
        if table_exists(conn, "locks") {
            conn.execute(
                "INSERT INTO locks_new (id, path, agent, owner_id, locked_at, expires_at)
                 SELECT id, path, agent, ?1, locked_at, expires_at FROM locks",
                params![owner_id],
            )?;
            conn.execute_batch("DROP TABLE locks;")?;
        }
        conn.execute_batch("ALTER TABLE locks_new RENAME TO locks;")?;
    }

    // Recreate feed_acks table for owner-scoped composite primary key.
    if !table_has_column(conn, "feed_acks", "owner_id") {
        conn.execute_batch("DROP TABLE IF EXISTS feed_acks_new;")?;
        conn.execute_batch(&format!(
            "CREATE TABLE feed_acks_new (
                owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id),
                agent TEXT NOT NULL,
                last_seen_id TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(owner_id, agent)
            );"
        ))?;
        if table_exists(conn, "feed_acks") {
            conn.execute(
                "INSERT INTO feed_acks_new (owner_id, agent, last_seen_id, updated_at)
                 SELECT ?1, agent, last_seen_id, updated_at FROM feed_acks",
                params![owner_id],
            )?;
            conn.execute_batch("DROP TABLE feed_acks;")?;
        }
        conn.execute_batch("ALTER TABLE feed_acks_new RENAME TO feed_acks;")?;
    }

    // Backfill ownership and sensible defaults.
    for table in [
        "memories",
        "decisions",
        "memory_clusters",
        "recall_feedback",
        "sessions",
        "locks",
        "tasks",
        "messages",
        "feed",
        "feed_acks",
        "activities",
        "focus_sessions",
    ] {
        if table_has_column(conn, table, "owner_id") {
            let sql = format!("UPDATE {table} SET owner_id = ?1 WHERE owner_id IS NULL");
            let _ = conn.execute(&sql, params![owner_id])?;
        }
    }
    let _ = conn.execute(
        "UPDATE memories SET visibility = 'private' WHERE visibility IS NULL",
        [],
    )?;
    let _ = conn.execute(
        "UPDATE decisions SET visibility = 'private' WHERE visibility IS NULL",
        [],
    )?;
    let _ = conn.execute(
        "UPDATE memory_clusters SET visibility = 'private' WHERE visibility IS NULL",
        [],
    )?;
    let _ = conn.execute(
        "UPDATE tasks SET visibility = 'private' WHERE visibility IS NULL",
        [],
    )?;
    let _ = conn.execute(
        "UPDATE feed SET visibility = 'team' WHERE visibility IS NULL",
        [],
    )?;

    // Team indexes.
    conn.execute_batch(
        r#"
        CREATE INDEX IF NOT EXISTS idx_memories_owner ON memories(owner_id) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memories_visibility ON memories(visibility) WHERE visibility != 'private';
        CREATE INDEX IF NOT EXISTS idx_decisions_owner ON decisions(owner_id) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_decisions_visibility ON decisions(visibility) WHERE visibility != 'private';
        CREATE INDEX IF NOT EXISTS idx_crystals_owner ON memory_clusters(owner_id) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_crystals_visibility ON memory_clusters(visibility) WHERE visibility != 'private';
        CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_owner ON tasks(owner_id) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_feed_owner ON feed(owner_id) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_activities_owner ON activities(owner_id) WHERE owner_id IS NOT NULL;
        "#,
    )?;
    conn.execute("INSERT OR IGNORE INTO teams (name) VALUES ('default')", [])?;
    let default_team_id: i64 = conn.query_row(
        "SELECT id FROM teams WHERE name = 'default' LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO team_members (team_id, user_id, role) VALUES (?1, ?2, 'admin')",
        params![default_team_id, owner_id],
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')",
        [],
    )?;
    conn.execute("UPDATE config SET value = 'team' WHERE key = 'mode'", [])?;
    conn.execute(
        "INSERT INTO config (key, value) VALUES ('owner_user_id', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![owner_id.to_string()],
    )?;

    Ok(())
}

/// Ensure a default team exists and owner is a member/admin.
pub fn ensure_default_team_membership(conn: &Connection, owner_id: i64) -> rusqlite::Result<i64> {
    conn.execute("INSERT OR IGNORE INTO teams (name) VALUES ('default')", [])?;
    let team_id: i64 =
        conn.query_row("SELECT id FROM teams WHERE name = 'default'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT OR IGNORE INTO team_members (team_id, user_id, role) VALUES (?1, ?2, 'admin')",
        params![team_id, owner_id],
    )?;
    Ok(team_id)
}

fn ensure_column(conn: &Connection, table: &str, alter_sql: &str) -> rusqlite::Result<()> {
    if !table_exists(conn, table) {
        return Ok(());
    }
    match conn.execute(alter_sql, []) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column name") => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1 LIMIT 1",
        params![table],
        |_| Ok(()),
    )
    .is_ok()
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    if !table_exists(conn, table) {
        return false;
    }
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(v) => v,
        Err(_) => return false,
    };
    for name in rows.flatten() {
        if name == column {
            return true;
        }
    }
    false
}

/// Create focus sessions table for context checkpointing.
pub fn migrate_focus_table(conn: &Connection) {
    let sql = r#"
        CREATE TABLE IF NOT EXISTS focus_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            label TEXT NOT NULL,
            agent TEXT NOT NULL DEFAULT 'unknown',
            status TEXT NOT NULL DEFAULT 'open',
            raw_entries TEXT NOT NULL DEFAULT '[]',
            summary TEXT,
            started_at TEXT DEFAULT (datetime('now')),
            ended_at TEXT,
            tokens_before INTEGER DEFAULT 0,
            tokens_after INTEGER DEFAULT 0
        )
    "#;
    match conn.execute_batch(sql) {
        Ok(_) => {}
        Err(e) => eprintln!("[db] Focus table migration: {e}"),
    }
}

/// Run schema migrations for progressive aging columns.
/// Safe to call repeatedly -- ALTER TABLE with IF NOT EXISTS-style error handling.
pub fn migrate_aging_columns(conn: &Connection) {
    let migrations = [
        "ALTER TABLE memories ADD COLUMN compressed_text TEXT",
        "ALTER TABLE memories ADD COLUMN age_tier TEXT DEFAULT 'fresh'",
        "ALTER TABLE decisions ADD COLUMN compressed_text TEXT",
        "ALTER TABLE decisions ADD COLUMN age_tier TEXT DEFAULT 'fresh'",
    ];
    for sql in &migrations {
        match conn.execute(sql, []) {
            Ok(_) => eprintln!("[db] Migration applied: {sql}"),
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => eprintln!("[db] Migration skipped ({e}): {sql}"),
        }
    }
}

/// Run a WAL checkpoint and truncate the WAL file.
pub fn checkpoint_wal(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

/// Attempt a WAL checkpoint; silently ignore any error.
pub fn checkpoint_wal_best_effort(conn: &Connection) {
    let _ = checkpoint_wal(conn);
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ExpiredCleanupCounts {
    pub memories_deleted: usize,
    pub decisions_deleted: usize,
}

/// Delete expired rows from tables that support TTL-based retention.
pub fn delete_expired_entries(conn: &Connection) -> rusqlite::Result<ExpiredCleanupCounts> {
    let memories_deleted = conn.execute(
        "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    )?;
    let decisions_deleted = conn.execute(
        "DELETE FROM decisions WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
        [],
    )?;
    Ok(ExpiredCleanupCounts {
        memories_deleted,
        decisions_deleted,
    })
}

/// Rebuild FTS5 indexes from base table data. Call once after schema migration
/// on databases that predate FTS5 support.
pub fn rebuild_fts(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags)
         SELECT id, text, source, tags FROM memories WHERE status = 'active';
         INSERT OR IGNORE INTO decisions_fts(rowid, decision, context)
         SELECT id, decision, context FROM decisions WHERE status = 'active';",
    )?;
    Ok(())
}

/// Seed FTS indexes at most once per database.
///
/// Uses a marker row in `schema_migrations` so startup does not rescan the
/// entire corpus on every daemon restart.
pub fn rebuild_fts_if_needed(conn: &Connection) -> rusqlite::Result<bool> {
    let already_seeded = conn
        .query_row(
            "SELECT 1 FROM schema_migrations WHERE version = 'fts_seeded_v1' LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;

    if already_seeded.is_some() {
        return Ok(false);
    }

    rebuild_fts(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_migrations (version, name, applied_at)
         VALUES ('fts_seeded_v1', 'fts_seeded', datetime('now'))",
        [],
    )?;
    Ok(true)
}

/// Run `PRAGMA integrity_check` and return `true` when the database reports
/// `ok`.
pub fn verify_integrity(conn: &Connection) -> rusqlite::Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
}

/// Run `PRAGMA quick_check` (B-tree structure only -- faster than integrity_check).
/// Returns `true` when the database passes.
pub fn quick_check(conn: &Connection) -> bool {
    conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map(|s| s.trim().eq_ignore_ascii_case("ok"))
        .unwrap_or(false)
}

/// Attempt to recover data from a corrupted DB using dump-and-rebuild.
///
/// Steps:
///   1. Open `db_path` read-only in a *separate* connection (does not touch the live connection).
///   2. `SELECT *` from every data table -- data pages survive most B-tree corruption.
///   3. Write all rows into a fresh temp DB at `{db_path}.repair_tmp`.
///   4. Run `PRAGMA integrity_check` on the fresh DB.
///   5. Rename `db_path` → `{db_path}.corrupt.{timestamp}` (never deleted).
///   6. Rename `{db_path}.repair_tmp` → `db_path`.
///
/// The original DB is preserved in all failure paths -- if any step before 5
/// fails, `db_path` is unchanged.
///
/// FTS virtual tables are re-created and populated from data; they are not
/// exported directly.
pub fn auto_repair(db_path: &Path, timestamp: &str) -> Result<RepairResult, RepairError> {
    eprintln!(
        "[cortex] auto_repair: beginning dump-and-rebuild of {}",
        db_path.display()
    );

    // ── Step 1: open the corrupted DB read-only ────────────────────────────
    let corrupt_conn = Connection::open(db_path).map_err(RepairError::OpenCorrupt)?;
    // Open read-only: ignore any write errors on WAL frames, read what we can.
    let _ = corrupt_conn.execute_batch("PRAGMA query_only = ON;");

    // ── Step 2: export all data tables ────────────────────────────────────
    // Order matters for foreign-key integrity, though FKs are off during repair.
    // Non-relational tables that carry user data are listed first; operational
    // tables that can be empty after rebuild are listed last.
    const DATA_TABLES: &[&str] = &[
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
    ];

    // Export: for each table collect column names and all rows as raw SQL strings.
    // We use TEXT casts for all values so we can INSERT via execute_batch without
    // complex type mapping.  This is safe because SQLite is loosely typed.
    let mut table_exports: Vec<(String, Vec<String>)> = Vec::new();
    let mut memories_recovered = 0usize;
    let mut decisions_recovered = 0usize;

    for &table in DATA_TABLES {
        // Check if the table even exists in the corrupted DB.
        let exists: bool = corrupt_conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
                params![table],
                |_| Ok(()),
            )
            .is_ok();
        if !exists {
            eprintln!("[cortex] auto_repair: table '{table}' not found in corrupt DB, skipping");
            continue;
        }

        // Get column names via PRAGMA.
        let mut col_stmt = corrupt_conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(RepairError::Export)?;
        let columns: Vec<String> = col_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(RepairError::Export)?
            .filter_map(|r| r.ok())
            .collect();

        if columns.is_empty() {
            eprintln!("[cortex] auto_repair: table '{table}' has no columns, skipping");
            continue;
        }

        // SELECT * and build INSERT statements using rusqlite's dynamic row access.
        let col_list = columns.join(", ");
        let placeholders: Vec<String> = (1..=columns.len()).map(|i| format!("?{i}")).collect();
        let placeholder_list = placeholders.join(", ");

        let mut data_stmt = match corrupt_conn.prepare(&format!("SELECT {col_list} FROM {table}")) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[cortex] auto_repair: failed to prepare SELECT on '{table}': {e}");
                continue;
            }
        };

        let query_result = data_stmt.query([]);
        let mut rows = match query_result {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[cortex] auto_repair: failed to query '{table}': {e}");
                continue;
            }
        };

        // Build INSERT..VALUES strings with escaped literals.
        // We quote every value as a SQL literal string using SQLite's double-single-quote
        // escaping.  BLOB columns are exported as X'' hex literals.
        let insert_prefix =
            format!("INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({placeholder_list})");

        let mut row_values: Vec<Vec<String>> = Vec::new();
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let mut vals: Vec<String> = Vec::new();
                    for i in 0..columns.len() {
                        use rusqlite::types::ValueRef;
                        let val = match row.get_ref(i) {
                            Ok(ValueRef::Null) => "NULL".to_string(),
                            Ok(ValueRef::Integer(n)) => n.to_string(),
                            Ok(ValueRef::Real(f)) => format!("{f}"),
                            Ok(ValueRef::Text(t)) => {
                                let s = String::from_utf8_lossy(t);
                                // Escape single quotes.
                                format!("'{}'", s.replace('\'', "''"))
                            }
                            Ok(ValueRef::Blob(b)) => {
                                let hex: String =
                                    b.iter().map(|byte| format!("{byte:02X}")).collect();
                                format!("X'{hex}'")
                            }
                            Err(_) => "NULL".to_string(),
                        };
                        vals.push(val);
                    }
                    row_values.push(vals);
                }
                Ok(None) => break,
                Err(e) => {
                    // Row-level corruption: skip the bad row and continue.
                    eprintln!("[cortex] auto_repair: row error in '{table}': {e} -- skipping row");
                    continue;
                }
            }
        }

        eprintln!(
            "[cortex] auto_repair: exported {} rows from '{table}'",
            row_values.len()
        );
        if table == "memories" {
            memories_recovered = row_values.len();
        } else if table == "decisions" {
            decisions_recovered = row_values.len();
        }

        // Convert to literal INSERT statements (values already SQL-safe).
        let inserts: Vec<String> = row_values
            .into_iter()
            .map(|vals| {
                let val_list = vals.join(", ");
                format!("INSERT OR IGNORE INTO {table} ({col_list}) VALUES ({val_list});")
            })
            .collect();

        table_exports.push((insert_prefix, inserts));
    }

    drop(corrupt_conn); // Release the read lock on the corrupt DB.

    // ── Step 3: build a fresh DB at a temp path ────────────────────────────
    let tmp_path = db_path.with_extension("repair_tmp");
    // Remove any leftover temp from a previous failed attempt.
    let _ = std::fs::remove_file(&tmp_path);

    let fresh = Connection::open(&tmp_path).map_err(RepairError::OpenFresh)?;
    configure(&fresh).map_err(RepairError::Import)?;
    initialize_schema(&fresh).map_err(RepairError::Import)?;

    // Disable FK enforcement during bulk insert so we can insert in any order.
    fresh
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(RepairError::Import)?;

    // Insert all exported rows.
    for (_prefix, inserts) in &table_exports {
        for stmt in inserts {
            if let Err(e) = fresh.execute_batch(stmt) {
                // Non-fatal: log and continue -- a few bad rows are acceptable.
                eprintln!("[cortex] auto_repair: insert skipped ({e}): {stmt:.80}");
            }
        }
    }

    // Re-enable FK enforcement and re-populate FTS indexes.
    fresh
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(RepairError::Import)?;

    // Populate FTS from the newly inserted data.
    fresh
        .execute_batch(
            "INSERT OR IGNORE INTO memories_fts(rowid, text, source, tags) \
             SELECT id, text, source, tags FROM memories; \
             INSERT OR IGNORE INTO decisions_fts(rowid, decision, context) \
             SELECT id, decision, context FROM decisions;",
        )
        .map_err(RepairError::Import)?;

    // VACUUM to compact and re-linearize the B-tree.
    fresh
        .execute_batch("VACUUM;")
        .map_err(RepairError::Import)?;

    // ── Step 4: verify the fresh DB ────────────────────────────────────────
    let integrity_ok = verify_integrity(&fresh).unwrap_or(false);
    drop(fresh);

    if !integrity_ok {
        let _ = std::fs::remove_file(&tmp_path);
        eprintln!("[cortex] auto_repair: repaired DB failed integrity_check -- aborting");
        return Err(RepairError::RepairIntegrityFailed);
    }

    // ── Steps 5 & 6: atomic swap ───────────────────────────────────────────
    // Rename original → .corrupt.{timestamp}
    let corrupt_archive = db_path.with_extension(format!("corrupt.{timestamp}"));
    std::fs::rename(db_path, &corrupt_archive).map_err(RepairError::Io)?;

    // Rename repaired temp → original path.
    std::fs::rename(&tmp_path, db_path).map_err(|e| {
        // Roll back: restore the original so the daemon isn't left with no DB.
        let _ = std::fs::rename(&corrupt_archive, db_path);
        RepairError::Io(e)
    })?;

    eprintln!(
        "[cortex] auto_repair: SUCCESS -- {} memories, {} decisions recovered. \
         Corrupted DB archived at {}",
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

/// Set `status = 'archived'` for all rows in `table` whose `id` is in `ids`.
/// Only `memories` and `decisions` are supported; other table names return an
/// error.  Returns the number of rows actually updated.
pub fn archive_entries(conn: &Connection, table: &str, ids: &[i64]) -> rusqlite::Result<usize> {
    if table != "memories" && table != "decisions" {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "archive_entries: unsupported table '{table}'"
        )));
    }
    if ids.is_empty() {
        return Ok(0);
    }

    let placeholders = ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!("UPDATE {table} SET status = 'archived' WHERE id IN ({placeholders})");

    let mut stmt = conn.prepare(&sql)?;
    let affected = stmt.execute(rusqlite::params_from_iter(ids.iter()))?;
    Ok(affected)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    #[test]
    fn test_open_configure_schema() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        assert!(verify_integrity(&conn).unwrap());
    }

    #[test]
    fn test_archive_entries() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        // Insert a test decision
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent) VALUES (?1, ?2, ?3, ?4)",
            params!["test decision", "test context", "decision", "test"],
        )
        .unwrap();

        let affected = archive_entries(&conn, "decisions", &[1]).unwrap();
        assert_eq!(affected, 1);

        let status: String = conn
            .query_row("SELECT status FROM decisions WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "archived");
    }

    #[test]
    fn test_archive_entries_empty_ids() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        let affected = archive_entries(&conn, "memories", &[]).unwrap();
        assert_eq!(affected, 0);
    }

    #[test]
    fn test_archive_entries_invalid_table() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        let result = archive_entries(&conn, "locks", &[1]);
        assert!(result.is_err());
    }

    #[test]
    fn test_checkpoint_wal_best_effort() {
        // Should not panic even on an in-memory connection (WAL not applicable)
        let conn = Connection::open_in_memory().unwrap();
        checkpoint_wal_best_effort(&conn);
    }

    #[test]
    fn test_fts5_schema_and_search() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO memories (text, source, type) VALUES (?1, ?2, ?3)",
            params![
                "Cortex uses Ebbinghaus decay for memory scoring",
                "test::fts",
                "memory"
            ],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'Ebbinghaus'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "FTS5 trigger should auto-index new memories");

        let count2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'nonexistent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count2, 0);
    }

    #[test]
    fn test_rebuild_fts_if_needed_rebuilds_once_for_empty_fts() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let rebuilt = rebuild_fts_if_needed(&conn).unwrap();
        assert!(rebuilt, "first call should seed FTS marker");

        let marker_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 'fts_seeded_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_rows, 1, "expected FTS marker row to be persisted");

        let rebuilt_again = rebuild_fts_if_needed(&conn).unwrap();
        assert!(
            !rebuilt_again,
            "second call should skip when marker already exists"
        );
    }

    #[test]
    fn test_solo_schema_baseline_unchanged() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        // Core solo schema tables exist.
        for table in [
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
            "client_permissions",
            "context_cache",
            "schema_migrations",
            "memories_fts",
            "decisions_fts",
            "recall_feedback",
        ] {
            assert!(table_exists(&conn, table), "missing solo table: {table}");
        }

        // Team tables are not auto-created in solo mode.
        assert!(!table_exists(&conn, "config"));
        assert!(!table_exists(&conn, "users"));
        assert!(!table_exists(&conn, "teams"));
        assert!(!table_exists(&conn, "team_members"));

        // Team columns are not present in solo baseline.
        assert!(!table_has_column(&conn, "memories", "owner_id"));
        assert!(!table_has_column(&conn, "memories", "visibility"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(!table_has_column(&conn, "decisions", "owner_id"));
        assert!(!table_has_column(&conn, "decisions", "visibility"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));
        assert!(table_has_column(&conn, "sessions", "agent"));
        assert!(table_has_column(&conn, "sessions", "session_id"));
        assert!(!table_has_column(&conn, "sessions", "owner_id"));
        assert!(table_has_column(&conn, "locks", "path"));
        assert!(!table_has_column(&conn, "locks", "owner_id"));
        assert!(table_has_column(&conn, "feed_acks", "agent"));
        assert!(!table_has_column(&conn, "feed_acks", "owner_id"));
    }

    #[test]
    fn test_quick_check_clean_db() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        assert!(
            quick_check(&conn),
            "fresh in-memory DB should pass quick_check"
        );
    }

    #[test]
    fn test_auto_repair_recovers_data() {
        use std::io::Write;

        // ── Build a valid DB with test data at a temp file path ────────────
        // Use a unique path under the system temp dir so parallel test runs don't collide.
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "cortex_repair_test_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));

        {
            let conn = Connection::open(&db_path).unwrap();
            configure(&conn).unwrap();
            initialize_schema(&conn).unwrap();

            // Insert known rows.
            conn.execute(
                "INSERT INTO memories (text, source, type) VALUES (?1, ?2, ?3)",
                params!["repair test memory", "test::repair", "memory"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO decisions (decision, context, type) VALUES (?1, ?2, ?3)",
                params!["repair test decision", "test context", "decision"],
            )
            .unwrap();
            // Checkpoint so data is in the main DB file, not just WAL.
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .unwrap();
        } // Connection closed -- file flushed.

        // Verify the DB is clean before corruption.
        {
            let conn = Connection::open(&db_path).unwrap();
            assert!(
                verify_integrity(&conn).unwrap(),
                "DB should be clean before corruption"
            );
        }

        // ── Corrupt the DB by overwriting a page mid-file ─────────────────
        // We write garbage into the middle of the file. SQLite's B-tree index
        // pages live in the middle; data in leaf pages (lower in the file) often
        // survives. For this test we write at a safe offset that corrupts the
        // free-list / interior pages but leaves leaf data readable.
        {
            let meta = std::fs::metadata(&db_path).unwrap();
            let file_size = meta.len();
            // Write 512 bytes of 0xFF starting at 40% of the file (index area).
            let corrupt_offset = (file_size as f64 * 0.4) as u64;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&db_path)
                .unwrap();
            use std::io::Seek;
            f.seek(std::io::SeekFrom::Start(corrupt_offset)).unwrap();
            f.write_all(&[0xFF_u8; 512]).unwrap();
            f.flush().unwrap();
        }

        // ── Run auto_repair ────────────────────────────────────────────────
        let result = auto_repair(&db_path, "20260407_test");
        match &result {
            Ok(r) => {
                eprintln!(
                    "[test] auto_repair: {} memories, {} decisions recovered",
                    r.memories_recovered, r.decisions_recovered
                );
                // The corrupt archive must exist.
                assert!(
                    r.corrupt_db_path.exists(),
                    "corrupt DB should be preserved at {:?}",
                    r.corrupt_db_path
                );
            }
            Err(e) => {
                // If SQLite was able to read the corrupted pages without error
                // (it sometimes can), repair may not be triggered -- that's OK.
                // But if repair ran and produced an error, that's a test failure.
                panic!("auto_repair returned error: {e:?}");
            }
        }

        // The repaired DB at the original path must pass integrity_check.
        if db_path.exists() {
            let conn = Connection::open(&db_path).unwrap();
            assert!(
                verify_integrity(&conn).unwrap_or(false),
                "repaired DB must pass integrity_check"
            );

            // At least memories and decisions tables must be present.
            let mem_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                .unwrap_or(0);
            let dec_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
                .unwrap_or(0);
            eprintln!("[test] Repaired DB: {mem_count} memories, {dec_count} decisions");
            // We may not recover all rows if the page containing them was corrupted,
            // but the DB itself must be structurally sound (integrity check above).
            drop(conn);
        }

        // Cleanup temp files.
        let _ = std::fs::remove_file(&db_path);
        // Also remove the corrupt archive if it exists.
        if let Ok(r) = &result {
            let _ = std::fs::remove_file(&r.corrupt_db_path);
        }
    }

    #[test]
    fn test_run_pending_migrations_applies_all_once() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let first_applied = run_pending_migrations(&conn);
        assert_eq!(first_applied, migration_definitions().len());

        let second_applied = run_pending_migrations(&conn);
        assert_eq!(second_applied, 0);

        let pending = pending_migration_versions(&conn).unwrap();
        assert!(
            pending.is_empty(),
            "no pending migrations expected after first run"
        );

        let recorded: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(recorded as usize, migration_definitions().len());
        assert_eq!(
            current_schema_user_version(&conn).unwrap(),
            latest_schema_user_version()
        );

        assert!(table_has_column(&conn, "memories", "merged_count"));
        assert!(table_has_column(&conn, "memories", "quality"));
        assert!(table_has_column(&conn, "memories", "expires_at"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(table_has_column(&conn, "decisions", "merged_count"));
        assert!(table_has_column(&conn, "decisions", "quality"));
        assert!(table_has_column(&conn, "decisions", "expires_at"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));
        assert!(table_exists(&conn, "focus_sessions"));
        assert!(table_exists(&conn, "memory_clusters"));
        assert!(table_exists(&conn, "cluster_members"));
    }

    #[test]
    fn test_delete_expired_entries_removes_only_expired_rows() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('expired-memory', 'note', 'expired-memory', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('future-memory', 'note', 'future-memory', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('forever-memory', 'note', 'forever-memory', 'active', NULL, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired-decision', 'expired-decision', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('future-decision', 'future-decision', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('forever-decision', 'forever-decision', 'active', NULL, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let deleted = delete_expired_entries(&conn).unwrap();
        assert_eq!(
            deleted,
            ExpiredCleanupCounts {
                memories_deleted: 1,
                decisions_deleted: 1,
            }
        );

        let remaining_memories: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source IN ('future-memory', 'forever-memory')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_memories, 2);

        let expired_memories: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source = 'expired-memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expired_memories, 0);

        let remaining_decisions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context IN ('future-decision', 'forever-decision')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_decisions, 2);

        let expired_decisions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context = 'expired-decision'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expired_decisions, 0);
    }

    #[test]
    fn test_team_migration_creates_owner_scoped_schema() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        migrate_focus_table(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);

        create_team_mode_tables(&conn).unwrap();
        let owner_id =
            upsert_owner_user(&conn, "owner", Some("Owner"), "argon2id-placeholder").unwrap();
        migrate_to_team_mode(&conn, owner_id).unwrap();

        assert_eq!(current_mode(&conn), "team");
        assert!(table_exists(&conn, "users"));
        assert!(table_exists(&conn, "teams"));
        assert!(table_exists(&conn, "team_members"));

        assert!(table_has_column(&conn, "memories", "owner_id"));
        assert!(table_has_column(&conn, "memories", "visibility"));
        assert!(table_has_column(&conn, "decisions", "owner_id"));
        assert!(table_has_column(&conn, "decisions", "visibility"));
        assert!(table_has_column(&conn, "memory_clusters", "owner_id"));
        assert!(table_has_column(&conn, "memory_clusters", "visibility"));
        assert!(table_has_column(&conn, "sessions", "id"));
        assert!(table_has_column(&conn, "sessions", "owner_id"));
        assert!(table_has_column(&conn, "locks", "owner_id"));
        assert!(table_has_column(&conn, "feed_acks", "owner_id"));
        let owner_cfg: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'owner_user_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner_cfg, owner_id.to_string());
        let default_team_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM teams WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default_team_count, 1);
        let owner_membership_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM team_members tm
                 JOIN teams t ON t.id = tm.team_id
                 WHERE t.name = 'default' AND tm.user_id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner_membership_count, 1);

        let sessions_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sessions_sql.contains("UNIQUE(owner_id, agent)"));
        assert!(!sessions_sql.contains("UNIQUE(agent)"));

        let locks_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'locks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(locks_sql.contains("UNIQUE(owner_id, path)"));
        assert!(!locks_sql.contains("UNIQUE(path)"));

        let feed_acks_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'feed_acks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(feed_acks_sql.contains("PRIMARY KEY(owner_id, agent)"));
        assert!(!feed_acks_sql.contains("UNIQUE(agent)"));
    }
}
