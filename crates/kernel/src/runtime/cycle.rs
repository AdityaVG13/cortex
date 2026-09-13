//! Scoped exact any-cue subscriptions over attributed observations.
//! Derived postings are discardable; every read revalidates authority and source availability.
use super::{CortexRuntime, observation};
use asupersync::Cx;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS observation_projection(source_id TEXT PRIMARY KEY, status TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS observation_postings(principal TEXT NOT NULL, scope TEXT NOT NULL, cue TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(principal,scope,cue,source_id));
CREATE TABLE IF NOT EXISTS observation_needs(principal TEXT NOT NULL, need_id TEXT NOT NULL, scope TEXT NOT NULL, spec_json TEXT NOT NULL, expires INTEGER NOT NULL, PRIMARY KEY(principal,need_id));
CREATE TABLE IF NOT EXISTS observation_need_cues(principal TEXT NOT NULL, need_id TEXT NOT NULL, scope TEXT NOT NULL, cue TEXT NOT NULL, PRIMARY KEY(principal,need_id,cue));
CREATE INDEX IF NOT EXISTS observation_reverse_cues ON observation_need_cues(principal,scope,cue,need_id);
CREATE TABLE IF NOT EXISTS observation_matches(principal TEXT NOT NULL, need_id TEXT NOT NULL, source_id TEXT NOT NULL, PRIMARY KEY(principal,need_id,source_id));
CREATE TABLE IF NOT EXISTS observation_retractions(source_id TEXT PRIMARY KEY, principal TEXT NOT NULL, reason TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS observation_deliveries(delivery_id TEXT PRIMARY KEY, principal TEXT NOT NULL, need_id TEXT NOT NULL, context TEXT NOT NULL, fingerprint TEXT NOT NULL, restore_epoch TEXT NOT NULL, expires INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS observation_requirements(principal TEXT NOT NULL, parent_id TEXT NOT NULL, child_id TEXT NOT NULL, PRIMARY KEY(principal,parent_id,child_id));
"#;

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

fn ensure(conn: &Connection) -> Result<(), String> {
    // Runtime open owns authoritative migrations. Initialized observation reads
    // must not acquire a writer lock just to repeat idempotent schema writes.
    let tables:i64=conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('observation_sources','observation_events','observation_cursors','capture_policy','observation_projection','observation_postings','observation_needs','observation_need_cues','observation_matches','observation_retractions','observation_deliveries','observation_requirements')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if tables == 12 {
        return Ok(());
    }
    observation::ensure(conn)?;
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

fn cues(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn validate(spec: &NeedSpec) -> Result<BTreeSet<String>, String> {
    if spec.id.is_empty()
        || spec.id.len() > 256
        || spec.scope.is_empty()
        || spec.scope.len() > 1024
        || spec.cues.is_empty()
        || spec.cues.len() > 32
        || spec.cues.iter().any(|c| c.len() > 128)
        || spec.exclude_cues.len() > 32
        || spec.exclude_cues.iter().any(|c| c.len() > 128)
        || spec.max_results == 0
        || spec.max_results > 128
        || spec.max_bytes == 0
        || spec.max_bytes > 65536
        || spec.ttl_seconds == 0
        || spec.ttl_seconds > 86400
    {
        return Err("invalid_need_bounds".into());
    }
    let normalized: BTreeSet<_> = spec.cues.iter().flat_map(|c| cues(c)).collect();
    if normalized.is_empty() || normalized.len() > 32 {
        return Err("invalid_need_cues".into());
    }
    Ok(normalized)
}

pub(super) fn on_capture(conn: &Connection, principal: &str, source: &str) -> Result<(), String> {
    ensure(conn)?;
    let scope: String = conn
        .query_row(
            "SELECT scope_label FROM observation_sources WHERE principal=?1 AND source_key=?2",
            params![principal, source],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    project(conn, principal, &scope, 32)?;
    Ok(())
}
/// Bounded maintenance, not a MAX(sequence) completeness assumption.
fn project(conn: &Connection, principal: &str, scope: &str, limit: usize) -> Result<usize, String> {
    let mut stmt = conn.prepare("SELECT e.source_id,s.inline_payload FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id LEFT JOIN observation_projection p ON p.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND p.source_id IS NULL AND s.availability='owned_inline' AND g.enabled=1 AND g.policy_epoch=(SELECT policy_epoch FROM brain_meta WHERE singleton=1) AND g.role!='delivery_only' ORDER BY s.capture_sequence,e.source_id LIMIT ?3").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![principal, scope, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (source, bytes) in &rows {
        let text = std::str::from_utf8(bytes).map_err(|_| "source_not_utf8")?;
        let terms = cues(text);
        if terms.len() > 8192 {
            conn.execute(
                "INSERT INTO observation_projection VALUES(?1,'quota_blocked')",
                params![source],
            )
            .map_err(|e| e.to_string())?;
            continue;
        }
        for cue in terms {
            conn.execute(
                "INSERT OR IGNORE INTO observation_postings VALUES(?1,?2,?3,?4)",
                params![principal, scope, cue, source],
            )
            .map_err(|e| e.to_string())?;
            conn.execute("INSERT OR IGNORE INTO observation_matches SELECT principal,need_id,?4 FROM observation_need_cues WHERE principal=?1 AND scope=?2 AND cue=?3",params![principal,scope,cue,source]).map_err(|e| e.to_string())?;
        }
        conn.execute(
            "INSERT INTO observation_projection VALUES(?1,'ready')",
            params![source],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(rows.len())
}

fn register(conn: &Connection, principal: &str, spec: &NeedSpec) -> Result<(), String> {
    let mut spec = spec.clone();
    spec.scope = observation::normalize_scope(&spec.scope);
    let normalized = validate(&spec)?;
    let now = chrono::Utc::now().timestamp();
    conn.execute("DELETE FROM observation_need_cues WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND expires<=?2)",params![principal,now]).map_err(|e|e.to_string())?;
    conn.execute("DELETE FROM observation_matches WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND expires<=?2)",params![principal,now]).map_err(|e|e.to_string())?;
    conn.execute(
        "DELETE FROM observation_needs WHERE principal=?1 AND expires<=?2",
        params![principal, now],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM observation_deliveries WHERE principal=?1 AND expires<=?2",
        params![principal, now],
    )
    .map_err(|e| e.to_string())?;
    let count: i64 = conn.query_row("SELECT count(*) FROM observation_needs WHERE principal=?1 AND expires>?2 AND need_id!=?3",params![principal,now,spec.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count >= 128 {
        return Err("active_need_quota".into());
    }
    // Only derived subscription state is replaced, never source evidence.
    conn.execute(
        "DELETE FROM observation_need_cues WHERE principal=?1 AND need_id=?2",
        params![principal, spec.id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM observation_matches WHERE principal=?1 AND need_id=?2",
        params![principal, spec.id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO observation_needs VALUES(?1,?2,?3,?4,?5) ON CONFLICT(principal,need_id) DO UPDATE SET scope=excluded.scope,spec_json=excluded.spec_json,expires=excluded.expires",params![principal,spec.id,spec.scope,serde_json::to_string(&spec).map_err(|e|e.to_string())?,now+spec.ttl_seconds as i64]).map_err(|e|e.to_string())?;
    for cue in normalized {
        conn.execute(
            "INSERT INTO observation_need_cues VALUES(?1,?2,?3,?4)",
            params![principal, spec.id, spec.scope, cue],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("INSERT OR IGNORE INTO observation_matches SELECT principal,?2,source_id FROM observation_postings WHERE principal=?1 AND scope=?3 AND cue=?4",params![principal,spec.id,spec.scope,cue]).map_err(|e|e.to_string())?;
    }
    Ok(())
}

fn load_evidence(
    conn: &Connection,
    principal: &str,
    scope: &str,
    source_id: &str,
    route: &str,
) -> Result<Option<Evidence>, String> {
    let row = conn
        .query_row(
            "SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE e.principal=?1 AND e.source_id=?2 AND g.scope_label=?3 AND g.enabled=1 AND g.policy_epoch=(SELECT policy_epoch FROM brain_meta WHERE singleton=1) AND g.role!='delivery_only' AND EXISTS(SELECT 1 FROM record_heads h WHERE h.revision_id=e.revision_id) AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id) AND s.availability='owned_inline'",
            params![principal, source_id, scope],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(source_id, revision_id, source_key, role, bytes)| {
        Ok(Evidence {
            source_id,
            revision_id,
            source_key,
            role,
            text: String::from_utf8(bytes).map_err(|_| "source_not_utf8")?,
            route: route.into(),
        })
    })
    .transpose()
}

fn close_evidence(
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
    while let Some(parent) = todo.pop() {
        if seen.len() > 128 {
            return Ok((Vec::new(), Some("closure_limit")));
        }
        let mut stmt = conn
            .prepare(
                "SELECT child_id FROM observation_requirements WHERE principal=?1 AND parent_id=?2",
            )
            .map_err(|e| e.to_string())?;
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

fn materialize(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
) -> Result<PreparedView, String> {
    let (_, restore, policy) = crate::db::records::brain_epochs(conn);
    let capture_state: Option<String> = conn.query_row("SELECT state FROM capture_policy WHERE scope IN (?1,'*') ORDER BY CASE WHEN scope=?1 THEN 0 ELSE 1 END LIMIT 1",params![spec.scope],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if capture_state.as_deref() == Some("stopped") {
        return Err("capture_stopped".into());
    }
    let mut pending: i64 = conn.query_row("SELECT count(*) FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id LEFT JOIN observation_projection p ON p.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND g.role!='delivery_only' AND s.availability='owned_inline' AND (p.status IS NULL OR p.status!='ready') AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    pending += conn.query_row("SELECT count(*) FROM observation_sources g WHERE g.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND substr(g.source_key,1,5)='file:' AND NOT EXISTS(SELECT 1 FROM observation_events e WHERE e.principal=g.principal AND e.source_key=g.source_key)",params![principal,spec.scope,policy],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())?;
    let selected_cues = if spec.id.starts_with("pull:") {
        serde_json::to_string(&spec.cues).map_err(|e| e.to_string())?
    } else {
        "[]".into()
    };
    let mut stmt=conn.prepare("WITH candidates AS (SELECT principal,source_id FROM observation_matches WHERE principal=?1 AND need_id=?2 UNION SELECT p.principal,p.source_id FROM observation_postings p JOIN json_each(?6) q ON q.value=p.cue WHERE p.principal=?1 AND p.scope=?3) SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM candidates m JOIN observation_events e ON e.source_id=m.source_id AND e.principal=m.principal JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE m.principal=?1 AND g.scope_label=?3 AND g.enabled=1 AND g.policy_epoch=?4 AND g.role!='delivery_only' AND s.availability='owned_inline' AND EXISTS(SELECT 1 FROM record_heads h WHERE h.revision_id=e.revision_id) AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id) ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?5").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(
            params![
                principal,
                spec.id,
                spec.scope,
                policy,
                spec.max_results as i64 + 1,
                selected_cues
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut evidence = Vec::new();
    for (source_id, revision_id, source_key, role, bytes) in rows {
        evidence.push(Evidence {
            source_id,
            revision_id,
            source_key,
            role,
            text: String::from_utf8(bytes).map_err(|_| "source_not_utf8")?,
            route: "literal".into(),
        });
    }
    if spec.learned {
        super::associations::maintain_in_transaction(conn, principal, &spec.scope)?;
        for (source_id, _) in super::associations::candidates(
            conn,
            principal,
            &spec.scope,
            &spec.cues,
            spec.max_results,
        )? {
            if evidence.iter().any(|e| e.source_id == source_id) {
                continue;
            }
            if let Some(item) = load_evidence(
                conn,
                principal,
                &spec.scope,
                &source_id,
                "learned_local_association",
            )? {
                evidence.push(item);
            }
        }
        for (source_id, _assembly, member_role) in super::assembly::suggest_authorized_members(
            conn,
            principal,
            &spec.scope,
            &spec.cues,
            spec.max_results,
        )? {
            if evidence.iter().any(|item| item.source_id == source_id) {
                continue;
            }
            if let Some(item) = load_evidence(
                conn,
                principal,
                &spec.scope,
                &source_id,
                if member_role.is_required_exception() {
                    "assembly_exception"
                } else {
                    "learned_assembly_route"
                },
            )? {
                evidence.push(item);
            } else if member_role.is_required_exception() {
                return Ok(PreparedView {
                    status: "qualification_unavailable".into(),
                    evidence: Vec::new(),
                    source_refs: Vec::new(),
                    delivery_id: None,
                    payload: String::new(),
                    payload_bytes: 0,
                    projection_pending: pending as usize,
                    restore_epoch: restore.clone(),
                    policy_epoch: policy.clone(),
                    fingerprint: cortex_logic::traces::content_hash("qualification_unavailable"),
                    assembly_brief: String::new(),
                });
            }
        }
    }
    let (mut evidence, closure_status) = close_evidence(conn, principal, spec, evidence)?;
    let rendered = serde_json::to_string(&evidence).map_err(|e| e.to_string())?;
    let bounded = evidence.len() > spec.max_results || rendered.len() > spec.max_bytes;
    let permission_required:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM observation_sources WHERE principal=?1 AND scope_label=?2 AND (enabled=0 OR policy_epoch!=?3))",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    let unavailable:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND s.availability!='owned_inline')",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    let status = if let Some(status) = closure_status {
        status
    } else if permission_required {
        "permission_required"
    } else if unavailable {
        "source_unavailable"
    } else if pending > 0 {
        "projection_pending"
    } else if bounded {
        "quota_blocked"
    } else {
        "ready"
    };
    // No partial bundle can masquerade as an absence/exception-complete result.
    let payload = if status == "ready" && !evidence.is_empty() {
        rendered
    } else {
        String::new()
    };
    let source_refs = evidence
        .iter()
        .take(spec.max_results)
        .map(|e| e.source_id.clone())
        .collect();
    if bounded {
        evidence.clear();
    }
    let fingerprint = cortex_logic::traces::content_hash(
        &serde_json::json!([principal, spec, restore, policy, status, &payload]).to_string(),
    );
    Ok(PreparedView {
        status: status.into(),
        evidence,
        source_refs,
        delivery_id: None,
        payload_bytes: payload.len(),
        payload,
        projection_pending: pending as usize,
        restore_epoch: restore,
        policy_epoch: policy,
        fingerprint,
        assembly_brief: String::new(),
    })
}

fn materialize_recent(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
) -> Result<PreparedView, String> {
    let (_, restore, policy) = crate::db::records::brain_epochs(conn);
    let capture_state: Option<String> = conn.query_row("SELECT state FROM capture_policy WHERE scope IN (?1,'*') ORDER BY CASE WHEN scope=?1 THEN 0 ELSE 1 END LIMIT 1",params![spec.scope],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if capture_state.as_deref() == Some("stopped") {
        return Err("capture_stopped".into());
    }
    let pending: i64 = conn.query_row("SELECT count(*) FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id LEFT JOIN observation_projection p ON p.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND g.role!='delivery_only' AND s.availability='owned_inline' AND (p.status IS NULL OR p.status!='ready') AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    let mut stmt = conn.prepare("SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND g.role!='delivery_only' AND s.availability='owned_inline' AND EXISTS(SELECT 1 FROM record_heads h WHERE h.revision_id=e.revision_id) AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id) ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?4").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![principal, spec.scope, policy, spec.max_results as i64 + 1],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut evidence = Vec::new();
    for (source_id, revision_id, source_key, role, bytes) in rows {
        evidence.push(Evidence {
            source_id,
            revision_id,
            source_key,
            role,
            text: String::from_utf8(bytes).map_err(|_| "source_not_utf8")?,
            route: "literal".into(),
        });
    }
    let rendered = serde_json::to_string(&evidence).map_err(|e| e.to_string())?;
    let bounded = evidence.len() > spec.max_results || rendered.len() > spec.max_bytes;
    let permission_required: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM observation_sources WHERE principal=?1 AND scope_label=?2 AND (enabled=0 OR policy_epoch!=?3))",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    let unavailable: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND s.availability!='owned_inline')",params![principal,spec.scope,policy],|r|r.get(0)).map_err(|e|e.to_string())?;
    let status = if permission_required {
        "permission_required"
    } else if unavailable {
        "source_unavailable"
    } else if pending > 0 {
        "projection_pending"
    } else if bounded {
        "quota_blocked"
    } else {
        "ready"
    };
    if bounded {
        evidence.clear();
    }
    let payload = if status == "ready" && !evidence.is_empty() {
        serde_json::to_string(&evidence).map_err(|e| e.to_string())?
    } else {
        String::new()
    };
    let source_refs = evidence
        .iter()
        .take(spec.max_results)
        .map(|e| e.source_id.clone())
        .collect();
    let fingerprint = cortex_logic::traces::content_hash(
        &serde_json::json!([principal, spec.scope, restore, policy, status, &payload]).to_string(),
    );
    Ok(PreparedView {
        status: status.into(),
        evidence,
        source_refs,
        delivery_id: None,
        payload: payload.clone(),
        payload_bytes: payload.len(),
        projection_pending: pending as usize,
        restore_epoch: restore,
        policy_epoch: policy,
        fingerprint,
        assembly_brief: String::new(),
    })
}

fn merge_prepared(views: Vec<PreparedView>, max_results: usize, max_bytes: usize) -> PreparedView {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    let mut pending = 0usize;
    let mut restore = String::new();
    let mut policy = String::new();
    let mut status = "ready".to_string();
    for view in views {
        pending += view.projection_pending;
        if restore.is_empty() {
            restore = view.restore_epoch;
        }
        if policy.is_empty() {
            policy = view.policy_epoch;
        }
        if evidence.is_empty() && view.status != "ready" {
            status = view.status.clone();
        }
        for item in view.evidence {
            if seen.insert(item.source_id.clone()) {
                evidence.push(item);
            }
        }
    }
    evidence.truncate(max_results);
    while !evidence.is_empty() {
        let rendered = serde_json::to_string(&evidence).unwrap_or_default();
        if rendered.len() <= max_bytes {
            break;
        }
        evidence.pop();
    }
    if !evidence.is_empty() {
        status = "ready".into();
    }
    let payload = if status == "ready" && !evidence.is_empty() {
        serde_json::to_string(&evidence).unwrap_or_default()
    } else {
        String::new()
    };
    let source_refs = evidence.iter().map(|e| e.source_id.clone()).collect();
    let fingerprint = cortex_logic::traces::content_hash(
        &serde_json::json!([&source_refs, &status, pending, &payload]).to_string(),
    );
    PreparedView {
        status,
        evidence,
        source_refs,
        delivery_id: None,
        payload: payload.clone(),
        payload_bytes: payload.len(),
        projection_pending: pending,
        restore_epoch: restore,
        policy_epoch: policy,
        fingerprint,
        assembly_brief: String::new(),
    }
}

fn pull_spec(scope: &str, query: &str, max_results: usize, max_bytes: usize, learned: bool) -> Result<NeedSpec, String> {
    let spec = NeedSpec {
        id: format!("pull:{}", uuid::Uuid::new_v4()),
        scope: observation::normalize_scope(scope),
        cues: cues(query).into_iter().collect(),
        exclude_cues: Vec::new(),
        max_results,
        max_bytes,
        ttl_seconds: 1,
        learned,
    };
    validate(&spec)?;
    Ok(spec)
}

impl CortexRuntime {
    pub async fn query_observations(
        &self,
        cx: &Cx,
        scope: &str,
        query: &str,
        max_results: usize,
        max_bytes: usize,
        learned: bool,
    ) -> Result<PreparedView, String> {
        if query.len() > 4096 {
            return Err("query_byte_limit".into());
        }
        let spec = pull_spec(scope, query, max_results, max_bytes, learned)?;
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        project(&tx, &principal, &spec.scope, 32)?;
        let view = materialize(&tx, &principal, &spec)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(view)
    }

    /// Cue-filtered pull over caller project roots. Path-scoped sources stay
    /// in their repository; the extra label (default `project`) remains the
    /// unscoped bucket when roots are named.
    pub async fn query_observations_for_paths(
        &self,
        cx: &Cx,
        query: &str,
        paths: &[String],
        extra_scope: Option<&str>,
        max_results: usize,
        max_bytes: usize,
        learned: bool,
    ) -> Result<PreparedView, String> {
        if query.len() > 4096 {
            return Err("query_byte_limit".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let scopes = observation::resolve_query_scopes(&tx, &principal, paths, extra_scope)?;
        let mut views = Vec::new();
        for scope in scopes {
            let spec = pull_spec(&scope, query, max_results, max_bytes, learned)?;
            project(&tx, &principal, &spec.scope, 32)?;
            views.push(materialize(&tx, &principal, &spec)?);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(merge_prepared(views, max_results, max_bytes))
    }

    /// Latest attributed observations in the caller roots, without a cue join.
    /// Used when orient names a project and the task is only that path.
    pub async fn recent_observations_for_paths(
        &self,
        cx: &Cx,
        paths: &[String],
        extra_scope: Option<&str>,
        max_results: usize,
        max_bytes: usize,
    ) -> Result<PreparedView, String> {
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        let scopes = observation::resolve_query_scopes(&tx, &principal, paths, extra_scope)?;
        let mut views = Vec::new();
        for scope in scopes {
            let spec = NeedSpec {
                id: format!("recent:{}", uuid::Uuid::new_v4()),
                scope: observation::normalize_scope(&scope),
                cues: vec!["_".into()],
                exclude_cues: Vec::new(),
                max_results,
                max_bytes,
                ttl_seconds: 1,
                learned: false,
            };
            views.push(materialize_recent(&tx, &principal, &spec)?);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(merge_prepared(views, max_results, max_bytes))
    }

    pub async fn rebuild_observation_projection(
        &self,
        cx: &Cx,
        scope: &str,
    ) -> Result<usize, String> {
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let scope = observation::normalize_scope(scope);
        if scope.is_empty() || scope.len() > 1024 {
            return Err("invalid_need_bounds".into());
        }
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM observation_projection WHERE source_id IN (SELECT e.source_id FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key WHERE e.principal=?1 AND g.scope_label=?2)",params![principal,scope]).map_err(|e|e.to_string())?;
        tx.execute(
            "DELETE FROM observation_postings WHERE principal=?1 AND scope=?2",
            params![principal, scope],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM observation_matches WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND scope=?2)",params![principal,scope]).map_err(|e|e.to_string())?;
        let count = project(&tx, &principal, &scope, 32)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(count)
    }
    pub async fn subscribe_observations(
        &self,
        cx: &Cx,
        spec: NeedSpec,
    ) -> Result<PreparedView, String> {
        if spec.id.starts_with("pull:") {
            return Err("reserved_need_identity".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        register(&tx, &principal, &spec)?;
        project(&tx, &principal, &spec.scope, 32)?;
        let view = materialize(&tx, &principal, &spec)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(view)
    }

    pub async fn prepare_observations(
        &self,
        cx: &Cx,
        id: &str,
        context: &str,
        present_delivery: Option<&str>,
    ) -> Result<PreparedView, String> {
        if context.is_empty() || context.len() > 256 {
            return Err("invalid_context_identity".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "DELETE FROM observation_deliveries WHERE principal=?1 AND expires<=?2",
            params![principal, now],
        )
        .map_err(|e| e.to_string())?;
        let raw:String=tx.query_row("SELECT spec_json FROM observation_needs WHERE principal=?1 AND need_id=?2 AND expires>?3",params![principal,id,now],|r|r.get(0)).optional().map_err(|e|e.to_string())?.ok_or("need_missing_or_expired")?;
        let spec: NeedSpec = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        project(&tx, &principal, &spec.scope, 32)?;
        let mut view = materialize(&tx, &principal, &spec)?;
        let present = if let Some(delivery) = present_delivery {
            tx.query_row("SELECT 1 FROM observation_deliveries WHERE delivery_id=?1 AND principal=?2 AND need_id=?3 AND context=?4 AND fingerprint=?5 AND restore_epoch=?6 AND expires>?7",params![delivery,principal,id,context,view.fingerprint,view.restore_epoch,now],|r|r.get::<_,i64>(0)).optional().map_err(|e|e.to_string())?.is_some()
        } else {
            false
        };
        if present {
            view.payload.clear();
            view.payload_bytes = 0;
            view.delivery_id = present_delivery.map(str::to_string);
        } else if !view.payload.is_empty() {
            let delivery = format!("delivery:{}", uuid::Uuid::new_v4());
            tx.execute(
                "INSERT INTO observation_deliveries VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![
                    delivery,
                    principal,
                    id,
                    context,
                    view.fingerprint,
                    view.restore_epoch,
                    now + 300
                ],
            )
            .map_err(|e| e.to_string())?;
            view.delivery_id = Some(delivery);
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(view)
    }

    /// Retraction excludes a source from active Views without deleting its exact bytes.
    pub async fn retract_observation(
        &self,
        cx: &Cx,
        source_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err("invalid_retraction_reason".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let key: String = tx
            .query_row(
                "SELECT source_key FROM observation_events WHERE principal=?1 AND source_id=?2",
                params![principal, source_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("source_not_authorized")?;
        observation::granted(&tx, &principal, &key, true)?;
        tx.execute("INSERT INTO observation_retractions VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET reason=excluded.reason",params![source_id,principal,reason]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    /// Bind a required child to a parent. Delivery of the parent is incomplete
    /// unless that child is still authorized, current, and unretracted.
    pub async fn require_observation(
        &self,
        cx: &Cx,
        parent_id: &str,
        child_id: &str,
    ) -> Result<(), String> {
        if parent_id == child_id {
            return Err("invalid_requirement".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let parent_scope: String = tx
            .query_row(
                "SELECT g.scope_label FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key WHERE e.principal=?1 AND e.source_id=?2",
                params![principal, parent_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("source_not_authorized")?;
        let child_scope: String = tx
            .query_row(
                "SELECT g.scope_label FROM observation_events e JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key WHERE e.principal=?1 AND e.source_id=?2",
                params![principal, child_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("source_not_authorized")?;
        if parent_scope != child_scope {
            return Err("requirement_scope_mismatch".into());
        }
        tx.execute(
            "INSERT OR IGNORE INTO observation_requirements VALUES(?1,?2,?3)",
            params![principal, parent_id, child_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }
}
