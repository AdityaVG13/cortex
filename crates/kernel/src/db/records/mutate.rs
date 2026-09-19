use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub fn append_commit(
    conn: &Connection,
    principal: &str,
    idempotency_key: Option<&str>,
    ack_profile: &str,
) -> rusqlite::Result<i64> {
    let (brain_id, _, _) = try_brain_epochs(conn)?;
    let counter: i64 = conn.query_row(
        "SELECT COALESCE(MAX(origin_counter), -1) + 1 FROM commits WHERE origin_id = ?1",
        params![brain_id],
        |r| r.get(0),
    )?;
    let commit_id = format!("{brain_id}:{counter}");
    conn.execute("INSERT INTO commits (commit_id, principal_id, idempotency_key, ack_profile, origin_id, origin_counter) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![commit_id, principal, idempotency_key, ack_profile, brain_id, counter])?;
    Ok(conn.last_insert_rowid())
}

pub fn ack_label(conn: &Connection) -> &'static str {
    crate::runtime::ack_profile_label_pub(&crate::store_spi::sqlite::ack_profile(conn))
}

pub fn append_ack_commit(conn: &Connection, principal: &str) -> Result<i64, String> {
    append_commit(conn, principal, None, ack_label(conn)).map_err(|e| e.to_string())
}

pub struct NewRevision<'a> {
    pub record_id: &'a str,
    pub kind: &'a str,
    pub retention: &'a str,
    pub body: Value,
    pub epistemic_status: &'a str,
    /// Parent revisions this revision descends from. Empty = baseline.
    pub parents: &'a [String],
    /// When true the parents are removed from the head set (a normal
    /// successor). When false the new revision is a concurrent head.
    pub replace_parents: bool,
    pub representation_version: &'a str,
}

/// Insert a record (if new) and an immutable revision, maintain the head set,
/// and append a change item. Never deletes a revision.
pub fn append_revision(
    conn: &Connection,
    sequence: i64,
    rev: NewRevision<'_>,
) -> rusqlite::Result<String> {
    conn.execute("INSERT OR IGNORE INTO records (record_id, scope_id, kind, retention, created_sequence) VALUES (?1, ?2, ?3, ?4, ?5)", params![rev.record_id, DEFAULT_SCOPE, rev.kind, rev.retention, sequence])?;
    let ordinal: i64 = conn.query_row(
        "SELECT COUNT(*) FROM revisions WHERE record_id = ?1",
        params![rev.record_id],
        |r| r.get(0),
    )?;
    let revision_id = format!("{}@{}", rev.record_id, ordinal + 1);
    conn.execute("INSERT INTO revisions (revision_id, record_id, body_json, epistemic_status, recorded_sequence, representation_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![revision_id, rev.record_id, rev.body.to_string(), rev.epistemic_status, sequence, rev.representation_version])?;
    for parent in rev.parents {
        conn.execute("INSERT INTO revision_parents (record_id, revision_id, parent_revision) VALUES (?1, ?2, ?3)", params![rev.record_id, revision_id, parent])?;
        if rev.replace_parents {
            conn.execute(
                "DELETE FROM record_heads WHERE record_id = ?1 AND revision_id = ?2",
                params![rev.record_id, parent],
            )?;
        }
    }
    conn.execute(
        "INSERT INTO record_heads (record_id, revision_id) VALUES (?1, ?2)",
        params![rev.record_id, revision_id],
    )?;
    // Negative-dependency guard: any cached read that searched this
    // record kind (or the scope at large) is invalidated by this insert.
    crate::db::compiled::bump_guard(conn, DEFAULT_SCOPE, rev.kind)?;
    let change_ordinal: i64 = conn.query_row(
        "SELECT COUNT(*) FROM change_items WHERE sequence = ?1",
        params![sequence],
        |r| r.get(0),
    )?;
    conn.execute("INSERT INTO change_items (sequence, ordinal, scope_id, record_id, change_kind) VALUES (?1, ?2, ?3, ?4, ?5)", params![sequence, change_ordinal, DEFAULT_SCOPE, rev.record_id, if rev.parents.is_empty() { "created" } else { "revised" }])?;
    Ok(revision_id)
}

pub fn heads(conn: &Connection, record_id: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT revision_id FROM record_heads WHERE record_id = ?1 ORDER BY revision_id",
    )?;
    let rows = stmt.query_map(params![record_id], |r| r.get::<_, String>(0))?;
    rows.collect()
}

pub fn revision_body(conn: &Connection, revision_id: &str) -> rusqlite::Result<Option<Value>> {
    let raw = conn
        .query_row(
            "SELECT body_json FROM revisions WHERE revision_id = ?1",
            params![revision_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    match raw {
        None => Ok(None),
        Some(s) => serde_json::from_str(&s).map(Some).map_err(|err| {
            rusqlite::Error::InvalidParameterName(format!(
                "revision `{revision_id}` body is not valid JSON: {err}"
            ))
        }),
    }
}

/// A resolution names the heads it considered, its authority and rationale;
/// it becomes the single head. Heads not listed stay unresolved.
pub fn resolve_heads(
    conn: &Connection,
    sequence: i64,
    record_id: &str,
    considered: &[String],
    authority: &str,
    rationale: &str,
    body: Value,
) -> rusqlite::Result<String> {
    let current = heads(conn, record_id)?;
    for head in considered {
        if !current.contains(head) {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "{head} is not a current head of {record_id}"
            )));
        }
    }
    let (kind, retention): (String, String) = conn.query_row(
        "SELECT kind, retention FROM records WHERE record_id = ?1",
        params![record_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let resolution_body = json!({"resolution": {"considered": considered, "authority": authority, "rationale": rationale}, "body": body});
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id,
            kind: &kind,
            retention: &retention,
            body: resolution_body,
            epistemic_status: "supported",
            parents: considered,
            replace_parents: true,
            representation_version: "resolution/1",
        },
    )
}

/// Legacy address of a `memories`/`decisions` row → record id, if imported.
pub fn record_for_legacy(
    conn: &Connection,
    table_kind: &str,
    id: i64,
) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT record_id FROM addresses WHERE scheme = 'legacy' AND namespace = ?1 AND address = ?2", params![table_kind, id.to_string()], |r| r.get(0)).optional()
}
