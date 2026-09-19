use super::*;
use crate::protocol::nonempty_str;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

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
    let record_id = format!("obligation:{}#{}", thread_slug(&thread_id), count + 1);
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
    conn.execute("INSERT INTO obligations (record_id, thread_id, state, predicate_json, verification_revision) VALUES (?1, ?2, 'proposed', ?3, NULL)", params![record_id, thread_id, predicate.to_string()])?;
    super::add_thread_member(conn, &thread_id, &record_id, "obligation")?;
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
            format!(
                "{from} -> verified_complete requires a checker receipt or authorized acceptance (use verify)"
            )
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
    let (state, registered): (String, String) = match conn.query_row(
        "SELECT state, predicate_json FROM obligations WHERE record_id = ?1",
        params![record_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ) {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(format!("unknown obligation {record_id}"));
        }
        Err(err) => return Err(err.to_string()),
    };
    let registered: Value = serde_json::from_str(&registered)
        .map_err(|e| format!("obligation {record_id} predicate_json is not valid JSON: {e}"))?;
    let expected = registered
        .get("predicate")
        .and_then(Value::as_str)
        .and_then(nonempty_str)
        .ok_or_else(|| format!("obligation {record_id} has no registered predicate"))?;
    if expected != predicate.trim() {
        return Err(format!(
            "checker predicate `{predicate}` does not match the registered predicate `{expected}`"
        ));
    }
    if !passed {
        return Err(format!(
            "checker `{checker}` reported failure for `{predicate}` on {artifact}; an obligation is not verified by a failing run"
        ));
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
    let revision = append_revision(conn, sequence, NewRevision { record_id, kind: "obligation", retention: "durable", body: json!({"state": "verified_complete", "from": state, "verification": {"predicate": predicate, "artifact": artifact, "checker": checker, "authority": authority}}), epistemic_status: "checker_verified", parents: &parents, replace_parents: true, representation_version: "obligation/1" }).map_err(|e| e.to_string())?;
    conn.execute("UPDATE obligations SET state = 'verified_complete', verification_revision = ?1 WHERE record_id = ?2", params![revision, record_id]).map_err(|e| e.to_string())?;
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
    let verification: Option<String> = conn.query_row("SELECT verification_revision FROM obligations WHERE record_id = ?1 AND state = 'verified_complete'", params![record_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?.flatten();
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
    let record_id = format!("attempt:{}#{}", thread_slug(&thread_id), count + 1);
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
    let status = if json_i64(&attempt["exit_status"]).is_some_and(|s| s != 0)
        || attempt["outcome"]
            .as_str()
            .and_then(crate::protocol::nonempty_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("failure"))
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
    super::add_thread_member(conn, &thread_id, &record_id, "attempt")?;
    Ok(record_id)
}
