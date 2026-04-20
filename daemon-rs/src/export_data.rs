// SPDX-License-Identifier: MIT
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub use crate::api_types::{ExportFormat, ImportCounts, ImportOptions, ImportPayload};

fn normalize_memory_entry_type(raw: Option<&str>) -> String {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "fact".to_string());
    match normalized.as_str() {
        "memory" | "note" | "finding" | "observation" | "fact" => "fact".to_string(),
        "episode" | "event" => "episode".to_string(),
        "procedure" | "playbook" | "runbook" | "howto" | "how-to" => "procedure".to_string(),
        "evidence" | "citation" | "reference" => "evidence".to_string(),
        "decision" | "policy" | "rule" => "decision".to_string(),
        other => other.to_string(),
    }
}

fn normalize_decision_entry_type(raw: Option<&str>) -> String {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "decision".to_string());
    match normalized.as_str() {
        "decision" | "policy" | "rule" => "decision".to_string(),
        "procedure" | "playbook" | "runbook" => "procedure".to_string(),
        "evidence" | "citation" | "reference" => "evidence".to_string(),
        "fact" | "memory" | "note" => "fact".to_string(),
        other => other.to_string(),
    }
}

pub fn export_json_value(conn: &Connection) -> Value {
    let memories = query_table_json(
        conn,
        "SELECT id, text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM memories WHERE status = 'active'",
    );
    let decisions = query_table_json(
        conn,
        "SELECT id, decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM decisions WHERE status = 'active'",
    );

    json!({
        "version": 1,
        "exported_at": now_iso(),
        "memories": memories,
        "decisions": decisions,
        "memories_count": memories.len(),
        "decisions_count": decisions.len(),
    })
}

pub fn export_json_changeset_value(conn: &Connection, since: Option<&str>) -> Value {
    let cursor = now_iso();
    let memories = query_table_json_since(
        conn,
        "SELECT id, text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM memories WHERE status = 'active' \
         AND (?1 IS NULL OR COALESCE(updated_at, created_at) > ?1) \
         AND COALESCE(updated_at, created_at) <= ?2",
        since,
        &cursor,
    );
    let decisions = query_table_json_since(
        conn,
        "SELECT id, decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM decisions WHERE status = 'active' \
         AND (?1 IS NULL OR COALESCE(updated_at, created_at) > ?1) \
         AND COALESCE(updated_at, created_at) <= ?2",
        since,
        &cursor,
    );

    json!({
        "version": 1,
        "mode": "changeset",
        "exported_at": cursor,
        "since": since,
        "cursor": cursor,
        "memories": memories,
        "decisions": decisions,
        "memories_count": memories.len(),
        "decisions_count": decisions.len(),
    })
}

