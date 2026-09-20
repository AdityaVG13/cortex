use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

const MAX_BOOST: f64 = 0.3;
const MIN_BOOST: f64 = -0.2;
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;
pub const IMMUNITY_THRESHOLD: i64 = 5;
pub const IMMUNITY_WINDOW_DAYS: i64 = 14;
pub fn compute_boosts(
    conn: &Connection,
    sources: &[String],
    query_vector: Option<&[f32]>,
) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    if sources.is_empty() {
        return boosts;
    }
    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;
    let placeholders = crate::handlers::recall::numbered_placeholders(1, sources.len());
    let sql = format!(
        "SELECT result_source, signal, julianday('now') - julianday(created_at) AS age_days FROM recall_feedback WHERE result_source IN ({placeholders})"
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::types::ToSql> = sources
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
            let source: String = row.get(0)?;
            let signal: f64 = row.get(1)?;
            let age_days: f64 = row.get::<_, f64>(2)?.max(0.0);
            Ok((
                source,
                signal
                    * query_similarity_weight(query_vector, None)
                    * (-decay_lambda * age_days).exp(),
            ))
        }) {
            for row in rows.flatten() {
                *boosts.entry(row.0).or_insert(0.0) += row.1;
            }
        }
    }
    for v in boosts.values_mut() {
        *v = v.clamp(MIN_BOOST, MAX_BOOST);
    }
    boosts
}
fn query_similarity_weight(_current_query: Option<&[f32]>, _stored_blob: Option<&[u8]>) -> f64 {
    1.0
}
pub fn has_retrieval_immunity(conn: &Connection, source: &str) -> Result<bool, String> {
    Ok(crate::db::count_sql(
        conn,
        "SELECT COUNT(*) FROM recall_feedback WHERE result_source = ?1 AND signal > 0 AND julianday('now') - julianday(created_at) <= ?2",
        params![source, IMMUNITY_WINDOW_DAYS],
    )? >= IMMUNITY_THRESHOLD)
}
/// Loop 1: turn a reported outcome into recall signals on the used
/// sources. Success/partial credit what was used; task failure alone never
/// indicts a source; harmful reuse penalizes. Returns the rows written.
/// Missing query text (or no usable sources) writes nothing — the outcome
/// row the caller already recorded is unaffected.
pub fn record_use_signals(
    conn: &Connection,
    agent: &str,
    outcome: &str,
    used: &[String],
    harmful_reuse: bool,
    explicit_query: Option<&str>,
    receipt: Option<&str>,
) -> Result<usize, String> {
    const MAX_SOURCES: usize = 64;
    const MAX_QUERY_CHARS: usize = 512;
    let signal = if harmful_reuse {
        -1.0
    } else {
        match outcome {
            "success" => 1.0,
            "partial" => 0.5,
            _ => return Ok(0),
        }
    };
    if used.is_empty() {
        return Ok(0);
    }
    let query_text = match resolve_query_text(conn, explicit_query, receipt, MAX_QUERY_CHARS)? {
        Some(text) => text,
        None => return Ok(0),
    };
    let frame = crate::clockwork::parse_query_frame(
        &query_text,
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
    );
    let signature = crate::clockwork::query_signature(&frame);
    let mut written = 0;
    for source in used.iter().take(MAX_SOURCES) {
        let source = source.trim();
        if source.is_empty() || source.len() > 256 {
            continue;
        }
        let (kind, id) = parse_source(source);
        conn.execute(
            "INSERT INTO recall_feedback (query_text, query_signature, result_source, result_type, result_id, signal, agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![query_text, signature, source, kind, id, signal, agent],
        )
        .map_err(|e| e.to_string())?;
        written += 1;
    }
    Ok(written)
}

fn resolve_query_text(
    conn: &Connection,
    explicit: Option<&str>,
    receipt: Option<&str>,
    max_chars: usize,
) -> Result<Option<String>, String> {
    if let Some(text) = explicit.map(str::trim).filter(|t| !t.is_empty()) {
        return Ok(Some(truncate_chars(text, max_chars)));
    }
    let Some(receipt) = receipt.filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let stored: Option<String> = conn
        .query_row(
            "SELECT receipt_json FROM view_receipts WHERE receipt_id = ?1",
            [receipt],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let need = serde_json::from_str::<serde_json::Value>(&stored)
        .ok()
        .and_then(|v| v.get("need").and_then(Value::as_str).map(str::to_string))
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    Ok(need.map(|n| truncate_chars(&n, max_chars)))
}

/// Resolve the asking need behind a view receipt, if the receipt recorded
/// one. Shared by signal recording and query-memory outcome attach.
pub fn query_text_for_receipt(conn: &Connection, receipt: &str) -> Option<String> {
    resolve_query_text(conn, None, Some(receipt), 512).unwrap_or(None)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

pub fn parse_source(source: &str) -> (String, Option<i64>) {
    [("decision::", "decision"), ("memory::", "memory")]
        .into_iter()
        .find_map(|(prefix, kind)| {
            source
                .strip_prefix(prefix)
                .map(|rest| (kind.to_string(), rest.parse().ok()))
        })
        .unwrap_or_else(|| ("unknown".to_string(), None))
}
