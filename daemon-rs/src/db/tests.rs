// SPDX-License-Identifier: MIT
//! Schema and migration integrity only. See Info/testing-philosophy.md.

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn open_configure_schema_roundtrip() {
        let conn = Connection::open_in_memory().expect("open db");
        configure(&conn).expect("configure");
        initialize_schema(&conn).expect("schema");
        run_pending_migrations(&conn);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('decisions', 'memories', 'events')",
                [],
                |row| row.get(0),
            )
            .expect("count tables");
        assert_eq!(tables, 3);
    }

    #[test]
    fn run_pending_migrations_is_idempotent() {
        let path = std::env::temp_dir().join(format!(
            "cortex-db-migrate-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let conn = Connection::open(&path).expect("open file db");
        configure(&conn).expect("configure");
        initialize_schema(&conn).expect("schema");
        run_pending_migrations(&conn);
        let first: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
            .unwrap_or(0);
        run_pending_migrations(&conn);
        let second: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
            .unwrap_or(0);
        assert_eq!(first, second);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn delete_expired_entries_removes_only_expired_rows() {
        let conn = Connection::open_in_memory().expect("open db");
        configure(&conn).expect("configure");
        initialize_schema(&conn).expect("schema");
        run_pending_migrations(&conn);
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired', 'ctx', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('active', 'ctx', 'active', datetime('now', '+1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let removed = delete_expired_entries(&conn).expect("delete expired");
        assert_eq!(removed.decisions_deleted, 1);
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn rebuild_fts_if_needed_rebuilds_empty_index() {
        let conn = Connection::open_in_memory().expect("open db");
        configure(&conn).expect("configure");
        initialize_schema(&conn).expect("schema");
        run_pending_migrations(&conn);
        conn.execute(
            "INSERT INTO decisions (decision, context, status, created_at, updated_at)
             VALUES ('fts smoke target', 'ctx', 'active', datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        rebuild_fts_if_needed(&conn).expect("fts seed");
        let hits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'smoke'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        assert!(hits >= 1);
    }
}
