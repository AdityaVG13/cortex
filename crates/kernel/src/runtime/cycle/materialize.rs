use super::*;
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

fn require_capture_running(conn: &Connection, scope: &str) -> Result<(), String> {
    let capture_state = crate::db::capture_policy::raw_state_for(conn, scope)?;
    if capture_state.as_deref() == Some("stopped") {
        Err("capture_stopped".into())
    } else {
        Ok(())
    }
}

fn pending_events(
    conn: &Connection,
    principal: &str,
    scope: &str,
    policy: &str,
) -> Result<i64, String> {
    conn.query_row(&format!("SELECT count(*) FROM {} JOIN sources s ON s.source_id=e.source_id LEFT JOIN observation_projection p ON p.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND g.role!='delivery_only' AND s.availability='owned_inline' AND (p.status IS NULL OR p.status!='ready') AND NOT EXISTS(SELECT 1 FROM observation_retractions t WHERE t.source_id=e.source_id)", crate::runtime::observation::EVENT_GRANT_JOIN),params![principal,scope,policy],|r|r.get(0)).map_err(|e|e.to_string())
}

fn pending_file_sources(
    conn: &Connection,
    principal: &str,
    scope: &str,
    policy: &str,
) -> Result<i64, String> {
    conn.query_row("SELECT count(*) FROM observation_sources g WHERE g.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND substr(g.source_key,1,5)='file:' AND NOT EXISTS(SELECT 1 FROM observation_events e WHERE e.principal=g.principal AND e.source_key=g.source_key)",params![principal,scope,policy],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())
}

fn permission_required(
    conn: &Connection,
    principal: &str,
    scope: &str,
    policy: &str,
) -> Result<bool, String> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM observation_sources WHERE principal=?1 AND scope_label=?2 AND (enabled=0 OR policy_epoch!=?3))",params![principal,scope,policy],|r|r.get(0)).map_err(|e|e.to_string())
}

fn source_unavailable(
    conn: &Connection,
    principal: &str,
    scope: &str,
    policy: &str,
) -> Result<bool, String> {
    conn.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND s.availability!='owned_inline')", crate::runtime::observation::EVENT_GRANT_JOIN),params![principal,scope,policy],|r|r.get(0)).map_err(|e|e.to_string())
}

fn evidence_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, String, String, Vec<u8>)> {
    Ok((
        r.get::<_, String>(0)?,
        r.get::<_, String>(1)?,
        r.get::<_, String>(2)?,
        r.get::<_, String>(3)?,
        r.get::<_, Vec<u8>>(4)?,
    ))
}

fn evidence_from_mapped(
    rows: Vec<(String, String, String, String, Vec<u8>)>,
) -> Result<Vec<Evidence>, String> {
    rows.into_iter()
        .map(|(source_id, revision_id, source_key, role, bytes)| {
            evidence_from_parts(source_id, revision_id, source_key, role, bytes, "literal")
        })
        .collect()
}

fn prepared_status(
    permission_required: bool,
    unavailable: bool,
    pending: i64,
    bounded: bool,
) -> &'static str {
    [
        (permission_required, "permission_required"),
        (unavailable, "source_unavailable"),
        (pending > 0, "projection_pending"),
        (bounded, "quota_blocked"),
    ]
    .into_iter()
    .find(|(hit, _)| *hit)
    .map(|(_, status)| status)
    .unwrap_or("ready")
}

fn prepared_view(
    status: impl Into<String>,
    evidence: Vec<Evidence>,
    source_refs: Vec<String>,
    payload: String,
    pending: usize,
    restore: String,
    policy: String,
    fingerprint: String,
) -> PreparedView {
    PreparedView {
        status: status.into(),
        evidence,
        source_refs,
        delivery_id: None,
        payload_bytes: payload.len(),
        payload,
        projection_pending: pending,
        restore_epoch: restore,
        policy_epoch: policy,
        fingerprint,
        assembly_brief: String::new(),
    }
}

fn bound_payload(
    spec: &NeedSpec,
    mut evidence: Vec<Evidence>,
    permission_required: bool,
    unavailable: bool,
    pending: i64,
    closure_status: Option<&'static str>,
) -> Result<(Vec<Evidence>, &'static str, String, Vec<String>), String> {
    let rendered = serde_json::to_string(&evidence).map_err(|e| e.to_string())?;
    let bounded = evidence.len() > spec.max_results || rendered.len() > spec.max_bytes;
    let status = closure_status
        .unwrap_or_else(|| prepared_status(permission_required, unavailable, pending, bounded));
    // No partial bundle can masquerade as an absence/exception-complete result.
    // `source_refs` used to be taken *before* the clear, so a quota-blocked
    // View still named the overflowing sources and a later expand could
    // fetch the prefix `merge_prepared` / `materialize_recent` refuse.
    if bounded {
        evidence.clear();
    }
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
    Ok((evidence, status, payload, source_refs))
}

enum FingerprintSeed<'a> {
    Spec {
        principal: &'a str,
        spec: &'a NeedSpec,
    },
    Scope {
        principal: &'a str,
        scope: &'a str,
    },
}

fn finish_prepared(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
    restore: String,
    policy: String,
    pending: i64,
    evidence: Vec<Evidence>,
    closure_status: Option<&'static str>,
    seed: FingerprintSeed<'_>,
) -> Result<PreparedView, String> {
    let needs_permission = permission_required(conn, principal, &spec.scope, &policy)?;
    let unavailable = source_unavailable(conn, principal, &spec.scope, &policy)?;
    let (evidence, status, payload, source_refs) = bound_payload(
        spec,
        evidence,
        needs_permission,
        unavailable,
        pending,
        closure_status,
    )?;
    let fingerprint = cortex_logic::traces::content_hash(
        &match seed {
            FingerprintSeed::Spec { principal, spec } => {
                serde_json::json!([principal, spec, restore, policy, status, &payload])
            }
            FingerprintSeed::Scope { principal, scope } => {
                serde_json::json!([principal, scope, restore, policy, status, &payload])
            }
        }
        .to_string(),
    );
    Ok(prepared_view(
        status,
        evidence,
        source_refs,
        payload,
        pending as usize,
        restore,
        policy,
        fingerprint,
    ))
}

mod run;
pub(in crate::runtime) use run::{materialize, materialize_recent, merge_prepared};
