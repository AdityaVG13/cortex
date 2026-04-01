use std::path::Path;

use rusqlite::Connection;

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
    let result: String =
        conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    Ok(result.trim().eq_ignore_ascii_case("ok"))
}

/// Set `status = 'archived'` for all rows in `table` whose `id` is in `ids`.
/// Only `memories` and `decisions` are supported; other table names return an
/// error.  Returns the number of rows actually updated.
pub fn archive_entries(
    conn: &Connection,
    table: &str,
    ids: &[i64],
) -> rusqlite::Result<usize> {
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

    let sql = format!(
        "UPDATE {table} SET status = 'archived' WHERE id IN ({placeholders})"
    );

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
            .query_row(
                "SELECT status FROM decisions WHERE id = 1",
                [],
                |row| row.get(0),
            )
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
            params!["Cortex uses Ebbinghaus decay for memory scoring", "test::fts", "memory"],
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
}
