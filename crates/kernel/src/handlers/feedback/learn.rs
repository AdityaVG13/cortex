//! Loop 3: outcome → learning events. A successful use records a
//! `verified_use` event (cues = query tokens in the read path's vocabulary,
//! target = a learned singleton assembly over the used decision, training
//! unit = the receipt); harmful reuse records −1. Partial/failure teach
//! nothing: the reward schema has no half votes and task failure does not
//! indict a source. Routes refresh inline when events land.

use rusqlite::{Connection, params};

/// Exact singleton bundle over one used decision, created once on first
/// successful use. Returns the assembly id, or `None` when the source does
/// not resolve to a stored record (skipped, never an error).
pub fn ensure_learned_singleton(
    conn: &Connection,
    principal: &str,
    scope: &str,
    source: &str,
) -> Result<Option<String>, String> {
    let id = format!("learned:{source}");
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM assemblies WHERE principal = ?1 AND assembly_id = ?2",
            params![principal, id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists > 0 {
        return Ok(Some(id));
    }
    let Some(record_id) = crate::handlers::operations::legacy_record(conn, source)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let heads = crate::db::records::heads(conn, &record_id).map_err(|e| e.to_string())?;
    let Some(head) = heads.first() else {
        return Ok(None);
    };
    let Some(body) = crate::db::records::revision_body(conn, head).map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let envelope = cortex_logic::assembly::factor_records(&[body]);
    let codec = envelope
        .get("codec")
        .and_then(serde_json::Value::as_str)
        .ok_or("unknown_factor_codec")?;
    cortex_logic::assembly::FactorCodec::parse(codec)?;
    let sequence = crate::db::records::append_ack_commit(conn, principal)?;
    conn.execute(
        "INSERT INTO assemblies(assembly_id,principal,scope_label,kind,created_sequence) VALUES(?1,?2,?3,'learned_singleton',?4) ON CONFLICT(principal,assembly_id) DO NOTHING",
        params![id, principal, scope, sequence],
    )
    .map_err(|e| e.to_string())?;
    let revision_id = format!("{id}@{sequence}");
    conn.execute(
        "INSERT INTO assembly_revisions VALUES(?1,?2,?3,?4,?5,?6)",
        params![revision_id, principal, id, codec, envelope.to_string(), sequence],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO assembly_members VALUES(?1,?2,?3,?4)",
        params![revision_id, head, 0, "observation"],
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(id))
}

/// Record outcome events for the used sources and refresh routes.
/// Returns the events recorded. Unresolvable sources and missing query
/// text skip quietly; the outcome row the caller recorded is unaffected.
pub fn record_outcome_events(
    conn: &Connection,
    principal: &str,
    scope: &str,
    ledger_id: i64,
    outcome: &str,
    used: &[String],
    harmful_reuse: bool,
    query_text: Option<&str>,
    receipt: Option<&str>,
) -> Result<usize, String> {
    const MAX_SOURCES: usize = 64;
    let reward = if harmful_reuse {
        -1
    } else {
        match outcome {
            "success" => 1,
            _ => return Ok(0),
        }
    };
    let Some(query_text) = query_text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(0);
    };
    let mut cues = crate::runtime::assembly::tokenize_cues(query_text);
    cues.truncate(64);
    if cues.is_empty() {
        return Ok(0);
    }
    let scope = crate::runtime::observation::normalize_scope(scope);
    if scope.is_empty() {
        return Ok(0);
    }
    let unit = receipt
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("feedback-{ledger_id}"));
    let now = chrono::Utc::now().timestamp();
    let mut recorded = 0;
    for (index, source) in used.iter().take(MAX_SOURCES).enumerate() {
        let source = source.trim();
        if source.is_empty() || source.len() > 256 {
            continue;
        }
        let Some(target) = ensure_learned_singleton(conn, principal, &scope, source)? else {
            continue;
        };
        let event = cortex_logic::assembly::LearningEvent {
            origin: "outcome".into(),
            origin_event_id: format!("feedback-{ledger_id}-{index}"),
            principal: principal.into(),
            scope: scope.clone(),
            training_unit: unit.clone(),
            target,
            kind: cortex_logic::assembly::LearningKind::VerifiedUse,
            reward,
            cues: cues.clone(),
            sources: vec![unit.clone()],
            observed_at: now,
            receipt_ref: unit.clone(),
        };
        event.validate()?;
        if crate::runtime::assembly::record_learning_event_conn(conn, principal, &event)? {
            recorded += 1;
        }
    }
    if recorded > 0 {
        crate::runtime::assembly::refresh_routes(conn, principal, &scope, now)?;
    }
    Ok(recorded)
}
