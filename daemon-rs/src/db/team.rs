// SPDX-License-Identifier: MIT
use rusqlite::{params, Connection};
pub fn current_mode(conn: &Connection) -> String {
    if !table_exists(conn, "config") {
        return "solo".to_string();
    }
    conn.query_row("SELECT value FROM config WHERE key = 'mode' LIMIT 1", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| "solo".to_string())
}
pub fn is_team_mode(conn: &Connection) -> bool {
    current_mode(conn) == "team"
}
pub fn migration_counts(conn: &Connection) -> Vec<(String, i64)> {
    pub(crate) const TABLES: &[&str] = &[
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
                conn.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE owner_id IS NOT NULL"), [], |row| row.get::<_, i64>(0))
                    .unwrap_or(0)
            } else {
                0
            };
            (table.to_string(), count)
        })
        .collect()
}
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
    conn.execute("INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')", [])?;
    Ok(())
}
pub fn upsert_owner_user(conn: &Connection, username: &str, display_name: Option<&str>, api_key_hash: &str) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO users (username, display_name, api_key_hash, role)
         VALUES (?1, ?2, ?3, 'owner')
         ON CONFLICT(username) DO UPDATE SET
           display_name = excluded.display_name,
           api_key_hash = excluded.api_key_hash,
           role = 'owner'",
        params![username, display_name, api_key_hash],
    )?;
    conn.query_row("SELECT id FROM users WHERE username = ?1", params![username], |row| row.get::<_, i64>(0))
}
pub fn migrate_to_team_mode(conn: &Connection, owner_id: i64) -> rusqlite::Result<()> {
    create_team_mode_tables(conn)?;
    ensure_column(conn, "memories", &format!("ALTER TABLE memories ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"))?;
    ensure_column(
        conn,
        "memories",
        "ALTER TABLE memories ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;
    ensure_column(conn, "decisions", &format!("ALTER TABLE decisions ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"))?;
    ensure_column(
        conn,
        "decisions",
        "ALTER TABLE decisions ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;
    ensure_column(
        conn,
        "memory_clusters",
        &format!("ALTER TABLE memory_clusters ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"),
    )?;
    ensure_column(
        conn,
        "memory_clusters",
        "ALTER TABLE memory_clusters ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;
    ensure_column(
        conn,
        "recall_feedback",
        &format!("ALTER TABLE recall_feedback ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"),
    )?;
    ensure_column(conn, "tasks", &format!("ALTER TABLE tasks ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"))?;
    ensure_column(
        conn,
        "tasks",
        "ALTER TABLE tasks ADD COLUMN visibility TEXT DEFAULT 'private' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;
    ensure_column(conn, "messages", &format!("ALTER TABLE messages ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"))?;
    ensure_column(conn, "feed", &format!("ALTER TABLE feed ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"))?;
    ensure_column(
        conn,
        "feed",
        "ALTER TABLE feed ADD COLUMN visibility TEXT DEFAULT 'team' CHECK (visibility IN ('private', 'team', 'shared'))",
    )?;
    ensure_column(
        conn,
        "focus_sessions",
        &format!("ALTER TABLE focus_sessions ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"),
    )?;
    ensure_column(
        conn,
        "activities",
        &format!("ALTER TABLE activities ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"),
    )?;
    if !table_has_column(conn, "sessions", "id") || !table_has_column(conn, "sessions", "owner_id") {
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
    let _ = conn.execute("UPDATE memories SET visibility = 'private' WHERE visibility IS NULL", [])?;
    let _ = conn.execute("UPDATE decisions SET visibility = 'private' WHERE visibility IS NULL", [])?;
    let _ = conn.execute("UPDATE memory_clusters SET visibility = 'private' WHERE visibility IS NULL", [])?;
    let _ = conn.execute("UPDATE tasks SET visibility = 'private' WHERE visibility IS NULL", [])?;
    let _ = conn.execute("UPDATE feed SET visibility = 'team' WHERE visibility IS NULL", [])?;
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
        CREATE INDEX IF NOT EXISTS idx_sessions_owner_heartbeat
          ON sessions(owner_id, last_heartbeat) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_activities_owner_timestamp
          ON activities(owner_id, timestamp) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_messages_owner_recipient_timestamp
          ON messages(owner_id, recipient, timestamp) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_feed_owner_timestamp
          ON feed(owner_id, timestamp) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_tasks_owner_status_created
          ON tasks(owner_id, status, created_at) WHERE owner_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_locks_owner_expires
          ON locks(owner_id, expires_at) WHERE owner_id IS NOT NULL;
        "#,
    )?;
    conn.execute("INSERT OR IGNORE INTO teams (name) VALUES ('default')", [])?;
    let default_team_id: i64 = conn.query_row("SELECT id FROM teams WHERE name = 'default' LIMIT 1", [], |row| row.get(0))?;
    conn.execute(
        "INSERT OR IGNORE INTO team_members (team_id, user_id, role) VALUES (?1, ?2, 'admin')",
        params![default_team_id, owner_id],
    )?;
    conn.execute("INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')", [])?;
    conn.execute("UPDATE config SET value = 'team' WHERE key = 'mode'", [])?;
    conn.execute(
        "INSERT INTO config (key, value) VALUES ('owner_user_id', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![owner_id.to_string()],
    )?;
    Ok(())
}
pub fn ensure_default_team_membership(conn: &Connection, owner_id: i64) -> rusqlite::Result<i64> {
    conn.execute("INSERT OR IGNORE INTO teams (name) VALUES ('default')", [])?;
    let team_id: i64 = conn.query_row("SELECT id FROM teams WHERE name = 'default'", [], |row| row.get(0))?;
    conn.execute("INSERT OR IGNORE INTO team_members (team_id, user_id, role) VALUES (?1, ?2, 'admin')", params![team_id, owner_id])?;
    Ok(team_id)
}
pub(crate) fn ensure_column(conn: &Connection, table: &str, alter_sql: &str) -> rusqlite::Result<()> {
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
    conn.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name = ?1 LIMIT 1", params![table], |_| Ok(()))
        .is_ok()
}
pub(crate) fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
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
