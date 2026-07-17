// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use rusqlite::Connection;
fn setup() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        r#"
            CREATE TABLE memories (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              text TEXT,
              source TEXT,
              source_agent TEXT,
              status TEXT DEFAULT 'active',
              created_at TEXT DEFAULT (datetime('now')),
              updated_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE decisions (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              decision TEXT,
              source_agent TEXT,
              status TEXT DEFAULT 'active',
              created_at TEXT DEFAULT (datetime('now')),
              updated_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE sessions (
              agent TEXT PRIMARY KEY,
              session_id TEXT NOT NULL,
              started_at TEXT NOT NULL,
              last_heartbeat TEXT NOT NULL,
              expires_at TEXT NOT NULL
            );
            "#,
    )
    .unwrap();
    conn
}
fn seed_session(conn: &Connection, agent: &str, session_id: &str, started_at: &str) {
    conn.execute(
        "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, ?3, ?3)",
        params![agent, session_id, started_at],
    )
    .unwrap();
}
fn seed_memory(conn: &Connection, agent: &str, text: &str, created_at: &str, status: &str) {
    conn.execute(
        "INSERT INTO memories(text, source_agent, status, created_at)
             VALUES (?1, ?2, ?3, ?4)",
        params![text, agent, status, created_at],
    )
    .unwrap();
}
fn seed_decision(conn: &Connection, agent: &str, decision: &str, created_at: &str) {
    conn.execute(
        "INSERT INTO decisions(decision, source_agent, status, created_at)
             VALUES (?1, ?2, 'active', ?3)",
        params![decision, agent, created_at],
    )
    .unwrap();
}
#[test]
fn unknown_session_returns_zero_stats() {
    let conn = setup();
    let stats = rollback_session_by_id(&conn, "nonexistent", false).unwrap();
    assert_eq!(stats.memories_affected, 0);
    assert_eq!(stats.decisions_affected, 0);
    assert_eq!(stats.agent, "");
    assert!(!stats.applied);
    assert!(!stats.already_rolled_back);
}
#[test]
fn dry_run_counts_without_writing() {
    let conn = setup();
    seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
    seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");
    seed_memory(&conn, "claude", "m2", "2026-04-24T00:10:00Z", "active");
    seed_decision(&conn, "claude", "d1", "2026-04-24T00:06:00Z");
    let stats = rollback_session_by_id(&conn, "sess-1", false).unwrap();
    assert_eq!(stats.agent, "claude");
    assert_eq!(stats.memories_affected, 2);
    assert_eq!(stats.decisions_affected, 1);
    assert!(!stats.applied);
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 2);
}
#[test]
fn apply_flips_statuses_and_excludes_older_rows() {
    let conn = setup();
    seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
    seed_memory(&conn, "claude", "old", "2026-04-23T23:59:00Z", "active");
    seed_memory(
        &conn,
        "codex",
        "other-agent",
        "2026-04-24T00:05:00Z",
        "active",
    );
    seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");
    seed_memory(&conn, "claude", "m2", "2026-04-24T00:10:00Z", "active");
    seed_memory(
        &conn,
        "claude",
        "pre-rolled",
        "2026-04-24T00:07:00Z",
        ROLLED_BACK_STATUS,
    );
    seed_decision(&conn, "claude", "d1", "2026-04-24T00:06:00Z");
    let stats = rollback_session_by_id(&conn, "sess-1", true).unwrap();
    assert_eq!(
        stats.memories_affected, 2,
        "only the 2 active in-session memories"
    );
    assert_eq!(stats.decisions_affected, 1);
    assert!(stats.applied);
    let old_status: String = conn
        .query_row("SELECT status FROM memories WHERE text = 'old'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(old_status, "active");
    let other_status: String = conn
        .query_row(
            "SELECT status FROM memories WHERE text = 'other-agent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(other_status, "active");
    let rolled: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE source_agent = 'claude' AND status = ?1",
            params![ROLLED_BACK_STATUS],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rolled, 3); // pre-rolled + 2 freshly rolled
}
#[test]
fn idempotent_second_apply() {
    let conn = setup();
    seed_session(&conn, "claude", "sess-1", "2026-04-24T00:00:00Z");
    seed_memory(&conn, "claude", "m1", "2026-04-24T00:05:00Z", "active");
    let first = rollback_session_by_id(&conn, "sess-1", true).unwrap();
    assert_eq!(first.memories_affected, 1);
    assert!(first.applied);
    let second = rollback_session_by_id(&conn, "sess-1", true).unwrap();
    assert_eq!(second.memories_affected, 0);
    assert_eq!(second.decisions_affected, 0);
    assert!(second.applied);
    assert!(second.already_rolled_back);
}
#[test]
fn multi_row_session_chooses_most_recent() {
    let conn = setup();
    conn.execute(
        "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES ('old-agent', 'sess-1', '2026-04-24T00:00:00Z',
                     '2026-04-24T00:00:00Z', '2026-04-25T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions(agent, session_id, started_at, last_heartbeat, expires_at)
             VALUES ('new-agent', 'sess-1', '2026-04-24T01:00:00Z',
                     '2026-04-24T01:00:00Z', '2026-04-25T01:00:00Z')",
        [],
    )
    .unwrap();
    seed_memory(&conn, "new-agent", "m1", "2026-04-24T01:30:00Z", "active");
    seed_memory(&conn, "old-agent", "o1", "2026-04-24T00:30:00Z", "active");
    let stats = rollback_session_by_id(&conn, "sess-1", false).unwrap();
    assert_eq!(stats.agent, "new-agent");
    assert_eq!(stats.memories_affected, 1);
}
