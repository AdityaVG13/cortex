// SPDX-License-Identifier: MIT
use super::*;

use super::*;
#[test]
fn test_jaccard_identical() {
    assert!(jaccard_similarity("hello world foo", "hello world foo") > 0.99);
}
#[test]
fn test_jaccard_similar() {
    assert!(jaccard_similarity("hello world foo", "hello world bar") > 0.3);
}
#[test]
fn test_jaccard_different() {
    assert!(jaccard_similarity("completely different text", "nothing alike here at all") < 0.1);
}
#[test]
fn test_jaccard_empty() {
    assert_eq!(jaccard_similarity("", ""), 1.0);
}
#[test]
fn test_detect_conflict() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, status) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params!["Cortex uses SQLite for storage", "test", "decision", "claude", "active"],
    )
    .unwrap();
    let result = detect_conflict(&conn, "Cortex uses SQLite for storage", "claude", None).unwrap();
    assert!(result.is_update);
    assert!(!result.is_conflict);
    assert_eq!(result.classification, ConflictClassification::Agrees);
    let result = detect_conflict(&conn, "Cortex uses SQLite for storage", "droid", None).unwrap();
    assert!(!result.is_conflict);
    assert!(!result.is_update);
    assert_eq!(result.classification, ConflictClassification::Agrees);
    let result = detect_conflict(&conn, "Never use SQLite for storage", "droid", None).unwrap();
    assert_eq!(result.classification, ConflictClassification::Contradicts);
    assert!(result.is_conflict);
    let result = detect_conflict(&conn, "Something totally different and new", "claude", None).unwrap();
    assert!(!result.is_conflict);
    assert!(!result.is_update);
    assert_eq!(result.classification, ConflictClassification::Unrelated);
}
#[test]
fn test_detect_conflict_ignores_expired_decisions() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, status, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now', '-1 second'))",
        rusqlite::params!["Cortex uses SQLite for storage", "test", "decision", "claude", "active"],
    )
    .unwrap();
    let result = detect_conflict(&conn, "Cortex uses SQLite for storage", "claude", None).unwrap();
    assert!(!result.is_conflict);
    assert!(!result.is_update);
}
#[test]
fn test_detect_conflict_scopes_by_owner_when_requested() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    conn.execute("ALTER TABLE decisions ADD COLUMN owner_id INTEGER DEFAULT 0", []).unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params!["Always use sqlite for local memory", "owner-one", "decision", "claude", "active", 1_i64],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params!["Never use sqlite for local memory", "owner-two", "decision", "droid", "active", 2_i64],
    )
    .unwrap();
    let result = detect_conflict(&conn, "Always use sqlite for local memory", "claude", Some(1)).unwrap();
    assert_eq!(result.matched_id, Some(1));
}
