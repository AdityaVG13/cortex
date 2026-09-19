use rusqlite::{Connection, params};
pub fn current_mode(conn: &Connection) -> String {
    if !table_exists(conn, "config") {
        return "solo".to_string();
    }
    match conn.query_row(
        "SELECT value FROM config WHERE key = 'mode' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(mode) => mode,
        // Missing key is the schema default (create_team_mode_tables seeds
        // solo). A readable 'solo' after a team→solo downgrade must stay
        // solo even if leftover `users` rows exist.
        Err(rusqlite::Error::QueryReturnedNoRows) => "solo".to_string(),
        // Unreadable config is not a solo verdict. `users` exists only in
        // team-mode DBs; infer the same way repair does so boot ACL cannot
        // fail open while identity tables are still present.
        Err(_) => {
            if table_exists(conn, "users") {
                "team".to_string()
            } else {
                "solo".to_string()
            }
        }
    }
}
pub fn is_team_mode(conn: &Connection) -> bool {
    current_mode(conn) == "team"
}
pub fn migration_counts(conn: &Connection) -> Vec<(String, i64)> {
    pub const TABLES: &[&str] = &[
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
                super::count_or_zero(
                    conn,
                    &format!("SELECT COUNT(*) FROM {table} WHERE owner_id IS NOT NULL"),
                )
            } else {
                0
            };
            (table.to_string(), count)
        })
        .collect()
}
pub fn create_team_mode_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT UNIQUE NOT NULL, display_name TEXT, api_key_hash TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('owner', 'admin', 'member')), created_at TEXT DEFAULT (datetime('now')), last_active_at TEXT); CREATE TABLE IF NOT EXISTS teams (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL, parent_team_id INTEGER REFERENCES teams(id), created_at TEXT DEFAULT (datetime('now'))); CREATE TABLE IF NOT EXISTS team_members (team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE, user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE, role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')), joined_at TEXT DEFAULT (datetime('now')), PRIMARY KEY (team_id, user_id));")?;
    conn.execute(
        "INSERT OR IGNORE INTO config (key, value) VALUES ('mode', 'solo')",
        [],
    )?;
    Ok(())
}
pub fn upsert_owner_user(
    conn: &Connection,
    username: &str,
    display_name: Option<&str>,
    api_key_hash: &str,
) -> rusqlite::Result<i64> {
    conn.execute("INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, 'owner') ON CONFLICT(username) DO UPDATE SET display_name = excluded.display_name, api_key_hash = excluded.api_key_hash, role = 'owner'", params![username, display_name, api_key_hash])?;
    conn.query_row(
        "SELECT id FROM users WHERE username = ?1",
        params![username],
        |row| row.get::<_, i64>(0),
    )
}
mod migrate;
pub use migrate::migrate_to_team_mode;
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
pub fn ensure_column(conn: &Connection, table: &str, alter_sql: &str) -> rusqlite::Result<()> {
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
fn sqlite_ident_ok(name: &str) -> bool {
    let bytes = name.as_bytes();
    matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic() || *b == b'_')
        && bytes.len() <= 64
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

pub fn pragma_column_names(conn: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(Result::ok)
        .collect())
}

pub fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    if !sqlite_ident_ok(table) || !table_exists(conn, table) {
        return false;
    }
    pragma_column_names(conn, table)
        .map(|names| names.iter().any(|name| name == column))
        .unwrap_or(false)
}

/// `AND owner_id = N` when the table carries owner scoping and a caller is
/// known. A team caller with a missing owner, or a missing or unreadable
/// `owner_id` column, must not fall open: `AND 0` matches nothing instead of
/// every row. Solo with no caller stays unscoped.
pub fn owner_and_clause(conn: &Connection, table: &str, owner: Option<i64>) -> String {
    match owner {
        Some(id) if table_has_column(conn, table, "owner_id") => format!(" AND owner_id = {id}"),
        Some(_) => " AND 0".to_string(),
        None if is_team_mode(conn) => " AND 0".to_string(),
        None => String::new(),
    }
}
