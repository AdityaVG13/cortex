//! Scoped exact any-cue subscriptions over attributed observations.
//! Derived postings are discardable; every read revalidates authority and source availability.
use super::observation;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
mod materialize;
mod needs;
mod runtime;
pub(super) use materialize::*;
pub(super) use needs::*;

const DDL: &str = "CREATE TABLE IF NOT EXISTS observation_projection(source_id TEXT PRIMARY KEY, status TEXT NOT NULL); CREATE TABLE IF NOT EXISTS observation_postings(principal TEXT NOT NULL, scope TEXT NOT NULL, cue TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(principal,scope,cue,source_id)); CREATE TABLE IF NOT EXISTS observation_needs(principal TEXT NOT NULL, need_id TEXT NOT NULL, scope TEXT NOT NULL, spec_json TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(principal,need_id)); CREATE TABLE IF NOT EXISTS observation_need_cues(principal TEXT NOT NULL, need_id TEXT NOT NULL, scope TEXT NOT NULL, cue TEXT NOT NULL, PRIMARY KEY(principal,need_id,cue)); CREATE INDEX IF NOT EXISTS observation_reverse_cues ON observation_need_cues(principal,scope,cue,need_id); CREATE TABLE IF NOT EXISTS observation_matches(principal TEXT NOT NULL, need_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(principal,need_id,source_id)); CREATE TABLE IF NOT EXISTS observation_retractions(source_id TEXT PRIMARY KEY, principal TEXT NOT NULL, reason TEXT NOT NULL); CREATE TABLE IF NOT EXISTS observation_deliveries(delivery_id TEXT PRIMARY KEY, principal TEXT NOT NULL, need_id TEXT NOT NULL, context TEXT NOT NULL, fingerprint TEXT NOT NULL, restore_epoch TEXT NOT NULL, expires INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS observation_requirements(principal TEXT NOT NULL, parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY(principal,parent_id,child_id));";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NeedSpec {
    pub id: String,
    pub scope: String,
    pub cues: Vec<String>,
    #[serde(default)]
    pub exclude_cues: Vec<String>,
    pub max_results: usize,
    pub max_bytes: usize,
    pub ttl_seconds: u64,
    #[serde(default)]
    pub learned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub source_id: String,
    pub revision_id: String,
    pub source_key: String,
    pub role: String,
    pub text: String,
    pub route: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedView {
    pub status: String,
    pub evidence: Vec<Evidence>,
    pub source_refs: Vec<String>,
    pub delivery_id: Option<String>,
    pub payload: String,
    pub payload_bytes: usize,
    pub projection_pending: usize,
    pub restore_epoch: String,
    pub policy_epoch: String,
    pub fingerprint: String,
    #[serde(default)]
    pub assembly_brief: String,
}

pub(super) fn ensure(conn: &Connection) -> Result<(), String> {
    // Runtime open owns authoritative migrations. Initialized observation reads
    // must not acquire a writer lock just to repeat idempotent schema writes.
    let tables:i64=conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('observation_sources','observation_events','observation_cursors','capture_policy','observation_projection','observation_postings','observation_needs','observation_need_cues','observation_matches','observation_retractions','observation_deliveries','observation_requirements')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if tables == 12 {
        return Ok(());
    }
    observation::ensure(conn)?;
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

pub(super) fn cues(text: &str) -> BTreeSet<String> {
    crate::handlers::alnum_underscore_tokens(text)
        .filter(|s| (3..=64).contains(&s.len()))
        .map(str::to_lowercase)
        .filter(|s| !is_observation_stop(s))
        .collect()
}

pub(super) fn is_observation_stop(token: &str) -> bool {
    const WORDS: &[&str] = &[
        "and", "are", "been", "but", "for", "from", "had", "has", "have", "into", "its", "not",
        "our", "than", "that", "the", "their", "them", "then", "they", "this", "use", "used",
        "using", "was", "were", "with", "you", "your",
    ];
    crate::handlers::sorted_has(WORDS, token)
}

pub(super) fn evidence_from_parts(
    source_id: String,
    revision_id: String,
    source_key: String,
    role: String,
    bytes: Vec<u8>,
    route: &str,
) -> Result<Evidence, String> {
    Ok(Evidence {
        source_id,
        revision_id,
        source_key,
        role,
        text: String::from_utf8(bytes).map_err(|_| "source_not_utf8")?,
        route: route.into(),
    })
}

pub(super) fn load_evidence(
    conn: &Connection,
    principal: &str,
    scope: &str,
    source_id: &str,
    route: &str,
) -> Result<Option<Evidence>, String> {
    let row = conn.query_row(&format!("SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM {} JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE e.principal=?1 AND e.source_id=?2 AND g.scope_label=?3 AND g.enabled=1 AND g.policy_epoch=({}) AND g.role!='delivery_only' AND {} AND s.availability='owned_inline'", observation::EVENT_GRANT_JOIN, crate::db::records::POLICY_EPOCH_SELECT, observation::LIVE_HEAD_SQL), params![principal, source_id, scope], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Vec<u8>>(4)?))).optional().map_err(|e| e.to_string())?;
    row.map(|(source_id, revision_id, source_key, role, bytes)| {
        evidence_from_parts(source_id, revision_id, source_key, role, bytes, route)
    })
    .transpose()
}

pub(super) fn close_evidence(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
    mut evidence: Vec<Evidence>,
) -> Result<(Vec<Evidence>, Option<&'static str>), String> {
    if !spec.exclude_cues.is_empty() {
        let banned: BTreeSet<_> = spec.exclude_cues.iter().flat_map(|c| cues(c)).collect();
        evidence.retain(|item| cues(&item.text).is_disjoint(&banned));
    }
    let mut seen: BTreeSet<String> = evidence.iter().map(|item| item.source_id.clone()).collect();
    let mut todo: Vec<String> = seen.iter().cloned().collect();
    if seen.len() > 128 {
        return Ok((Vec::new(), Some("closure_limit")));
    }
    while let Some(parent) = todo.pop() {
        // Fetch one extra row so a star of 129+ required children cannot silently omit a child and still claim ready.
        let mut stmt = conn.prepare("SELECT child_id FROM observation_requirements WHERE principal=?1 AND parent_id=?2 ORDER BY child_id LIMIT 129").map_err(|e| e.to_string())?;
        let children = stmt
            .query_map(params![principal, parent], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);
        for child in children {
            if !seen.insert(child.clone()) {
                continue;
            }
            // The previous check sat *before* this parent's children were
            // loaded, so one parent with >128 required children blew past
            // the closure cap and still returned a ready-looking bundle.
            if seen.len() > 128 {
                return Ok((Vec::new(), Some("closure_limit")));
            }
            match load_evidence(conn, principal, &spec.scope, &child, "required")? {
                Some(item) => {
                    todo.push(child);
                    evidence.push(item);
                }
                None => return Ok((Vec::new(), Some("qualification_unavailable"))),
            }
        }
    }
    Ok((evidence, None))
}
