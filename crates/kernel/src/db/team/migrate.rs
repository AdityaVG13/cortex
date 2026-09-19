use super::*;
use rusqlite::{Connection, params};

fn add_owner_column(conn: &Connection, table: &str, owner_id: i64) -> rusqlite::Result<()> {
    ensure_column(
        conn,
        table,
        &format!(
            "ALTER TABLE {table} ADD COLUMN owner_id INTEGER DEFAULT {owner_id} REFERENCES users(id)"
        ),
    )
}

fn add_visibility_column(conn: &Connection, table: &str, default: &str) -> rusqlite::Result<()> {
    ensure_column(
        conn,
        table,
        &format!(
            "ALTER TABLE {table} ADD COLUMN visibility TEXT DEFAULT '{default}' CHECK (visibility IN ('private', 'team', 'shared'))"
        ),
    )
}

fn rebuild_owned_table(
    conn: &Connection,
    table: &str,
    rebuild: bool,
    create_sql: &str,
    copy_sql: &str,
    owner_id: i64,
) -> rusqlite::Result<()> {
    if !rebuild {
        return Ok(());
    }
    conn.execute_batch(&format!("DROP TABLE IF EXISTS {table}_new;"))?;
    conn.execute_batch(create_sql)?;
    if table_exists(conn, table) {
        conn.execute(copy_sql, params![owner_id])?;
        conn.execute_batch(&format!("DROP TABLE {table};"))?;
    }
    conn.execute_batch(&format!("ALTER TABLE {table}_new RENAME TO {table};"))
}

pub fn migrate_to_team_mode(conn: &Connection, owner_id: i64) -> rusqlite::Result<()> {
    create_team_mode_tables(conn)?;
    for (table, visibility) in [
        ("memories", Some("private")),
        ("decisions", Some("private")),
        ("memory_clusters", Some("private")),
        ("recall_feedback", None),
        ("tasks", Some("private")),
        ("messages", None),
        ("feed", Some("team")),
        ("focus_sessions", None),
        ("activities", None),
    ] {
        add_owner_column(conn, table, owner_id)?;
        if let Some(default) = visibility {
            add_visibility_column(conn, table, default)?;
        }
    }
    rebuild_owned_table(
        conn,
        "sessions",
        !table_has_column(conn, "sessions", "id")
            || !table_has_column(conn, "sessions", "owner_id"),
        &format!(
            "CREATE TABLE sessions_new (id INTEGER PRIMARY KEY AUTOINCREMENT, agent TEXT NOT NULL, owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id), session_id TEXT NOT NULL, project TEXT, files_json TEXT NOT NULL DEFAULT '[]', description TEXT, started_at TEXT NOT NULL, last_heartbeat TEXT NOT NULL, expires_at TEXT NOT NULL, UNIQUE(owner_id, agent));"
        ),
        "INSERT INTO sessions_new (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) SELECT agent, ?1, session_id, project, files_json, description, started_at, last_heartbeat, expires_at FROM sessions",
        owner_id,
    )?;
    rebuild_owned_table(
        conn,
        "locks",
        !table_has_column(conn, "locks", "owner_id"),
        &format!(
            "CREATE TABLE locks_new (id TEXT PRIMARY KEY, path TEXT NOT NULL, agent TEXT NOT NULL, owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id), locked_at TEXT NOT NULL, expires_at TEXT, UNIQUE(owner_id, path));"
        ),
        "INSERT INTO locks_new (id, path, agent, owner_id, locked_at, expires_at) SELECT id, path, agent, ?1, locked_at, expires_at FROM locks",
        owner_id,
    )?;
    rebuild_owned_table(
        conn,
        "feed_acks",
        !table_has_column(conn, "feed_acks", "owner_id"),
        &format!(
            "CREATE TABLE feed_acks_new (owner_id INTEGER NOT NULL DEFAULT {owner_id} REFERENCES users(id), agent TEXT NOT NULL, last_seen_id TEXT NOT NULL, updated_at TEXT NOT NULL, PRIMARY KEY(owner_id, agent));"
        ),
        "INSERT INTO feed_acks_new (owner_id, agent, last_seen_id, updated_at) SELECT ?1, agent, last_seen_id, updated_at FROM feed_acks",
        owner_id,
    )?;
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
    for (table, default) in [
        ("memories", "private"),
        ("decisions", "private"),
        ("memory_clusters", "private"),
        ("tasks", "private"),
        ("feed", "team"),
    ] {
        let _ = conn.execute(
            &format!("UPDATE {table} SET visibility = '{default}' WHERE visibility IS NULL"),
            [],
        )?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_owner ON memories(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_memories_visibility ON memories(visibility) WHERE visibility != 'private'; CREATE INDEX IF NOT EXISTS idx_decisions_owner ON decisions(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_decisions_visibility ON decisions(visibility) WHERE visibility != 'private'; CREATE INDEX IF NOT EXISTS idx_crystals_owner ON memory_clusters(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_crystals_visibility ON memory_clusters(visibility) WHERE visibility != 'private'; CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_members(user_id); CREATE INDEX IF NOT EXISTS idx_tasks_owner ON tasks(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_feed_owner ON feed(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_activities_owner ON activities(owner_id) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_sessions_owner_heartbeat ON sessions(owner_id, last_heartbeat) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_activities_owner_timestamp ON activities(owner_id, timestamp) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_messages_owner_recipient_timestamp ON messages(owner_id, recipient, timestamp) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_feed_owner_timestamp ON feed(owner_id, timestamp) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_tasks_owner_status_created ON tasks(owner_id, status, created_at) WHERE owner_id IS NOT NULL; CREATE INDEX IF NOT EXISTS idx_locks_owner_expires ON locks(owner_id, expires_at) WHERE owner_id IS NOT NULL;")?;
    super::ensure_default_team_membership(conn, owner_id)?;
    conn.execute(
        "INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')",
        [],
    )?;
    conn.execute("UPDATE config SET value = 'team' WHERE key = 'mode'", [])?;
    conn.execute("INSERT INTO config (key, value) VALUES ('owner_user_id', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![owner_id.to_string()])?;
    Ok(())
}
