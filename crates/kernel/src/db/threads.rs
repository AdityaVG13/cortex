//! Threads, obligations and attempts over the authoritative tables.
//!
//! An obligation's workflow state is orthogonal to retention and epistemic
//! state. `verified_complete` is reachable only through a checker receipt or
//! authorized acceptance naming the predicate and the artifact it was checked
//! on; a later artifact revision reopens it without erasing the earlier
//! result. Timeouts and disconnects prove nothing and are not transitions.

use super::records::{append_revision, heads, revision_body, NewRevision, DEFAULT_SCOPE};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

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
    if label.starts_with("thread:") {
        return label.to_string();
    }
    format!(
        "thread:{}",
        label
            .trim()
            .to_ascii_lowercase()
            .replace(char::is_whitespace, "-")
    )
}

pub fn ensure_thread(conn: &Connection, sequence: i64, label: &str) -> rusqlite::Result<String> {
    let thread_id = thread_id_for(label);
    conn.execute(
        "INSERT OR IGNORE INTO threads (thread_id, scope_id, title, created_sequence) VALUES (?1, ?2, ?3, ?4)",
        params![thread_id, DEFAULT_SCOPE, label, sequence],
    )?;
    Ok(thread_id)
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

pub fn create_obligation(
    conn: &Connection,
    sequence: i64,
    thread_label: &str,
    title: &str,
    predicate: Value,
    agent: &str,
) -> rusqlite::Result<String> {
    let thread_id = ensure_thread(conn, sequence, thread_label)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM obligations WHERE thread_id = ?1",
        params![thread_id],
        |r| r.get(0),
    )?;
    let record_id = format!(
        "obligation:{}#{}",
        thread_id.trim_start_matches("thread:"),
        count + 1
    );
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id: &record_id,
            kind: "obligation",
            retention: "durable",
            body: json!({"title": title, "state": "proposed", "agent": agent}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "obligation/1",
        },
    )?;
    conn.execute(
        "INSERT INTO obligations (record_id, thread_id, state, predicate_json, verification_revision) VALUES (?1, ?2, 'proposed', ?3, NULL)",
        params![record_id, thread_id, predicate.to_string()],
    )?;
    conn.execute("INSERT OR IGNORE INTO thread_members (thread_id, record_id, role) VALUES (?1, ?2, 'obligation')", params![thread_id, record_id])?;
    Ok(record_id)
}

pub fn obligation_state(conn: &Connection, record_id: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT state FROM obligations WHERE record_id = ?1",
        params![record_id],
        |r| r.get(0),
    )
    .optional()
}

