// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use rusqlite::Connection;
fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    conn
}
fn insert_entry(conn: &Connection, id: &str, agent: &str, timestamp: &str) {
    insert_feed_entry(
        conn,
        &FeedEntry {
            id: id.to_string(),
            agent: agent.to_string(),
            kind: "task_complete".to_string(),
            summary: format!("{agent} summary"),
            content: None,
            files: json!([]),
            task_id: None,
            trace_id: None,
            priority: "medium".to_string(),
            timestamp: timestamp.to_string(),
            tokens: 1,
        },
    )
    .unwrap();
}
#[test]
fn unread_feed_falls_back_when_ack_anchor_is_missing() {
    let conn = setup_conn();
    insert_entry(&conn, "a1", "alpha", "2026-04-10T00:00:00Z");
    insert_entry(&conn, "b1", "beta", "2026-04-10T00:01:00Z");
    insert_entry(&conn, "g1", "gamma", "2026-04-10T00:02:00Z");
    conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)",
        params!["alpha", "missing-anchor", "2026-04-10T00:03:00Z"],
    )
    .unwrap();
    let unread = get_unread_feed(&conn, "alpha", None).unwrap();
    let ids = unread.into_iter().map(|entry| entry.id).collect::<Vec<_>>();
    assert_eq!(ids, vec!["b1".to_string(), "g1".to_string()]);
}
#[test]
fn unread_feed_starts_after_ack_and_skips_self_entries() {
    let conn = setup_conn();
    insert_entry(&conn, "a1", "alpha", "2026-04-10T00:00:00Z");
    insert_entry(&conn, "b1", "beta", "2026-04-10T00:01:00Z");
    insert_entry(&conn, "a2", "alpha", "2026-04-10T00:02:00Z");
    insert_entry(&conn, "g1", "gamma", "2026-04-10T00:03:00Z");
    conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)",
        params!["alpha", "b1", "2026-04-10T00:04:00Z"],
    )
    .unwrap();
    let unread = get_unread_feed(&conn, "alpha", None).unwrap();
    let ids = unread.into_iter().map(|entry| entry.id).collect::<Vec<_>>();
    assert_eq!(ids, vec!["g1".to_string()]);
}
