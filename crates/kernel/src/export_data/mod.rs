pub use crate::api_types::{ImportCounts, ImportOptions, ImportPayload};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub const DEFAULT_EXPORT_PAGE_LIMIT: usize = 1000;
pub const MAX_EXPORT_PAGE_LIMIT: usize = 5000;
fn normalize_entry_type(raw: Option<&str>, default: &str, aliases: &[(&[&str], &str)]) -> String {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| default.to_string());
    for (keys, mapped) in aliases {
        if keys.contains(&normalized.as_str()) {
            return (*mapped).to_string();
        }
    }
    normalized
}
fn normalize_memory_entry_type(raw: Option<&str>) -> String {
    normalize_entry_type(
        raw,
        "fact",
        &[
            (
                &["memory", "note", "finding", "observation", "fact"],
                "fact",
            ),
            (&["episode", "event"], "episode"),
            (
                &["procedure", "playbook", "runbook", "howto", "how-to"],
                "procedure",
            ),
            (&["evidence", "citation", "reference"], "evidence"),
        ],
    )
}
fn normalize_decision_entry_type(raw: Option<&str>) -> String {
    normalize_entry_type(
        raw,
        "decision",
        &[
            // policy/rule/convention/contract/constraint/preference are their
            // own kinds (boot Constraints, required-role recall, coverage).
            // Mapping them to "decision" dropped that identity on import.
            (&["procedure", "playbook", "runbook"], "procedure"),
            (&["evidence", "citation", "reference"], "evidence"),
            (&["fact", "memory", "note"], "fact"),
        ],
    )
}
/// SQLite `COALESCE` does not skip `''`. Empty temporal fields must bind as
/// NULL so they mean "unbounded", matching omitted JSON properties.
fn optional_time(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}
pub fn export_json_page_value(
    conn: &Connection,
    limit: usize,
    memories_offset: usize,
    decisions_offset: usize,
) -> Result<Value, String> {
    let limit = limit.clamp(1, MAX_EXPORT_PAGE_LIMIT);
    let(memories,memories_has_more)=
query_table_json_page(conn,
"SELECT id, text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, retention_class, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM memories WHERE status = 'active' ORDER BY id LIMIT ?1 OFFSET ?2"
,limit,memories_offset,)?;
    let(decisions,decisions_has_more)=query_table_json_page(conn,
"SELECT id, decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, retention_class, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM decisions WHERE status = 'active' ORDER BY id LIMIT ?1 OFFSET ?2"
,limit,decisions_offset,)?;
    Ok(
        json!({"version":1,"mode":"page","exported_at":now_iso(),"limit":limit,"memories_offset":memories_offset
,"decisions_offset":decisions_offset,"next_memories_offset":next_page_offset(memories_offset,&memories,memories_has_more),"next_decisions_offset":next_page_offset(decisions_offset,&decisions,decisions_has_more),"truncated":memories_has_more||decisions_has_more,"memories":memories,"decisions":decisions,"memories_count":
memories.len(),"decisions_count":decisions.len(),}),
    )
}
pub fn export_json_changeset_value(
    conn: &Connection,
    since: Option<&str>,
) -> Result<Value, String> {
    let cursor = now_iso();
    // `updated_at` mixes RFC3339 (`now_iso`) and SQLite `datetime('now')`.
    // Lexicographic compare treats space as less than `T`, so a RFC3339
    // cursor silently skips same-day SQLite timestamps. Compare as instants.
    let lower = since.unwrap_or("0001-01-01T00:00:00.000Z");
    let memories = query_rows_json(
        conn,
        "SELECT id, text, source, type, status, created_at, updated_at FROM memories WHERE status = 'active' AND julianday(updated_at) > julianday(?1) AND julianday(updated_at) <= julianday(?2) ORDER BY id",
        &[&lower, &cursor],
    )?;
    let decisions = query_rows_json(
        conn,
        "SELECT id, decision, context, type, status, created_at, updated_at FROM decisions WHERE status = 'active' AND julianday(updated_at) > julianday(?1) AND julianday(updated_at) <= julianday(?2) ORDER BY id",
        &[&lower, &cursor],
    )?;
    Ok(
        json!({"version":1,"mode":"changeset","cursor":cursor,"since":since,"memories":memories,"decisions":decisions}),
    )
}
fn redacted_payload(payload: &ImportPayload, counts: &mut ImportCounts) -> ImportPayload {
    let mut out = payload.clone();
    if let Some(memories) = out.memories.as_mut() {
        memories.retain_mut(|m| {
            let redacted = crate::handlers::redact_secrets(m.text.trim());
            if redacted.is_empty() {
                counts.excluded += 1;
                return false;
            }
            if redacted != m.text.trim() {
                counts.redacted += 1;
            }
            m.text = redacted;
            true
        });
    }
    if let Some(decisions) = out.decisions.as_mut() {
        decisions.retain_mut(|d| {
            let redacted = crate::handlers::redact_secrets(d.decision.trim());
            let context = d
                .context
                .as_deref()
                .map(|c| crate::handlers::redact_secrets(c));
            if redacted.is_empty() {
                counts.excluded += 1;
                return false;
            }
            if redacted != d.decision.trim() || context != d.context {
                counts.redacted += 1;
            }
            d.decision = redacted;
            d.context = context;
            true
        });
    }
    out
}
pub fn import_payload(
    conn: &mut Connection,
    payload: &ImportPayload,
    options: &ImportOptions,
) -> Result<ImportCounts, String> {
    let mut counts = ImportCounts::default();
    let visibility = options.visibility.as_deref().unwrap_or("private");
    let fallback = options.source_agent_fallback.as_str();
    let memories_has_owner = column_exists(conn, "memories", "owner_id")?;
    let memories_has_visibility = column_exists(conn, "memories", "visibility")?;
    let decisions_has_owner = column_exists(conn, "decisions", "owner_id")?;
    let decisions_has_visibility = column_exists(conn, "decisions", "visibility")?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("failed to start import transaction: {e}"))?;
    // One redaction stage ahead of every write path: imported rows are
    // redacted exactly like /store and /feed before FTS, entities or anchors
    // can see them. Redaction and exclusion are counted, never silent.
    let payload = redacted_payload(payload, &mut counts);
    let payload = &payload;
    if let Some(memories) = &payload.memories {
        for (idx, m) in memories.iter().enumerate() {
            let entry_type = normalize_memory_entry_type(m.entry_type.as_deref());
            let observed_at = optional_time(m.observed_at.as_deref());
            let valid_from = optional_time(m.valid_from.as_deref());
            let valid_until = optional_time(m.valid_until.as_deref());
            let inserted = if memories_has_owner && memories_has_visibility {
                tx.
execute(
"INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', COALESCE(?13, ?14, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?14, ?13, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?15, ?16, ?17)"
,params![m.text,m.source,entry_type,m.tags,m.source_agent.as_deref().unwrap_or(fallback),m.source_client.as_deref().unwrap_or(m.
source_agent.as_deref().unwrap_or(fallback)),m.source_model.as_deref(),m.confidence.unwrap_or(0.8),m.reasoning_depth.as_deref().
unwrap_or("single-shot"),m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),m.score.unwrap_or(1.0),m.retention_class.
unwrap_or_default().as_str(),observed_at,valid_from,valid_until,options.owner_id,visibility
,],)
            } else {
                tx.execute(
"INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', COALESCE(?13, ?14, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?14, ?13, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?15)"
,params![m.text,m.source,entry_type,m.tags,m.source_agent.as_deref().unwrap_or(fallback),m.source_client.as_deref().unwrap_or(m.
source_agent.as_deref().unwrap_or(fallback)),m.source_model.as_deref(),m.confidence.unwrap_or(0.8),m.reasoning_depth.as_deref().
unwrap_or("single-shot"),m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),m.score.unwrap_or(1.0),m.retention_class.
unwrap_or_default().as_str(),observed_at,valid_from,valid_until,],)
            };
            match inserted {
                Ok(_) => counts.memories += 1,
                Err(e) => return Err(format!("failed to import memories[{idx}]: {e}")),
            }
        }
    }
    if let Some(decisions) = &payload.decisions {
        for (idx, d) in decisions.iter().enumerate() {
            let entry_type = normalize_decision_entry_type(d.entry_type.as_deref());
            let observed_at = optional_time(d.observed_at.as_deref());
            let valid_from = optional_time(d.valid_from.as_deref());
            let valid_until = optional_time(d.valid_until.as_deref());
            let inserted = if decisions_has_owner && decisions_has_visibility {
                tx.execute(
"INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', COALESCE(?12, ?13, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?13, ?12, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?14, ?15, ?16)"
,params![d.decision,d.context,entry_type,d.source_agent.as_deref().unwrap_or(fallback),d.source_client.as_deref().unwrap_or(d.
source_agent.as_deref().unwrap_or(fallback)),d.source_model.as_deref(),d.confidence.unwrap_or(0.8),d.reasoning_depth.as_deref().
unwrap_or("single-shot"),d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),d.score.unwrap_or(1.0),d.retention_class.
unwrap_or_default().as_str(),observed_at,valid_from,valid_until,options.owner_id,visibility
,],)
            } else {
                tx.execute(
"INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', COALESCE(?12, ?13, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), COALESCE(?13, ?12, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?14)"
,params![d.decision,d.context,entry_type,d.source_agent.as_deref().unwrap_or(fallback),d.source_client.as_deref().unwrap_or(d.
source_agent.as_deref().unwrap_or(fallback)),d.source_model.as_deref(),d.confidence.unwrap_or(0.8),d.reasoning_depth.as_deref().
unwrap_or("single-shot"),d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),d.score.unwrap_or(1.0),d.retention_class.
unwrap_or_default().as_str(),observed_at,valid_from,valid_until,],)
            };
            match inserted {
                Ok(_) => counts.decisions += 1,
                Err(e) => return Err(format!("failed to import decisions[{idx}]: {e}")),
            }
        }
    }
    tx.commit()
        .map_err(|e| format!("failed to commit import transaction: {e}"))?;
    Ok(counts)
}
fn row_to_json(row: &rusqlite::Row<'_>, column_names: &[String]) -> rusqlite::Result<Value> {
    let mut obj = serde_json::Map::new();
    for (i, name) in column_names.iter().enumerate() {
        let val: Value = match row.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Null) => Value::Null,
            Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
            Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
            Ok(rusqlite::types::ValueRef::Text(s)) => {
                let text = std::str::from_utf8(s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        i,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?;
                json!(text)
            }
            Ok(rusqlite::types::ValueRef::Blob(_)) => Value::Null,
            Err(err) => return Err(err),
        };
        obj.insert(name.clone(), val);
    }
    Ok(Value::Object(obj))
}
/// A failed export is never indistinguishable from an empty database:
/// prepare/query errors propagate to the caller as `Err`.
fn query_rows_json(
    conn: &Connection,
    sql: &str,
    bind: &[&dyn rusqlite::types::ToSql],
) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("export prepare failed: {e}"))?;
    let column_names: Vec<String> = (0..stmt.column_count())
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();
    let rows = stmt
        .query_map(bind, |row| row_to_json(row, &column_names))
        .map_err(|e| format!("export query failed: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("export row failed: {e}"))
}
fn query_table_json_page(
    conn: &Connection,
    sql: &str,
    limit: usize,
    offset: usize,
) -> Result<(Vec<Value>, bool), String> {
    let fetch_limit = i64::try_from(limit.saturating_add(1))
        .map_err(|_| "export_page_limit".to_string())?;
    let offset =
        i64::try_from(offset).map_err(|_| "export_page_offset".to_string())?;
    let mut rows = query_rows_json(conn, sql, &[&fetch_limit, &offset])?;
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    Ok((rows, has_more))
}
/// Resume cursor for the other table when this page is short but the
/// sibling table still has rows. `None` only when this collection is empty
/// at `offset` (nothing to skip on the next call).
fn next_page_offset(offset: usize, page: &[Value], has_more: bool) -> Option<usize> {
    if has_more || !page.is_empty() {
        Some(offset.saturating_add(page.len()))
    } else {
        None
    }
}
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    if !table.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err("export_schema_table".into());
    }
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("schema inspect failed: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("schema inspect failed: {e}"))?;
    for name in rows {
        let name = name.map_err(|e| format!("schema inspect failed: {e}"))?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}