/// Workflow transition by a contributor. Records a revision so the history
/// of claims is preserved.
pub fn transition_obligation(
    conn: &Connection,
    sequence: i64,
    record_id: &str,
    to: &str,
    agent: &str,
    note: Option<&str>,
) -> Result<String, String> {
    if !OBLIGATION_STATES.contains(&to) {
        return Err(format!(
            "unknown obligation state `{to}`; known: {}",
            OBLIGATION_STATES.join(", ")
        ));
    }
    let from = obligation_state(conn, record_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("unknown obligation {record_id}"))?;
    if !transition_allowed(&from, to) {
        return Err(if to == "verified_complete" {
            format!("{from} -> verified_complete requires a checker receipt or authorized acceptance (use verify)")
        } else {
            format!("transition {from} -> {to} is not allowed")
        });
    }
    let parents = heads(conn, record_id).map_err(|e| e.to_string())?;
    let (kind, retention) = ("obligation", "durable");
    let revision = append_revision(
        conn,
        sequence,
        NewRevision {
            record_id,
            kind,
            retention,
            body: json!({"state": to, "from": from, "agent": agent, "note": note}),
            epistemic_status: "asserted",
            parents: &parents,
            replace_parents: true,
            representation_version: "obligation/1",
        },
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE obligations SET state = ?1 WHERE record_id = ?2",
        params![to, record_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(revision)
}

/// Verified completion: the named predicate must match the obligation's
/// registered predicate and name the artifact it was checked on. A later
/// artifact revision makes the verification stale (`reopen_if_artifact_changed`).
pub fn verify_obligation(
    conn: &Connection,
    sequence: i64,
    record_id: &str,
    predicate: &str,
    artifact: &str,
    checker: &str,
    passed: bool,
    authority: Option<&str>,
) -> Result<String, String> {
    let (state, registered): (String, String) = conn
        .query_row(
            "SELECT state, predicate_json FROM obligations WHERE record_id = ?1",
            params![record_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| format!("unknown obligation {record_id}"))?;
    let registered: Value = serde_json::from_str(&registered).map_err(|e| {
        format!("obligation {record_id} predicate_json is not valid JSON: {e}")
    })?;
    let expected = registered["predicate"].as_str().unwrap_or("");
    if !expected.is_empty() && expected != predicate {
        return Err(format!(
            "checker predicate `{predicate}` does not match the registered predicate `{expected}`"
        ));
    }
    if !passed {
        return Err(format!("checker `{checker}` reported failure for `{predicate}` on {artifact}; an obligation is not verified by a failing run"));
    }
    if !matches!(
        state.as_str(),
        "observed_complete" | "in_progress" | "reopened"
    ) && authority.is_none()
    {
        return Err(format!(
            "{state} -> verified_complete needs observed work or explicit authority"
        ));
    }
    let parents = heads(conn, record_id).map_err(|e| e.to_string())?;
    let revision = append_revision(
        conn,
        sequence,
        NewRevision {
            record_id,
            kind: "obligation",
            retention: "durable",
            body: json!({"state": "verified_complete", "from": state, "verification": {"predicate": predicate, "artifact": artifact, "checker": checker, "authority": authority}}),
            epistemic_status: "checker_verified",
            parents: &parents,
            replace_parents: true,
            representation_version: "obligation/1",
        },
    )
    .map_err(|e| e.to_string())?;
    conn.execute("UPDATE obligations SET state = 'verified_complete', verification_revision = ?1 WHERE record_id = ?2", params![revision, record_id])
        .map_err(|e| e.to_string())?;
    Ok(revision)
}

/// A verification is bound to its artifact: a new artifact revision reopens
/// the obligation without erasing the earlier verified result.
pub fn reopen_if_artifact_changed(
    conn: &Connection,
    sequence: i64,
    record_id: &str,
    current_artifact: &str,
    agent: &str,
) -> Result<Option<String>, String> {
    let verification: Option<String> = conn
        .query_row("SELECT verification_revision FROM obligations WHERE record_id = ?1 AND state = 'verified_complete'", params![record_id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let Some(verification) = verification else {
        return Ok(None);
    };
    let verified_artifact = revision_body(conn, &verification)
        .map_err(|e| e.to_string())?
        .and_then(|b| b["verification"]["artifact"].as_str().map(str::to_string))
        .unwrap_or_default();
    if verified_artifact == current_artifact {
        return Ok(None);
    }
    let note = format!("verified on {verified_artifact}; artifact is now {current_artifact}");
    transition_obligation(conn, sequence, record_id, "reopened", agent, Some(&note)).map(Some)
}

/// An attempt: obligation, inputs, environment, procedure, artifacts, exit
/// status and the contextual failure — never a reasoning transcript.
pub fn record_attempt(
    conn: &Connection,
    sequence: i64,
    thread_label: &str,
    obligation: Option<&str>,
    body: Value,
    agent: &str,
) -> rusqlite::Result<String> {
    let thread_id = ensure_thread(conn, sequence, thread_label)?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM thread_members WHERE thread_id = ?1 AND role = 'attempt'",
        params![thread_id],
        |r| r.get(0),
    )?;
    let record_id = format!(
        "attempt:{}#{}",
        thread_id.trim_start_matches("thread:"),
        count + 1
    );
    let mut attempt = json!({"agent": agent, "obligation": obligation});
    for key in [
        "inputs",
        "environment",
        "procedure",
        "artifacts",
        "exit_status",
        "checker",
        "failure",
        "outcome",
        "decision_points",
        "text",
    ] {
        if let Some(v) = body.get(key) {
            attempt[key] = v.clone();
        }
    }
    let status = if attempt["exit_status"]
        .as_i64()
        .map(|s| s != 0)
        .unwrap_or(false)
        || attempt["outcome"].as_str() == Some("failure")
    {
        "failure"
    } else {
        "outcome"
    };
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id: &record_id,
            kind: status,
            retention: "durable",
            body: attempt,
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "attempt/1",
        },
    )?;
    conn.execute("INSERT OR IGNORE INTO thread_members (thread_id, record_id, role) VALUES (?1, ?2, 'attempt')", params![thread_id, record_id])?;
    Ok(record_id)
}

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
        let title = head
            .as_ref()
            .and_then(|h| revision_body(conn, h).ok().flatten())
            .and_then(|b| b["title"].as_str().map(str::to_string))
            .or_else(|| {
                conn.query_row("SELECT body_json FROM revisions WHERE record_id = ?1 ORDER BY recorded_sequence ASC LIMIT 1", params![record_id], |r| {
                    r.get::<_, String>(0)
                })
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .and_then(|b| b["title"].as_str().map(str::to_string))
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
    let checkpoint = heads(conn, &checkpoint_record)?
        .first()
        .and_then(|h| revision_body(conn, h).ok().flatten());
    Ok(
        json!({"thread": thread_id, "exists": true, "obligations": obligations, "attempts": attempts, "checkpoint": checkpoint}),
    )
}
