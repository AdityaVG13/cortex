// SPDX-License-Identifier: MIT
//! Store data-integrity boundaries only.

use super::*;
use rusqlite::{params, Connection};

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    crate::db::run_pending_migrations(&conn);
    conn
}

fn insert_existing_decision(conn: &Connection, decision: &str, context: Option<&str>, vector: &[f32]) -> i64 {
    conn.execute(
        "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
         VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 50, datetime('now'), datetime('now'))",
        params![decision, context],
    )
    .unwrap();
    let id = conn.last_insert_rowid();
    persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key()).unwrap();
    id
}

#[test]
fn benchmark_entries_bypass_semantic_merge() {
    let mut conn = test_conn();
    insert_existing_decision(
        &conn,
        "store benchmark messages without dedup collapsing",
        Some("seed"),
        &[1.0, 0.0],
    );

    let (_entry, new_id) = store_decision_with_input_embedding(
        &mut conn,
        "store benchmark messages without dedup collapsing",
        Some("bench-doc".to_string()),
        Some("benchmark".to_string()),
        "tester".to_string(),
        None,
        None,
        Some(&[1.0, 0.0]),
        None,
    )
    .unwrap();

    assert!(new_id.is_some());
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn store_decision_rejects_invalid_explicit_ttl() {
    let mut conn = test_conn();
    let err = store_decision_with_ttl(
        &mut conn,
        "ttl smoke",
        Some("ctx".to_string()),
        Some("decision".to_string()),
        "tester".to_string(),
        None,
        Some(-1),
        None,
    )
    .unwrap_err();
    assert!(err.contains("ttl") || err.contains("TTL"));
}
