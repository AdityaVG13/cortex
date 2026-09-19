pub use crate::api_types::{ImportCounts, ImportOptions, ImportPayload};
use crate::handlers::now_iso;
use crate::protocol::nonempty_opt;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
pub const DEFAULT_EXPORT_PAGE_LIMIT: usize = 1000;
pub const MAX_EXPORT_PAGE_LIMIT: usize = 5000;
fn normalize_entry_type(raw: Option<&str>, default: &str, aliases: &[(&[&str], &str)]) -> String {
    let normalized = nonempty_opt(raw)
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
    nonempty_opt(raw)
}

const PAGE_TAIL: &str = "WHERE status = 'active' ORDER BY id LIMIT ?1 OFFSET ?2";
const CHANGESET_TAIL: &str = "WHERE status = 'active' AND julianday(updated_at) > julianday(?1) AND julianday(updated_at) <= julianday(?2) ORDER BY id";
const PAGE_META: &str = "source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, retention_class, status, score, retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at";
const CHANGESET_META: &str = "type, status, created_at, updated_at";

const IMPORT_NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
const MEMORIES_INSERT: &str = "INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until{acl_cols}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', COALESCE(?13, ?14, {now}), COALESCE(?14, ?13, {now}), ?15{acl_vals})";
const DECISIONS_INSERT: &str = "INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until{acl_cols}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', COALESCE(?12, ?13, {now}), COALESCE(?13, ?12, {now}), ?14{acl_vals})";

fn import_insert_sql(template: &str, has_acl: bool, first_acl: u8) -> String {
    let sql = template.replace("{now}", IMPORT_NOW);
    if has_acl {
        sql.replace("{acl_cols}", ", owner_id, visibility")
            .replace("{acl_vals}", &format!(", ?{first_acl}, ?{}", first_acl + 1))
    } else {
        sql.replace("{acl_cols}", "").replace("{acl_vals}", "")
    }
}

fn imported(result: rusqlite::Result<usize>, kind: &str, idx: usize) -> Result<(), String> {
    result
        .map(|_| ())
        .map_err(|e| format!("failed to import {kind}[{idx}]: {e}"))
}

fn has_acl(conn: &Connection, table: &str) -> Result<bool, String> {
    Ok(column_exists(conn, table, "owner_id")? && column_exists(conn, table, "visibility")?)
}

fn agent_client<'a>(
    agent: Option<&'a str>,
    client: Option<&'a str>,
    fallback: &'a str,
) -> (&'a str, &'a str) {
    let agent = agent.unwrap_or(fallback);
    (agent, client.unwrap_or(agent))
}
pub fn export_json_page_value(
    conn: &Connection,
    limit: usize,
    memories_offset: usize,
    decisions_offset: usize,
) -> Result<Value, String> {
    let limit = limit.clamp(1, MAX_EXPORT_PAGE_LIMIT);
    let (memories, memories_has_more) = query_table_json_page(
        conn,
        &format!("SELECT id, text, source, type, tags, {PAGE_META} FROM memories {PAGE_TAIL}"),
        limit,
        memories_offset,
    )?;
    let (decisions, decisions_has_more) = query_table_json_page(
        conn,
        &format!("SELECT id, decision, context, type, {PAGE_META} FROM decisions {PAGE_TAIL}"),
        limit,
        decisions_offset,
    )?;
    Ok(
        json!({"version":1,"mode":"page","exported_at":now_iso(),"limit":limit,"memories_offset":memories_offset,"decisions_offset":decisions_offset,"next_memories_offset":next_page_offset(memories_offset,&memories,memories_has_more),"next_decisions_offset":next_page_offset(decisions_offset,&decisions,decisions_has_more),"truncated":memories_has_more||decisions_has_more,"memories":memories,"decisions":decisions,"memories_count":memories.len(),"decisions_count":decisions.len(),}),
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
        &format!("SELECT id, text, source, {CHANGESET_META} FROM memories {CHANGESET_TAIL}"),
        &[&lower, &cursor],
    )?;
    let decisions = query_rows_json(
        conn,
        &format!("SELECT id, decision, context, {CHANGESET_META} FROM decisions {CHANGESET_TAIL}"),
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
            let context = d.context.as_deref().map(crate::handlers::redact_secrets);
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
    let memories_has_acl = has_acl(conn, "memories")?;
    let decisions_has_acl = has_acl(conn, "decisions")?;
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
            let (agent, client) = agent_client(
                m.source_agent.as_deref(),
                m.source_client.as_deref(),
                fallback,
            );
            let inserted = if memories_has_acl {
                tx.execute(
                    &import_insert_sql(MEMORIES_INSERT, true, 16),
                    params![
                        m.text,
                        m.source,
                        entry_type,
                        m.tags,
                        agent,
                        client,
                        m.source_model.as_deref(),
                        m.confidence.unwrap_or(0.8),
                        m.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),
                        m.score.unwrap_or(1.0),
                        m.retention_class.unwrap_or_default().as_str(),
                        observed_at,
                        valid_from,
                        valid_until,
                        options.owner_id,
                        visibility
                    ],
                )
            } else {
                tx.execute(
                    &import_insert_sql(MEMORIES_INSERT, false, 16),
                    params![
                        m.text,
                        m.source,
                        entry_type,
                        m.tags,
                        agent,
                        client,
                        m.source_model.as_deref(),
                        m.confidence.unwrap_or(0.8),
                        m.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),
                        m.score.unwrap_or(1.0),
                        m.retention_class.unwrap_or_default().as_str(),
                        observed_at,
                        valid_from,
                        valid_until
                    ],
                )
            };
            imported(inserted, "memories", idx)?;
            counts.memories += 1;
        }
    }
    if let Some(decisions) = &payload.decisions {
        for (idx, d) in decisions.iter().enumerate() {
            let entry_type = normalize_decision_entry_type(d.entry_type.as_deref());
            let observed_at = optional_time(d.observed_at.as_deref());
            let valid_from = optional_time(d.valid_from.as_deref());
            let valid_until = optional_time(d.valid_until.as_deref());
            let (agent, client) = agent_client(
                d.source_agent.as_deref(),
                d.source_client.as_deref(),
                fallback,
            );
            let inserted = if decisions_has_acl {
                tx.execute(
                    &import_insert_sql(DECISIONS_INSERT, true, 15),
                    params![
                        d.decision,
                        d.context,
                        entry_type,
                        agent,
                        client,
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
                        d.retention_class.unwrap_or_default().as_str(),
                        observed_at,
                        valid_from,
                        valid_until,
                        options.owner_id,
                        visibility
                    ],
                )
            } else {
                tx.execute(
                    &import_insert_sql(DECISIONS_INSERT, false, 15),
                    params![
                        d.decision,
                        d.context,
                        entry_type,
                        agent,
                        client,
                        d.source_model.as_deref(),
                        d.confidence.unwrap_or(0.8),
                        d.reasoning_depth.as_deref().unwrap_or("single-shot"),
                        d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),
                        d.score.unwrap_or(1.0),
                        d.retention_class.unwrap_or_default().as_str(),
                        observed_at,
                        valid_from,
                        valid_until
                    ],
                )
            };
            imported(inserted, "decisions", idx)?;
            counts.decisions += 1;
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
    let fetch_limit =
        i64::try_from(limit.saturating_add(1)).map_err(|_| "export_page_limit".to_string())?;
    let offset = i64::try_from(offset).map_err(|_| "export_page_offset".to_string())?;
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
    (has_more || !page.is_empty()).then(|| offset.saturating_add(page.len()))
}
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    if !table
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
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
