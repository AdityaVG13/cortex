// SPDX-License-Identifier: MIT
use std::path::Path;

use rusqlite::{params, Connection};

/// Open a SQLite connection at the given path.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open(path)
}

/// Apply WAL mode, FULL synchronous writes, and foreign-key enforcement.
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
          confidence REAL DEFAULT 0.8,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS decisions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          decision TEXT NOT NULL,
          context TEXT,
          type TEXT DEFAULT 'decision',
          source_agent TEXT DEFAULT 'unknown',
          confidence REAL DEFAULT 0.8,
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

fn table_exists(conn: &Connection, table: &str) -> bool {
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

/// Run `PRAGMA integrity_check` and return `true` when the database reports
/// `ok`.
pub fn verify_integrity(conn: &Connection) -> rusqlite::Result<bool> {
    let result: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
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
            "context_cache",
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
        assert!(!table_has_column(&conn, "decisions", "owner_id"));
        assert!(!table_has_column(&conn, "decisions", "visibility"));
        assert!(table_has_column(&conn, "sessions", "agent"));
        assert!(table_has_column(&conn, "sessions", "session_id"));
        assert!(!table_has_column(&conn, "sessions", "owner_id"));
        assert!(table_has_column(&conn, "locks", "path"));
        assert!(!table_has_column(&conn, "locks", "owner_id"));
        assert!(table_has_column(&conn, "feed_acks", "agent"));
        assert!(!table_has_column(&conn, "feed_acks", "owner_id"));
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

