pub use crate::api_types::{ImportCounts, ImportOptions, ImportPayload};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub const DEFAULT_EXPORT_PAGE_LIMIT: usize = 1000;
pub const MAX_EXPORT_PAGE_LIMIT: usize = 5000;
fn normalize_entry_type(raw: Option<&str>, default: &str, aliases: &[(&[&str], &str)]) -> String {
    let normalized = raw.map(str::trim).filter(|value| !value.is_empty()).map(str::to_ascii_lowercase).unwrap_or_else(|| default.to_string());
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
            (&["memory", "note", "finding", "observation", "fact"], "fact"),
            (&["episode", "event"], "episode"),
            (&["procedure", "playbook", "runbook", "howto", "how-to"], "procedure"),
            (&["evidence", "citation", "reference"], "evidence"),
            (&["decision", "policy", "rule"], "decision"),
        ],
    )
}
fn normalize_decision_entry_type(raw: Option<&str>) -> String {
    normalize_entry_type(
        raw,
        "decision",
        &[
            (&["decision", "policy", "rule"], "decision"),
            (&["procedure", "playbook", "runbook"], "procedure"),
            (&["evidence", "citation", "reference"], "evidence"),
            (&["fact", "memory", "note"], "fact"),
        ],
    )
}
pub fn export_json_page_value(conn: &Connection, limit: usize, memories_offset: usize, decisions_offset: usize) -> Value {
    let limit = limit.clamp(1, MAX_EXPORT_PAGE_LIMIT);
    let(memories,memories_has_more)=
query_table_json_page(conn,
"SELECT id, text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, retention_class, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM memories WHERE status = 'active' ORDER BY id LIMIT ?1 OFFSET ?2"
,limit,memories_offset,);
    let(decisions,decisions_has_more)=query_table_json_page(conn,
"SELECT id, decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, retention_class, status, score, \
         retrievals, pinned, observed_at, valid_from, valid_until, created_at, updated_at FROM decisions WHERE status = 'active' ORDER BY id LIMIT ?1 OFFSET ?2"
,limit,decisions_offset,);
    json!({"version":1,"mode":"page","exported_at":now_iso(),"limit":limit,"memories_offset":memories_offset
,"decisions_offset":decisions_offset,"next_memories_offset":if memories_has_more{Some(memories_offset.saturating_add(memories.len(
)))}else{None::<usize>},"next_decisions_offset":if decisions_has_more{Some(decisions_offset.saturating_add(decisions.len()))}else{
None::<usize>},"truncated":memories_has_more||decisions_has_more,"memories":memories,"decisions":decisions,"memories_count":
memories.len(),"decisions_count":decisions.len(),})
}
pub fn export_json_changeset_value(conn: &Connection, since: Option<&str>) -> Value {
    let cursor = now_iso();
    let lower = since.unwrap_or("0000-00-00T00:00:00Z");
    let memories = query_rows_json(
        conn,
        "SELECT id, text, source, type, status, created_at, updated_at FROM memories WHERE status = 'active' AND updated_at > ?1 AND updated_at <= ?2 ORDER BY id",
        &[&lower, &cursor],
    );
    let decisions = query_rows_json(
        conn,
        "SELECT id, decision, context, type, status, created_at, updated_at FROM decisions WHERE status = 'active' AND updated_at > ?1 AND updated_at <= ?2 ORDER BY id",
        &[&lower, &cursor],
    );
    json!({"version":1,"mode":"changeset","cursor":cursor,"since":since,"memories":memories,"decisions":decisions})
}
pub fn import_payload(conn: &mut Connection, payload: &ImportPayload, options: &ImportOptions) -> Result<ImportCounts, String> {
    let mut counts = ImportCounts::default();
    let visibility = options.visibility.as_deref().unwrap_or("private");
    let fallback = options.source_agent_fallback.as_str();
    let memories_has_owner = column_exists(conn, "memories", "owner_id");
    let memories_has_visibility = column_exists(conn, "memories", "visibility");
    let decisions_has_owner = column_exists(conn, "decisions", "owner_id");
    let decisions_has_visibility = column_exists(conn, "decisions", "visibility");
    let tx = conn.transaction().map_err(|e| format!("failed to start import transaction: {e}"))?;
    if let Some(memories) = &payload.memories {
        for (idx, m) in memories.iter().enumerate() {
            let entry_type = normalize_memory_entry_type(m.entry_type.as_deref());
            let inserted = if memories_has_owner && memories_has_visibility {
                tx.
execute(
"INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14, ?15, ?16, ?17)"
,params![m.text,m.source,entry_type,m.tags,m.source_agent.as_deref().unwrap_or(fallback),m.source_client.as_deref().unwrap_or(m.
source_agent.as_deref().unwrap_or(fallback)),m.source_model.as_deref(),m.confidence.unwrap_or(0.8),m.reasoning_depth.as_deref().
unwrap_or("single-shot"),m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),m.score.unwrap_or(1.0),m.retention_class.
unwrap_or_default().as_str(),m.observed_at.as_deref(),m.valid_from.as_deref(),m.valid_until.as_deref(),options.owner_id,visibility
,],)
            } else {
                tx.execute(
"INSERT INTO memories (text, source, type, tags, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14, ?15)"
,params![m.text,m.source,entry_type,m.tags,m.source_agent.as_deref().unwrap_or(fallback),m.source_client.as_deref().unwrap_or(m.
source_agent.as_deref().unwrap_or(fallback)),m.source_model.as_deref(),m.confidence.unwrap_or(0.8),m.reasoning_depth.as_deref().
unwrap_or("single-shot"),m.trust_score.unwrap_or(m.confidence.unwrap_or(0.8)),m.score.unwrap_or(1.0),m.retention_class.
unwrap_or_default().as_str(),m.observed_at.as_deref(),m.valid_from.as_deref(),m.valid_until.as_deref(),],)
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
            let inserted = if decisions_has_owner && decisions_has_visibility {
                tx.execute(
"INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until, owner_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, ?14, ?15, ?16)"
,params![d.decision,d.context,entry_type,d.source_agent.as_deref().unwrap_or(fallback),d.source_client.as_deref().unwrap_or(d.
source_agent.as_deref().unwrap_or(fallback)),d.source_model.as_deref(),d.confidence.unwrap_or(0.8),d.reasoning_depth.as_deref().
unwrap_or("single-shot"),d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),d.score.unwrap_or(1.0),d.retention_class.
unwrap_or_default().as_str(),d.observed_at.as_deref(),d.valid_from.as_deref(),d.valid_until.as_deref(),options.owner_id,visibility
,],)
            } else {
                tx.execute(
"INSERT INTO decisions (decision, context, type, source_agent, source_client, source_model, confidence, reasoning_depth, trust_score, score, retention_class, status, observed_at, valid_from, valid_until)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active', ?12, ?13, ?14)"
,params![d.decision,d.context,entry_type,d.source_agent.as_deref().unwrap_or(fallback),d.source_client.as_deref().unwrap_or(d.
source_agent.as_deref().unwrap_or(fallback)),d.source_model.as_deref(),d.confidence.unwrap_or(0.8),d.reasoning_depth.as_deref().
unwrap_or("single-shot"),d.trust_score.unwrap_or(d.confidence.unwrap_or(0.8)),d.score.unwrap_or(1.0),d.retention_class.
unwrap_or_default().as_str(),d.observed_at.as_deref(),d.valid_from.as_deref(),d.valid_until.as_deref(),],)
            };
            match inserted {
                Ok(_) => counts.decisions += 1,
                Err(e) => return Err(format!("failed to import decisions[{idx}]: {e}")),
            }
        }
    }
    tx.commit().map_err(|e| format!("failed to commit import transaction: {e}"))?;
    Ok(counts)
}
fn row_to_json(row: &rusqlite::Row<'_>, column_names: &[String]) -> rusqlite::Result<Value> {
    let mut obj = serde_json::Map::new();
    for (i, name) in column_names.iter().enumerate() {
        let val: Value = match row.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Null) => Value::Null,
            Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
            Ok(rusqlite::types::ValueRef::Real(f)) => json!(f),
            Ok(rusqlite::types::ValueRef::Text(s)) => json!(std::str::from_utf8(s).unwrap_or("")),
            Ok(rusqlite::types::ValueRef::Blob(_)) => Value::Null,
            Err(_) => Value::Null,
        };
        obj.insert(name.clone(), val);
    }
    Ok(Value::Object(obj))
}
fn query_rows_json(conn: &Connection, sql: &str, bind: &[&dyn rusqlite::types::ToSql]) -> Vec<Value> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let column_names: Vec<String> = (0..stmt.column_count()).map(|i| stmt.column_name(i).unwrap_or("?").to_string()).collect();
    stmt.query_map(bind, |row| row_to_json(row, &column_names)).ok().into_iter().flatten().filter_map(|r| r.ok()).collect()
}
fn query_table_json_page(conn: &Connection, sql: &str, limit: usize, offset: usize) -> (Vec<Value>, bool) {
    let fetch_limit = limit.saturating_add(1) as i64;
    let offset = offset as i64;
    let mut rows = query_rows_json(conn, sql, &[&fetch_limit, &offset]);
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    (rows, has_more)
}
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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
mod tests;
