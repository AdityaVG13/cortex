//! Threads, obligations and attempts over the authoritative tables.
//!
//! An obligation's workflow state is orthogonal to retention and epistemic
//! state. `verified_complete` is reachable only through a checker receipt or
//! authorized acceptance naming the predicate and the artifact it was checked
//! on; a later artifact revision reopens it without erasing the earlier
//! result. Timeouts and disconnects prove nothing and are not transitions.

use super::records::{DEFAULT_SCOPE, NewRevision, append_revision, heads, revision_body};
use crate::protocol::json_i64;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub const OBLIGATION_STATES: [&str; 8] = [
    "proposed",
    "ready",
    "in_progress",
    "blocked",
    "observed_complete",
    "verified_complete",
    "reopened",
    "cancelled",
];

pub fn thread_id_for(label: &str) -> String {
    let normalized = label
        .trim()
        .to_ascii_lowercase()
        .replace(char::is_whitespace, "-");
    let body = normalized.strip_prefix("thread:").unwrap_or(&normalized);
    format!("thread:{body}")
}

fn thread_slug(thread_id: &str) -> &str {
    thread_id.strip_prefix("thread:").unwrap_or(thread_id)
}

pub fn ensure_thread(conn: &Connection, sequence: i64, label: &str) -> rusqlite::Result<String> {
    let thread_id = thread_id_for(label);
    conn.execute("INSERT OR IGNORE INTO threads (thread_id, scope_id, title, created_sequence) VALUES (?1, ?2, ?3, ?4)", params![thread_id, DEFAULT_SCOPE, label, sequence])?;
    Ok(thread_id)
}

pub fn add_thread_member(
    conn: &Connection,
    thread_id: &str,
    record_id: &str,
    role: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_members (thread_id, record_id, role) VALUES (?1, ?2, ?3)",
        params![thread_id, record_id, role],
    )
}

/// Allowed workflow transitions. `verified_complete` is never a plain
/// transition; it requires `verify_obligation`.
pub fn transition_allowed(from: &str, to: &str) -> bool {
    match (from, to) {
        (_, "verified_complete") => false,
        ("cancelled", _) => false,
        ("proposed", "ready" | "in_progress" | "cancelled") => true,
        ("ready", "in_progress" | "blocked" | "cancelled") => true,
        ("in_progress", "blocked" | "observed_complete" | "ready" | "cancelled") => true,
        ("blocked", "ready" | "in_progress" | "cancelled") => true,
        ("observed_complete", "reopened" | "in_progress" | "cancelled") => true,
        ("verified_complete", "reopened") => true,
        ("reopened", "ready" | "in_progress" | "cancelled") => true,
        _ => false,
    }
}

mod obligations;
pub use obligations::*;

/// Thread state for continuation: obligations by state, attempts, checkpoint.
pub fn thread_summary(conn: &Connection, thread_label: &str) -> rusqlite::Result<Value> {
    let thread_id = thread_id_for(thread_label);
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE thread_id = ?1",
        params![thread_id],
        |r| r.get::<_, i64>(0),
    )? > 0;
    if !exists {
        return Ok(json!({"thread": thread_id, "exists": false}));
    }
    let mut obligations = Vec::new();
    let mut stmt = conn.prepare("SELECT o.record_id, o.state, o.verification_revision FROM obligations o WHERE o.thread_id = ?1 ORDER BY o.record_id")?;
    for row in stmt.query_map(params![thread_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (record_id, state, verification) = row?;
        let head = heads(conn, &record_id)?.first().cloned();
        let title = head.as_ref().and_then(|h| revision_body(conn, h).ok().flatten()).and_then(|b| b["title"].as_str().map(str::to_string)).or_else(|| {
                conn.query_row("SELECT body_json FROM revisions WHERE record_id = ?1 ORDER BY recorded_sequence ASC LIMIT 1", params![record_id], |r| r.get::<_, String>(0)).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).and_then(|b| b["title"].as_str().map(str::to_string))
            });
        obligations.push(json!({"record": record_id, "state": state, "title": title, "verification": verification}));
    }
    let mut attempts = Vec::new();
    let mut stmt = conn.prepare("SELECT record_id FROM thread_members WHERE thread_id = ?1 AND role = 'attempt' ORDER BY record_id")?;
    for row in stmt.query_map(params![thread_id], |r| r.get::<_, String>(0))? {
        let record_id = row?;
        if let Some(head) = heads(conn, &record_id)?.first() {
            let kind: String = conn.query_row(
                "SELECT kind FROM records WHERE record_id = ?1",
                params![record_id],
                |r| r.get(0),
            )?;
            attempts.push(
                json!({"record": record_id, "kind": kind, "body": revision_body(conn, head)?}),
            );
        }
    }
    let checkpoint_record = format!("checkpoint:{thread_id}");
    let checkpoint = match heads(conn, &checkpoint_record)?.first() {
        Some(h) => revision_body(conn, h)?,
        None => None,
    };
    Ok(
        json!({"thread": thread_id, "exists": true, "obligations": obligations, "attempts": attempts, "checkpoint": checkpoint}),
    )
}
