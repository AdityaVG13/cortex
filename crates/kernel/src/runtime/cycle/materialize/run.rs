use super::*;
use rusqlite::{Connection, params};

pub(in crate::runtime) fn materialize(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
) -> Result<PreparedView, String> {
    let (_, restore, policy) = crate::db::records::brain_epochs(conn);
    require_capture_running(conn, &spec.scope)?;
    let mut pending = pending_events(conn, principal, &spec.scope, &policy)?;
    pending += pending_file_sources(conn, principal, &spec.scope, &policy)?;
    let selected_cues = if spec.id.starts_with("pull:") {
        serde_json::to_string(&spec.cues).map_err(|e| e.to_string())?
    } else {
        "[]".into()
    };
    let mut stmt=conn.prepare(&format!("WITH candidates AS (SELECT principal,source_id FROM observation_matches WHERE principal=?1 AND need_id=?2 UNION SELECT p.principal,p.source_id FROM observation_postings p JOIN json_each(?6) q ON q.value=p.cue WHERE p.principal=?1 AND p.scope=?3) SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM candidates m JOIN observation_events e ON e.source_id=m.source_id AND e.principal=m.principal JOIN observation_sources g ON g.principal=e.principal AND g.source_key=e.source_key JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE m.principal=?1 AND g.scope_label=?3 AND g.enabled=1 AND g.policy_epoch=?4 AND g.role!='delivery_only' AND s.availability='owned_inline' AND {} ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?5", crate::runtime::observation::LIVE_HEAD_SQL)).map_err(|e|e.to_string())?;
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
            evidence_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut evidence = evidence_from_mapped(rows)?;
    if spec.learned {
        crate::runtime::associations::maintain_in_transaction(conn, principal, &spec.scope)?;
        for (source_id, _) in crate::runtime::associations::candidates(
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
        for (source_id, _assembly, member_role) in
            crate::runtime::assembly::suggest_authorized_members(
                conn,
                principal,
                &spec.scope,
                &spec.cues,
                spec.max_results,
            )?
        {
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
                return Ok(prepared_view(
                    "qualification_unavailable",
                    Vec::new(),
                    Vec::new(),
                    String::new(),
                    pending as usize,
                    restore,
                    policy,
                    cortex_logic::traces::content_hash("qualification_unavailable"),
                ));
            }
        }
    }
    let (evidence, closure_status) = close_evidence(conn, principal, spec, evidence)?;
    finish_prepared(
        conn,
        principal,
        spec,
        restore,
        policy,
        pending,
        evidence,
        closure_status,
        FingerprintSeed::Spec { principal, spec },
    )
}

pub(in crate::runtime) fn materialize_recent(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
) -> Result<PreparedView, String> {
    let (_, restore, policy) = crate::db::records::brain_epochs(conn);
    require_capture_running(conn, &spec.scope)?;
    let pending = pending_events(conn, principal, &spec.scope, &policy)?;
    let mut stmt = conn.prepare(&format!("SELECT e.source_id,e.revision_id,e.source_key,g.role,s.inline_payload FROM {} JOIN sources s ON s.source_id=e.source_id JOIN revisions v ON v.revision_id=e.revision_id WHERE e.principal=?1 AND g.scope_label=?2 AND g.enabled=1 AND g.policy_epoch=?3 AND g.role!='delivery_only' AND s.availability='owned_inline' AND {} ORDER BY s.capture_sequence DESC,e.source_id LIMIT ?4", crate::runtime::observation::EVENT_GRANT_JOIN, crate::runtime::observation::LIVE_HEAD_SQL)).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(
            params![principal, spec.scope, policy, spec.max_results as i64 + 1],
            evidence_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let evidence = evidence_from_mapped(rows)?;
    finish_prepared(
        conn,
        principal,
        spec,
        restore,
        policy,
        pending,
        evidence,
        None,
        FingerprintSeed::Scope {
            principal,
            scope: &spec.scope,
        },
    )
}

pub(in crate::runtime) fn merge_prepared(
    views: Vec<PreparedView>,
    max_results: usize,
    max_bytes: usize,
) -> PreparedView {
    let mut seen = BTreeSet::new();
    let mut evidence = Vec::new();
    let mut pending = 0usize;
    let mut restore = String::new();
    let mut policy = String::new();
    let mut blocking = String::new();
    for view in &views {
        pending += view.projection_pending;
        if restore.is_empty() {
            restore = view.restore_epoch.clone();
        }
        if policy.is_empty() {
            policy = view.policy_epoch.clone();
        }
        if view.status != "ready" && blocking.is_empty() {
            blocking = view.status.clone();
        }
    }
    // Incomplete scopes keep the merge unready. One ready sibling must not
    // report a complete answer while another root is still pending, denied,
    // or exception-incomplete — that would omit the blocked root's evidence
    // while claiming `ready`.
    let status = if !blocking.is_empty() {
        blocking
    } else {
        "ready".to_string()
    };
    if status == "ready" {
        for view in views {
            for item in view.evidence {
                if seen.insert(item.source_id.clone()) {
                    evidence.push(item);
                }
            }
        }
    }
    evidence.truncate(max_results);
    let mut payload = String::new();
    if status == "ready" {
        while !evidence.is_empty() {
            match serde_json::to_string(&evidence) {
                Ok(rendered) if rendered.len() <= max_bytes => {
                    payload = rendered;
                    break;
                }
                Ok(_) => {
                    evidence.pop();
                }
                Err(_) => {
                    evidence.clear();
                    break;
                }
            }
        }
    } else {
        evidence.clear();
    }
    let source_refs = evidence.iter().map(|e| e.source_id.clone()).collect();
    let fingerprint = cortex_logic::traces::content_hash(
        &serde_json::json!([&source_refs, &status, pending, &payload]).to_string(),
    );
    prepared_view(
        status,
        evidence,
        source_refs,
        payload,
        pending,
        restore,
        policy,
        fingerprint,
    )
}
