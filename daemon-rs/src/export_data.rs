// SPDX-License-Identifier: MIT
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Sql,
}

impl ExportFormat {
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "json" => Some(Self::Json),
            "sql" => Some(Self::Sql),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
pub struct ImportPayload {
    pub memories: Option<Vec<ImportMemory>>,
    pub decisions: Option<Vec<ImportDecision>>,
}

#[derive(Deserialize)]
pub struct ImportMemory {
    pub text: String,
    pub source: Option<String>,
    #[serde(rename = "type")]
    pub typ: Option<String>,
    pub tags: Option<String>,
    pub source_agent: Option<String>,
    pub source_client: Option<String>,
    pub source_model: Option<String>,
    pub confidence: Option<f64>,
    pub reasoning_depth: Option<String>,
    pub trust_score: Option<f64>,
    pub score: Option<f64>,
}

#[derive(Deserialize)]
pub struct ImportDecision {
    pub decision: String,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub typ: Option<String>,
    pub source_agent: Option<String>,
    pub source_client: Option<String>,
    pub source_model: Option<String>,
    pub confidence: Option<f64>,
    pub reasoning_depth: Option<String>,
    pub trust_score: Option<f64>,
    pub score: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub owner_id: Option<i64>,
    pub visibility: Option<String>,
    pub source_agent_fallback: String,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            owner_id: None,
            visibility: None,
            source_agent_fallback: "import".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportCounts {
    pub memories: usize,
    pub decisions: usize,
}

pub fn export_json_value(conn: &Connection) -> Value {
    let memories = query_table_json(
        conn,
        "SELECT id, text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, created_at, updated_at FROM memories WHERE status = 'active'",
    );
    let decisions = query_table_json(
        conn,
        "SELECT id, decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, status, score, \
         retrievals, pinned, created_at, updated_at FROM decisions WHERE status = 'active'",
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

pub fn export_sql_text(conn: &Connection) -> String {
    let mut lines: Vec<String> = vec![
        "-- Cortex export".to_string(),
        format!("-- Exported at {}", now_iso()),
        "BEGIN TRANSACTION;".to_string(),
    ];

    if let Ok(mut stmt) = conn.prepare(
        "SELECT text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score FROM memories WHERE status = 'active'",
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
                ) = row;
                lines.push(format!(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'active');",
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
                ));
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score FROM decisions WHERE status = 'active'",
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
                ) = row;
                lines.push(format!(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'active');",
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
            let inserted = if memories_has_owner && memories_has_visibility {
                conn.execute(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13)",
                    params![
                        m.text,
                        m.source,
                        m.typ.as_deref().unwrap_or("memory"),
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
                        options.owner_id,
                        visibility,
                    ],
                )
            } else {
                conn.execute(
                    "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active')",
                    params![
                        m.text,
                        m.source,
                        m.typ.as_deref().unwrap_or("memory"),
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
            let inserted = if decisions_has_owner && decisions_has_visibility {
                conn.execute(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active', ?11, ?12)",
                    params![
                        d.decision,
                        d.context,
                        d.typ.as_deref().unwrap_or("decision"),
                        d.source_agent.as_deref().unwrap_or(fallback),
                        d.source_client
                            .as_deref()
                            .unwrap_or(d.source_agent.as_deref().unwrap_or(fallback)),
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
                        options.owner_id,
                        visibility,
                    ],
                )
            } else {
                conn.execute(
                    "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'active')",
                    params![
                        d.decision,
                        d.context,
                        d.typ.as_deref().unwrap_or("decision"),
                        d.source_agent.as_deref().unwrap_or(fallback),
                        d.source_client
                            .as_deref()
                            .unwrap_or(d.source_agent.as_deref().unwrap_or(fallback)),
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
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
