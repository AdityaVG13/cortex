//! Export and import handlers.
//!
//! GET  /export?format=json|sql  -- dump all active memories + decisions
//! POST /import                  -- restore from a JSON export payload

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ensure_auth, json_error, json_response};
use crate::state::RuntimeState;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
}

pub async fn handle_export(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let format = query.format.as_deref().unwrap_or("json");
    let conn = state.db.lock().await;

    match format {
        "json" => export_json(&conn),
        "sql" => export_sql(&conn),
        _ => json_error(
            StatusCode::BAD_REQUEST,
            "Unsupported format: use json or sql",
        ),
    }
}

fn export_json(conn: &rusqlite::Connection) -> Response {
    let memories = query_table_json(
        conn,
        "memories",
        "SELECT id, text, source, type, tags, source_agent, confidence, status, score, \
         retrievals, pinned, created_at, updated_at FROM memories WHERE status = 'active'",
    );

    let decisions = query_table_json(
        conn,
        "decisions",
        "SELECT id, decision, context, type, source_agent, confidence, status, score, \
         retrievals, pinned, created_at, updated_at FROM decisions WHERE status = 'active'",
    );

    json_response(
        StatusCode::OK,
        json!({
            "version": 1,
            "exported_at": super::now_iso(),
            "memories": memories,
            "decisions": decisions,
            "memories_count": memories.len(),
            "decisions_count": decisions.len(),
        }),
    )
}

fn query_table_json(conn: &rusqlite::Connection, _table: &str, sql: &str) -> Vec<Value> {
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

fn export_sql(conn: &rusqlite::Connection) -> Response {
    let mut lines: Vec<String> = vec![
        "-- Cortex export".to_string(),
        format!("-- Exported at {}", super::now_iso()),
        "BEGIN TRANSACTION;".to_string(),
    ];

    if let Ok(mut stmt) = conn.prepare(
        "SELECT text, source, type, tags, source_agent, confidence, score FROM memories WHERE status = 'active'"
    ) {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<f64>>(5)?,
                row.get::<_, Option<f64>>(6)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (text, source, typ, tags, agent, confidence, score) = row;
                lines.push(format!(
                    "INSERT INTO memories (text, source, type, tags, source_agent, confidence, score, status) VALUES ({}, {}, {}, {}, {}, {}, {}, 'active');",
                    sql_quote(&text),
                    sql_quote_opt(&source),
                    sql_quote_opt(&typ),
                    sql_quote_opt(&tags),
                    sql_quote_opt(&agent),
                    confidence.unwrap_or(0.8),
                    score.unwrap_or(1.0),
                ));
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT decision, context, type, source_agent, confidence, score FROM decisions WHERE status = 'active'"
    ) {
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<f64>>(5)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (decision, context, typ, agent, confidence, score) = row;
                lines.push(format!(
                    "INSERT INTO decisions (decision, context, type, source_agent, confidence, score, status) VALUES ({}, {}, {}, {}, {}, {}, 'active');",
                    sql_quote(&decision),
                    sql_quote_opt(&context),
                    sql_quote_opt(&typ),
                    sql_quote_opt(&agent),
                    confidence.unwrap_or(0.8),
                    score.unwrap_or(1.0),
                ));
            }
        }
    }

    lines.push("COMMIT;".to_string());

    let body = lines.join("\n");
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut()
        .insert("content-type", "text/plain; charset=utf-8".parse().unwrap());
    resp
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

#[derive(Deserialize)]
pub struct ImportPayload {
    memories: Option<Vec<ImportMemory>>,
    decisions: Option<Vec<ImportDecision>>,
}

#[derive(Deserialize)]
struct ImportMemory {
    text: String,
    source: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    tags: Option<String>,
    source_agent: Option<String>,
    confidence: Option<f64>,
    score: Option<f64>,
}

#[derive(Deserialize)]
struct ImportDecision {
    decision: String,
    context: Option<String>,
    #[serde(rename = "type")]
    typ: Option<String>,
    source_agent: Option<String>,
    confidence: Option<f64>,
    score: Option<f64>,
}

pub async fn handle_import(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ImportPayload>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let conn = state.db.lock().await;
    let mut mem_count = 0usize;
    let mut dec_count = 0usize;

    if let Some(memories) = &payload.memories {
        for m in memories {
            let result = conn.execute(
                "INSERT INTO memories (text, source, type, tags, source_agent, confidence, score, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')",
                params![
                    m.text,
                    m.source,
                    m.typ.as_deref().unwrap_or("memory"),
                    m.tags,
                    m.source_agent.as_deref().unwrap_or("import"),
                    m.confidence.unwrap_or(0.8),
                    m.score.unwrap_or(1.0),
                ],
            );
            if result.is_ok() {
                mem_count += 1;
            }
        }
    }

    if let Some(decisions) = &payload.decisions {
        for d in decisions {
            let result = conn.execute(
                "INSERT INTO decisions (decision, context, type, source_agent, confidence, score, status) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
                params![
                    d.decision,
                    d.context,
                    d.typ.as_deref().unwrap_or("decision"),
                    d.source_agent.as_deref().unwrap_or("import"),
                    d.confidence.unwrap_or(0.8),
                    d.score.unwrap_or(1.0),
                ],
            );
            if result.is_ok() {
                dec_count += 1;
            }
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "imported": {
                "memories": mem_count,
                "decisions": dec_count,
            }
        }),
    )
}

use axum::response::IntoResponse;
