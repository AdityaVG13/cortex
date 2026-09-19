use super::*;
use rusqlite::Connection;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct AsOfRecallItem {
    pub excerpt: String,
    pub source: String,
    pub relevance: f64,
    pub method: String,
    pub why: Value,
    pub status: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}
impl AsOfRecallItem {
    pub fn as_json(&self, as_of: &str) -> Value {
        json!({"excerpt":self.excerpt,"source":self.source,"relevance":self.relevance,"method":self.method,"status":self.status.clone().unwrap_or_else(|| "historical".to_string()),"validFrom":self.valid_from,"validUntil":self.valid_until,"asOf":as_of,"why":self.why})
    }
}
pub fn normalize_as_of_timestamp(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("as-of timestamp is empty".to_string());
    }
    if chrono::DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Ok(trimmed.to_string());
    }
    if chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S").is_ok() {
        return Ok(trimmed.to_string());
    }
    if chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d").is_ok() {
        return Ok(format!("{trimmed} 00:00:00"));
    }
    Err(format!("invalid as-of timestamp: {trimmed}"))
}
pub fn run_as_of_recall(
    conn: &Connection,
    query_text: &str,
    as_of: &str,
    k: usize,
    ctx: &RecallContext,
) -> Result<Vec<AsOfRecallItem>, String> {
    let mut ctx = ctx.clone();
    ctx.as_of = Some(as_of.to_string());
    let items = run_clock_quorum_recall(conn, query_text, 0, k, &ctx, None)?;
    Ok(items
        .into_iter()
        .map(|item| AsOfRecallItem {
            excerpt: item.excerpt.clone(),
            source: item.source.clone(),
            relevance: item.relevance,
            method: item.method.clone(),
            why: item.why_json(),
            status: item.status.clone(),
            valid_from: item.valid_from.clone(),
            valid_until: item.valid_until.clone(),
        })
        .collect())
}
