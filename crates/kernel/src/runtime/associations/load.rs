use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

pub(super) fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}
pub(super) fn exists(conn: &Connection, table: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn enabled(conn: &Connection, principal: &str, scope: &str) -> Result<bool, String> {
    // Missing state is not opted in. `rebuild_associations` writes enabled=1;
    // `reset_associations` writes 0. Default-true would let `learned: true`
    // refresh incidence without that rebuild, which the contract forbids.
    Ok(conn.query_row("SELECT enabled FROM observation_association_state WHERE principal=?1 AND scope_label=?2", params![principal, scope], |r| r.get::<_, bool>(0)).optional().map_err(|e| e.to_string())?.unwrap_or(false))
}
pub(super) fn tokens(text: &str) -> BTreeSet<String> {
    crate::handlers::alnum_underscore_tokens(text)
        .filter(|s| (3..=64).contains(&s.len()))
        .take(2048)
        .map(str::to_lowercase)
        .filter(|s| {
            !matches!(
                s.as_str(),
                "the" | "and" | "that" | "this" | "with" | "from" | "for" | "are" | "was" | "not"
            )
        })
        .take(MAX_TOKENS)
        .collect()
}
pub(super) struct Evidence {
    pub(super) id: String,
    pub(super) lineage: String,
    pub(super) digest: String,
    pub(super) tokens: BTreeSet<String>,
}

// Exact rows and CURRENT policy are always consulted, including for supporters.
// Unresolved agent/tool derivation is conservatively not independent evidence.
pub(super) fn evidence(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<Vec<Evidence>, String> {
    if !exists(conn, "observation_events")? {
        return Ok(Vec::new());
    }
    let retract = if exists(conn, "observation_retractions")? {
        " AND NOT EXISTS(SELECT 1 FROM observation_retractions x WHERE x.source_id=e.source_id)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT e.source_id,e.source_key,s.inline_payload FROM observation_events e JOIN observation_sources o ON o.principal=e.principal AND o.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id JOIN scopes sc ON sc.scope_id=s.scope_id JOIN revisions r ON r.revision_id=e.revision_id JOIN record_heads h ON h.revision_id=r.revision_id AND h.record_id=r.record_id WHERE e.principal=?1 AND o.scope_label=?2 AND sc.owner_id=?1 AND o.scope_id=s.scope_id AND o.enabled=1 AND o.policy_epoch=({}) AND o.role IN ('document','user_statement') AND json_extract(r.body_json,'$.role')=o.role AND r.epistemic_status!='retracted' AND s.availability='owned_inline' AND length(s.inline_payload)<=65536 AND COALESCE(({}), 'active')!='stopped' {retract} ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?3",
        crate::db::records::POLICY_EPOCH_SELECT,
        crate::db::capture_policy::scope_state_select_sql("?2")
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![principal, scope, MAX_SOURCES as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for row in rows {
        let (id, lineage, bytes) = row.map_err(|e| e.to_string())?;
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        result.push(Evidence {
            id,
            lineage,
            digest: crate::handlers::digest_hex(&bytes),
            tokens: tokens(text),
        });
    }
    Ok(result)
}

pub(super) fn ensure_feedback(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS observation_association_feedback(principal TEXT NOT NULL,scope_label TEXT NOT NULL,event_key TEXT NOT NULL,source_id TEXT NOT NULL,value INTEGER NOT NULL CHECK(value BETWEEN -1 AND 1),active INTEGER NOT NULL DEFAULT 1,PRIMARY KEY(principal,scope_label,event_key))").map_err(|e|e.to_string())
}
pub(crate) fn maintain_in_transaction(
    conn: &Connection,
    principal: &str,
    scope: &str,
) -> Result<usize, String> {
    ensure(conn)?;
    if !enabled(conn, principal, scope)? {
        return Ok(0);
    }
    refresh(conn, principal, scope)
}
/// Bounded refresh for prepare. Explicit reset is sticky until explicit rebuild.

pub(super) fn refresh(conn: &Connection, principal: &str, scope: &str) -> Result<usize, String> {
    let rows = evidence(conn, principal, scope)?;
    conn.execute(
        "DELETE FROM observation_association_incidence WHERE principal=?1 AND scope_label=?2",
        params![principal, scope],
    )
    .map_err(|e| e.to_string())?;
    for row in &rows {
        conn.execute(
            "INSERT INTO observation_association_incidence VALUES(?1,?2,?3,?4,?5)",
            params![
                principal,
                scope,
                row.id,
                row.digest,
                serde_json::to_string(&row.tokens).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(rows.len())
}