pub fn export_sql_text(conn: &Connection) -> String {
    let mut lines: Vec<String> = vec![
        "-- Cortex export".to_string(),
        format!("-- Exported at {}", now_iso()),
        "BEGIN TRANSACTION;".to_string(),
    ];

    if let Ok(mut stmt) = conn.prepare(
        "SELECT text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, observed_at, valid_from, valid_until FROM memories WHERE status = 'active'",
    ) {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<f64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, Option<f64>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (
                    text,
                    source,
                    typ,
                    tags,
                    agent,
                    source_client,
                    source_model,
                    confidence,
                    reasoning_depth,
                    trust_score,
                    score,
                    observed_at,
                    valid_from,
                    valid_until,
                ) = row;
                lines.push(format!(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, observed_at, valid_from, valid_until, status) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'active');",
                    sql_quote(&text),
                    sql_quote_opt(&source),
                    sql_quote_opt(&typ),
                    sql_quote_opt(&tags),
                    sql_quote_opt(&agent),
                    sql_quote_opt(&source_client),
                    sql_quote_opt(&source_model),
                    confidence.unwrap_or(0.8),
                    sql_quote_opt(&reasoning_depth),
                    trust_score.unwrap_or(confidence.unwrap_or(0.8)),
                    score.unwrap_or(1.0),
                    sql_quote_opt(&observed_at),
                    sql_quote_opt(&valid_from),
                    sql_quote_opt(&valid_until),
                ));
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, observed_at, valid_from, valid_until FROM decisions WHERE status = 'active'",
    ) {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<f64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<f64>>(8)?,
                row.get::<_, Option<f64>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (
                    decision,
                    context,
                    typ,
                    agent,
                    source_client,
                    source_model,
                    confidence,
                    reasoning_depth,
                    trust_score,
                    score,
                    observed_at,
                    valid_from,
                    valid_until,
                ) = row;
                lines.push(format!(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, observed_at, valid_from, valid_until, status) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'active');",
                    sql_quote(&decision),
                    sql_quote_opt(&context),
                    sql_quote_opt(&typ),
                    sql_quote_opt(&agent),
                    sql_quote_opt(&source_client),
                    sql_quote_opt(&source_model),
                    confidence.unwrap_or(0.8),
                    sql_quote_opt(&reasoning_depth),
                    trust_score.unwrap_or(confidence.unwrap_or(0.8)),
                    score.unwrap_or(1.0),
                    sql_quote_opt(&observed_at),
                    sql_quote_opt(&valid_from),
                    sql_quote_opt(&valid_until),
                ));
            }
        }
    }

    lines.push("COMMIT;".to_string());
    lines.join("\n")
}

pub fn import_payload(
    conn: &Connection,
    payload: &ImportPayload,
    options: &ImportOptions,
) -> ImportCounts {
    let mut counts = ImportCounts::default();
    let visibility = options.visibility.as_deref().unwrap_or("private");
    let fallback = options.source_agent_fallback.as_str();

    let memories_has_owner = column_exists(conn, "memories", "owner_id");
    let memories_has_visibility = column_exists(conn, "memories", "visibility");
    let decisions_has_owner = column_exists(conn, "decisions", "owner_id");
    let decisions_has_visibility = column_exists(conn, "decisions", "visibility");

    if let Some(memories) = &payload.memories {
        for m in memories {
            let entry_type = normalize_memory_entry_type(m.entry_type.as_deref());
            let inserted = if memories_has_owner && memories_has_visibility {
                conn.execute(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, ?14, ?15, ?16)",
                    params![
                        m.text,
                        m.source,
                        entry_type,
                        m.tags,
                        m.source_agent.as_deref().unwrap_or(fallback),
                        m.source_client
                            .as_deref()
                            .unwrap_or(m.source_agent.as_deref().unwrap_or(fallback)),
                        m.source_model.as_deref(),
                        m.confidence.unwrap_or(0.8),
                        m.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),
                        m.score.unwrap_or(1.0),
                        m.observed_at.as_deref(),
                        m.valid_from.as_deref(),
                        m.valid_until.as_deref(),
                        options.owner_id,
                        visibility,
                    ],
                )
            } else {
                conn.execute(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, ?14)",
                    params![
                        m.text,
                        m.source,
                        entry_type,
                        m.tags,
                        m.source_agent.as_deref().unwrap_or(fallback),
                        m.source_client
                            .as_deref()
                            .unwrap_or(m.source_agent.as_deref().unwrap_or(fallback)),
                        m.source_model.as_deref(),
                        m.confidence.unwrap_or(0.8),
                        m.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),
                        m.score.unwrap_or(1.0),
                        m.observed_at.as_deref(),
                        m.valid_from.as_deref(),
                        m.valid_until.as_deref(),
                    ],
                )
            };

            if inserted.is_ok() {
                counts.memories += 1;
            }
        }
    }

    if let Some(decisions) = &payload.decisions {
        for d in decisions {
            let entry_type = normalize_decision_entry_type(d.entry_type.as_deref());
            let inserted = if decisions_has_owner && decisions_has_visibility {
                conn.execute(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12, ?13, ?14, ?15)",
                    params![
                        d.decision,
                        d.context,
                        entry_type,
                        d.source_agent.as_deref().unwrap_or(fallback),
                        d.source_client
                            .as_deref()
                            .unwrap_or(d.source_agent.as_deref().unwrap_or(fallback)),
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
                        d.observed_at.as_deref(),
                        d.valid_from.as_deref(),
                        d.valid_until.as_deref(),
                        options.owner_id,
                        visibility,
                    ],
                )
            } else {
                conn.execute(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12, ?13)",
                    params![
                        d.decision,
                        d.context,
                        entry_type,
                        d.source_agent.as_deref().unwrap_or(fallback),
                        d.source_client
                            .as_deref()
                            .unwrap_or(d.source_agent.as_deref().unwrap_or(fallback)),
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
                        d.observed_at.as_deref(),
                        d.valid_from.as_deref(),
                        d.valid_until.as_deref(),
                    ],
                )
            };

            if inserted.is_ok() {
                counts.decisions += 1;
            }
        }
    }

    counts
}

