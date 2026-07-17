// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use crate::compaction::{prune_expired_entries, prune_old_events, purge_benchmark_artifacts};
    use rusqlite::Connection;
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }
    #[test]
    fn prune_old_events_removes_stale_rows() {
        let conn = setup();
        conn.execute("INSERT INTO events (type, data, created_at) VALUES ('boot', '{}', datetime('now', '-40 days'))", [])
            .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, created_at) VALUES ('boot', '{}', datetime('now'))",
            [],
        )
        .unwrap();
        let removed = prune_old_events(&conn);
        assert_eq!(removed, 1);
    }
    #[test]
    fn prune_expired_entries_removes_expired_decisions() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('old', 'ctx', 'active', datetime('now', '-1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('fresh', 'ctx', 'active', datetime('now', '+1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let removed = prune_expired_entries(&conn);
        assert_eq!(removed, 1);
    }
    #[test]
    fn purge_benchmark_artifacts_removes_benchmark_rows() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, created_at, updated_at)
             VALUES ('bench row', 'ctx', 'amb-cortex::smoke', 'active', datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let result = purge_benchmark_artifacts(&conn);
        assert!(result.total_deleted() >= 1);
    }
}
