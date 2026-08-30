use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::hash::{DefaultHasher, Hash, Hasher};

const SUMMARY_MAX_CHARS: usize = 160;

pub fn migrate_history_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS traces (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          session_id TEXT,
          agent TEXT NOT NULL DEFAULT 'unknown',
          role TEXT NOT NULL DEFAULT 'store',
          text TEXT NOT NULL,
          content_hash TEXT NOT NULL,
          source_uri TEXT,
          target_type TEXT,
          target_id INTEGER,
          owner_id INTEGER,
          visibility TEXT,
          t_event TEXT,
          t_ingest TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_traces_target ON traces(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_traces_ingest ON traces(t_ingest);
        CREATE TABLE IF NOT EXISTS versions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          op TEXT NOT NULL,
          summary TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'active'
            CHECK (status IN ('active', 'orphaned')),
          trace_id INTEGER,
          target_type TEXT,
          target_id INTEGER,
          owner_id INTEGER,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_versions_status ON versions(status);
        CREATE TABLE IF NOT EXISTS head_state (
          scope TEXT PRIMARY KEY,
          head_id INTEGER NOT NULL
        );
        "#,
    )?;
    for sql in [
        "ALTER TABLE decisions ADD COLUMN version_id INTEGER",
        "ALTER TABLE memories ADD COLUMN version_id INTEGER",
    ] {
        match conn.execute(sql, []) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("duplicate column") => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn content_hash(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn summary_of(op: &str, text: &str) -> String {
    let trimmed: String = text.chars().take(SUMMARY_MAX_CHARS).collect();
    format!("{op}: {trimmed}")
}

/// SQL fragment: row is visible under the current HEAD.
/// `prefix` is the table alias including the trailing dot, or "" for bare columns.
pub fn live_version_clause(prefix: &str) -> String {
    format!("({prefix}version_id IS NULL OR {prefix}version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))")
}

/// Record one observed store write: trace row + commit descriptor + version stamp.
/// Returns the version id. Failures are reported but must never fail the store.
pub fn record_store_write(
    conn: &Connection,
    agent: &str,
    text: &str,
    action: &str,
    target_type: &str,
    target_id: Option<i64>,
    owner_id: Option<i64>,
) -> Option<i64> {
    let insert_trace = conn.execute(
        "INSERT INTO traces (agent, role, text, content_hash, target_type, target_id, owner_id) \
         VALUES (?1, 'store', ?2, ?3, ?4, ?5, ?6)",
        params![
            agent,
            text,
            content_hash(text),
            target_type,
            target_id,
            owner_id
        ],
    );
    let trace_id = match insert_trace {
        Ok(_) => conn.last_insert_rowid(),
        Err(err) => {
            eprintln!("[traces] Warning: failed to append trace: {err}");
            return None;
        }
    };
    let version = conn.execute(
        "INSERT INTO versions (op, summary, status, trace_id, target_type, target_id, owner_id) \
         VALUES (?1, ?2, 'active', ?3, ?4, ?5, ?6)",
        params![
            action,
            summary_of(action, text),
            trace_id,
            target_type,
            target_id,
            owner_id
        ],
    );
    let version_id = match version {
        Ok(_) => conn.last_insert_rowid(),
        Err(err) => {
            eprintln!("[traces] Warning: failed to record version: {err}");
            return None;
        }
    };
    if let Some(id) = target_id {
        let table = if target_type == "memory" {
            "memories"
        } else {
            "decisions"
        };
        let _ = conn.execute(
            &format!("UPDATE {table} SET version_id = ?1 WHERE id = ?2"),
            params![version_id, id],
        );
    }
    let _ = conn.execute(
        "INSERT INTO head_state (scope, head_id) VALUES ('default', ?1) \
         ON CONFLICT(scope) DO UPDATE SET head_id = excluded.head_id",
        params![version_id],
    );
    Some(version_id)
}

pub fn current_head(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT head_id FROM head_state WHERE scope = 'default'",
        [],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

pub fn list_versions(conn: &Connection, limit: usize) -> Vec<Value> {
    let head = current_head(conn);
    let mut stmt = match conn.prepare(
        "SELECT id, op, summary, status, target_type, target_id, created_at \
         FROM versions ORDER BY id DESC LIMIT ?1",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };
    stmt.query_map(params![limit as i64], |row| {
        let id: i64 = row.get(0)?;
        Ok(json!({
            "id": id,
            "op": row.get::<_, String>(1)?,
            "summary": row.get::<_, String>(2)?,
            "status": row.get::<_, String>(3)?,
            "targetType": row.get::<_, Option<String>>(4)?,
            "targetId": row.get::<_, Option<i64>>(5)?,
            "createdAt": row.get::<_, String>(6)?,
            "head": head == Some(id),
        }))
    })
    .map(|rows| rows.filter_map(Result::ok).collect())
    .unwrap_or_default()
}

/// Moves HEAD to `to` and orphans newer active versions.
/// Returns the orphaned count and new HEAD.
pub fn rollback_to(conn: &Connection, to: i64) -> Result<(usize, i64), String> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM versions WHERE id = ?1",
            params![to],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if exists.is_none() {
        return Err(format!("Unknown version id: {to}"));
    }
    let orphaned = conn
        .execute(
            "UPDATE versions SET status = 'orphaned' WHERE id > ?1 AND status = 'active'",
            params![to],
        )
        .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO head_state (scope, head_id) VALUES ('default', ?1) \
         ON CONFLICT(scope) DO UPDATE SET head_id = excluded.head_id",
        params![to],
    )
    .map_err(|e| e.to_string())?;
    Ok((orphaned, to))
}