fn query_table_json(conn: &Connection, sql: &str) -> Vec<Value> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let column_count = stmt.column_count();
    let column_names: Vec<String> = (0..column_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    stmt.query_map([], |row| {
        let mut obj = serde_json::Map::new();
        for (i, name) in column_names.iter().enumerate() {
            let val: Value = match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
                Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
                Ok(rusqlite::types::ValueRef::Text(s)) => {
                    json!(std::str::from_utf8(s).unwrap_or(""))
                }
                Ok(rusqlite::types::ValueRef::Blob(_)) => Value::Null,
                Err(_) => Value::Null,
            };
            obj.insert(name.clone(), val);
        }
        Ok(Value::Object(obj))
    })
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|r| r.ok())
    .collect()
}

fn query_table_json_since(
    conn: &Connection,
    sql: &str,
    since: Option<&str>,
    cursor: &str,
) -> Vec<Value> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    let column_count = stmt.column_count();
    let column_names: Vec<String> = (0..column_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    stmt.query_map(params![since, cursor], |row| {
        let mut obj = serde_json::Map::new();
        for (i, name) in column_names.iter().enumerate() {
            let val: Value = match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
                Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
                Ok(rusqlite::types::ValueRef::Text(s)) => {
                    json!(std::str::from_utf8(s).unwrap_or(""))
                }
                Ok(rusqlite::types::ValueRef::Blob(_)) => Value::Null,
                Err(_) => Value::Null,
            };
            obj.insert(name.clone(), val);
        }
        Ok(Value::Object(obj))
    })
    .ok()
    .into_iter()
    .flatten()
    .filter_map(|r| r.ok())
    .collect()
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn sql_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn sql_quote_opt(s: &Option<String>) -> String {
    match s {
        Some(v) => sql_quote(v),
        None => "NULL".to_string(),
    }
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
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

#[cfg(test)]
mod tests {
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
    fn import_payload_normalizes_types_and_preserves_temporal_fields() {
        let conn = Connection::open_in_memory().expect("open sqlite");
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
            }]),
        };

        let counts = import_payload(&conn, &payload, &ImportOptions::default());
        assert_eq!(counts.memories, 1);
        assert_eq!(counts.decisions, 1);

        let memory_row: (String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT type, observed_at, valid_from, valid_until FROM memories LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("memory row");
        assert_eq!(memory_row.0, "fact");
        assert_eq!(memory_row.1.as_deref(), Some("2026-04-18T10:00:00Z"));
        assert_eq!(memory_row.2.as_deref(), Some("2026-04-18T00:00:00Z"));
        assert_eq!(memory_row.3.as_deref(), Some("2026-05-18T00:00:00Z"));

        let decision_row: (String, Option<String>, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT type, observed_at, valid_from, valid_until FROM decisions LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("decision row");
        assert_eq!(decision_row.0, "decision");
        assert_eq!(decision_row.1.as_deref(), Some("2026-04-18T11:00:00Z"));
        assert_eq!(decision_row.2.as_deref(), Some("2026-04-18T00:00:00Z"));
        assert_eq!(decision_row.3.as_deref(), Some("2026-05-01T00:00:00Z"));
    }
}
