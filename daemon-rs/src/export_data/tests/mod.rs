// SPDX-License-Identifier: MIT
use super::*;

use super::*;
#[test]
fn export_changeset_filters_rows_by_since_cutoff() {
    let conn = Connection::open_in_memory().expect("open sqlite");
    crate::db::configure(&conn).expect("configure sqlite");
    crate::db::initialize_schema(&conn).expect("initialize schema");
    crate::db::run_pending_migrations(&conn);
    conn.execute(
        "INSERT INTO memories (text, source, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4)",
        params![
            "old memory",
            "sync::old-memory",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z"
        ],
    )
    .expect("insert old memory");
    conn.execute(
        "INSERT INTO memories (text, source, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4)",
        params![
            "new memory",
            "sync::new-memory",
            "2026-03-01T00:00:00Z",
            "2026-03-01T00:00:00Z"
        ],
    )
    .expect("insert new memory");
    conn.execute(
        "INSERT INTO decisions (decision, context, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4)",
        params![
            "old decision",
            "sync::old-decision",
            "2026-01-01T00:00:00Z",
            "2026-01-01T00:00:00Z"
        ],
    )
    .expect("insert old decision");
    conn.execute(
        "INSERT INTO decisions (decision, context, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4)",
        params![
            "new decision",
            "sync::new-decision",
            "2026-03-01T00:00:00Z",
            "2026-03-01T00:00:00Z"
        ],
    )
    .expect("insert new decision");
    let changeset = export_json_changeset_value(&conn, Some("2026-02-01T00:00:00Z"));
    let memories = changeset
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let decisions = changeset
        .get("decisions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(memories.len(), 1, "only new memory should be exported");
    assert_eq!(decisions.len(), 1, "only new decision should be exported");
    assert_eq!(
        memories[0].get("source").and_then(Value::as_str),
        Some("sync::new-memory")
    );
    assert_eq!(
        decisions[0].get("context").and_then(Value::as_str),
        Some("sync::new-decision")
    );
}
#[test]
fn export_changeset_respects_cursor_upper_bound() {
    let conn = Connection::open_in_memory().expect("open sqlite");
    crate::db::configure(&conn).expect("configure sqlite");
    crate::db::initialize_schema(&conn).expect("initialize schema");
    crate::db::run_pending_migrations(&conn);
    conn.execute(
        "INSERT INTO memories (text, source, status, created_at, updated_at)
             VALUES (?1, ?2, 'active', ?3, ?4)",
        params![
            "future memory",
            "sync::future-memory",
            "9999-01-01T00:00:00Z",
            "9999-01-01T00:00:00Z"
        ],
    )
    .expect("insert future memory");
    let changeset = export_json_changeset_value(&conn, None);
    let memories = changeset
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        memories.is_empty(),
        "rows newer than cursor should be excluded"
    );
    assert!(
        changeset
            .get("cursor")
            .and_then(Value::as_str)
            .is_some_and(|cursor| !cursor.trim().is_empty()),
        "changeset cursor should always be emitted"
    );
}
#[test]
fn export_json_page_limits_rows_and_emits_next_offsets() {
    let conn = Connection::open_in_memory().expect("open sqlite");
    crate::db::configure(&conn).expect("configure sqlite");
    crate::db::initialize_schema(&conn).expect("initialize schema");
    crate::db::run_pending_migrations(&conn);
    for idx in 0..3 {
        conn.execute(
            "INSERT INTO memories (text, source, status) VALUES (?1, ?2, 'active')",
            params![format!("memory {idx}"), format!("page::memory::{idx}")],
        )
        .expect("insert memory");
    }
    for idx in 0..2 {
        conn.execute(
            "INSERT INTO decisions (decision, context, status) VALUES (?1, ?2, 'active')",
            params![format!("decision {idx}"), format!("page::decision::{idx}")],
        )
        .expect("insert decision");
    }
    let first_page = export_json_page_value(&conn, 2, 0, 0);
    assert_eq!(
        first_page
            .get("memories")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        first_page
            .get("decisions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        first_page
            .get("next_memories_offset")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        first_page
            .get("next_decisions_offset")
            .and_then(Value::as_u64),
        None
    );
    assert_eq!(
        first_page.get("truncated").and_then(Value::as_bool),
        Some(true)
    );
    let second_page = export_json_page_value(&conn, 2, 2, 0);
    let memories = second_page
        .get("memories")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(memories.len(), 1);
    assert_eq!(
        memories[0].get("source").and_then(Value::as_str),
        Some("page::memory::2")
    );
    assert_eq!(
        second_page
            .get("next_memories_offset")
            .and_then(Value::as_u64),
        None
    );
}
#[test]
fn import_payload_normalizes_types_and_preserves_temporal_fields() {
    let mut conn = Connection::open_in_memory().expect("open sqlite");
    crate::db::configure(&conn).expect("configure sqlite");
    crate::db::initialize_schema(&conn).expect("initialize schema");
    crate::db::run_pending_migrations(&conn);
    let payload = ImportPayload {
        memories: Some(vec![crate::api_types::ImportMemory {
            text: "deployment runbook".to_string(),
            source: Some("ops".to_string()),
            entry_type: Some("note".to_string()),
            tags: Some("deploy".to_string()),
            source_agent: Some("importer".to_string()),
            source_client: Some("tests".to_string()),
            source_model: Some("model-a".to_string()),
            confidence: Some(0.91),
            reasoning_depth: Some("analysis".to_string()),
            trust_score: Some(0.88),
            score: Some(1.2),
            observed_at: Some("2026-04-18T10:00:00Z".to_string()),
            valid_from: Some("2026-04-18T00:00:00Z".to_string()),
            valid_until: Some("2026-05-18T00:00:00Z".to_string()),
            retention_class: Some(crate::api_types::RetentionClass::Operational),
        }]),
        decisions: Some(vec![crate::api_types::ImportDecision {
            decision: "route traffic via canary".to_string(),
            context: Some("release gate".to_string()),
            entry_type: Some("rule".to_string()),
            source_agent: Some("importer".to_string()),
            source_client: Some("tests".to_string()),
            source_model: Some("model-b".to_string()),
            confidence: Some(0.86),
            reasoning_depth: Some("analysis".to_string()),
            trust_score: Some(0.83),
            score: Some(1.1),
            observed_at: Some("2026-04-18T11:00:00Z".to_string()),
            valid_from: Some("2026-04-18T00:00:00Z".to_string()),
            valid_until: Some("2026-05-01T00:00:00Z".to_string()),
            retention_class: Some(crate::api_types::RetentionClass::Audit),
        }]),
    };
    let counts = import_payload(&mut conn, &payload, &ImportOptions::default())
        .expect("import should succeed");
    assert_eq!(counts.memories, 1);
    assert_eq!(counts.decisions, 1);
    let memory_row: (String, String, Option<String>, Option<String>, Option<String>) = conn
            .query_row("SELECT type, retention_class, observed_at, valid_from, valid_until FROM memories LIMIT 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })
            .expect("memory row");
    assert_eq!(memory_row.0, "fact");
    assert_eq!(memory_row.1, "operational");
    assert_eq!(memory_row.2.as_deref(), Some("2026-04-18T10:00:00Z"));
    assert_eq!(memory_row.3.as_deref(), Some("2026-04-18T00:00:00Z"));
    assert_eq!(memory_row.4.as_deref(), Some("2026-05-18T00:00:00Z"));
    let decision_row: (String, String, Option<String>, Option<String>, Option<String>) = conn
            .query_row("SELECT type, retention_class, observed_at, valid_from, valid_until FROM decisions LIMIT 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })
            .expect("decision row");
    assert_eq!(decision_row.0, "decision");
    assert_eq!(decision_row.1, "audit");
    assert_eq!(decision_row.2.as_deref(), Some("2026-04-18T11:00:00Z"));
    assert_eq!(decision_row.3.as_deref(), Some("2026-04-18T00:00:00Z"));
    assert_eq!(decision_row.4.as_deref(), Some("2026-05-01T00:00:00Z"));
}
#[test]
fn import_payload_rolls_back_and_reports_failed_rows() {
    let mut conn = Connection::open_in_memory().expect("open sqlite");
    crate::db::configure(&conn).expect("configure sqlite");
    crate::db::initialize_schema(&conn).expect("initialize schema");
    crate::db::run_pending_migrations(&conn);
    conn.execute(
        "CREATE TRIGGER fail_import_memory BEFORE INSERT ON memories
             WHEN NEW.source = 'fail'
             BEGIN
                 SELECT RAISE(ABORT, 'forced import failure');
             END",
        [],
    )
    .expect("create failure trigger");
    let payload = ImportPayload {
        memories: Some(vec![
            crate::api_types::ImportMemory {
                text: "first memory".to_string(),
                source: Some("ok".to_string()),
                entry_type: None,
                tags: None,
                source_agent: None,
                source_client: None,
                source_model: None,
                confidence: None,
                reasoning_depth: None,
                trust_score: None,
                score: None,
                observed_at: None,
                valid_from: None,
                valid_until: None,
                retention_class: None,
            },
            crate::api_types::ImportMemory {
                text: "second memory".to_string(),
                source: Some("fail".to_string()),
                entry_type: None,
                tags: None,
                source_agent: None,
                source_client: None,
                source_model: None,
                confidence: None,
                reasoning_depth: None,
                trust_score: None,
                score: None,
                observed_at: None,
                valid_from: None,
                valid_until: None,
                retention_class: None,
            },
        ]),
        decisions: None,
    };
    let err = import_payload(&mut conn, &payload, &ImportOptions::default())
        .expect_err("second memory should fail");
    assert!(err.contains("memories[1]"));
    let row_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count memories");
    assert_eq!(row_count, 0, "import should roll back earlier rows");
}
