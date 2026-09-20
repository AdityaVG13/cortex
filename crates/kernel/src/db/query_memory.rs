//! Query memory (Loop 2): past queries as navigable structure.
//!
//! Every asked query persists its frame signature, terms, and strong
//! anchors; outcomes attach successes and the evidence used. Recall expands
//! new queries with terms from similar *successful* past queries —
//! spec-1 access aids through the existing expansion channel, never hard
//! anchors. Rows are principal-scoped: one principal's questions never
//! expand another's. Bounded by maintenance trim, never on the hot path.

use rusqlite::{Connection, params};

pub const DDL: &str = "CREATE TABLE IF NOT EXISTS query_memory (principal TEXT NOT NULL, signature TEXT NOT NULL, terms_json TEXT NOT NULL DEFAULT '[]', anchors_json TEXT NOT NULL DEFAULT '[]', asks INTEGER NOT NULL DEFAULT 0, successes INTEGER NOT NULL DEFAULT 0, last_evidence_json TEXT NOT NULL DEFAULT '[]', first_seen TEXT NOT NULL DEFAULT (datetime('now')), last_seen TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (principal, signature));";

pub const TRIM_KEEP_ROWS: i64 = 10_000;

pub fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

/// Record that `principal` asked `text`. Returns the frame signature, or
/// `None` for empty text. Upserts: repeat asks count, never duplicate.
pub fn record_ask(
    conn: &Connection,
    principal: &str,
    text: &str,
) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    ensure(conn)?;
    // Minimal frame: no session/owner context, so the signature is stable
    // across sessions. Principal isolation lives in the row key, not the
    // signature.
    let frame = crate::clockwork::parse_query_frame(
        text,
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
        None,
    );
    let signature = crate::clockwork::query_signature(&frame);
    let mut terms = frame.terms.clone();
    terms.sort();
    terms.dedup();
    let mut anchors: Vec<String> = frame
        .anchors
        .iter()
        .filter(|a| a.specificity >= 2)
        .map(|a| format!("{}:{}", a.kind.as_str(), a.value))
        .collect();
    anchors.sort();
    anchors.dedup();
    let terms_json = serde_json::to_string(&terms).map_err(|e| e.to_string())?;
    let anchors_json = serde_json::to_string(&anchors).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO query_memory (principal, signature, terms_json, anchors_json, asks) VALUES (?1, ?2, ?3, ?4, 1) ON CONFLICT (principal, signature) DO UPDATE SET asks = asks + 1, last_seen = datetime('now')",
        params![principal, signature, terms_json, anchors_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(signature))
}

/// Attach an outcome to a remembered query. No row, no-op: outcomes for
/// queries that never ran (explicit-query feedback) must `record_ask` first.
pub fn record_outcome(
    conn: &Connection,
    principal: &str,
    signature: &str,
    success: bool,
    evidence: &[String],
) -> Result<(), String> {
    ensure(conn)?;
    let evidence_json = serde_json::to_string(evidence).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE query_memory SET successes = successes + ?1, last_evidence_json = ?2, last_seen = datetime('now') WHERE principal = ?3 AND signature = ?4",
        params![if success { 1 } else { 0 }, evidence_json, principal, signature],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read path lives in `clockwork::bridge` (raw SQL beside the other
/// expansion sources). Maintenance-only trim below: keep the most recently
/// seen rows. Never called on
/// the query hot path.
pub fn trim(conn: &Connection, keep: i64) -> Result<usize, String> {
    ensure(conn)?;
    let deleted = conn
        .execute(
            "DELETE FROM query_memory WHERE rowid NOT IN (SELECT rowid FROM query_memory ORDER BY last_seen DESC, signature ASC LIMIT ?1)",
            params![keep.max(1)],
        )
        .map_err(|e| e.to_string())?;
    Ok(deleted)
}
