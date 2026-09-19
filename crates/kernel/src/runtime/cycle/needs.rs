use super::observation;
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

pub(in crate::runtime) fn validate(spec: &NeedSpec) -> Result<BTreeSet<String>, String> {
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

pub(in crate::runtime) fn on_capture(
    conn: &Connection,
    principal: &str,
    source: &str,
) -> Result<(), String> {
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
pub(in crate::runtime) fn project(
    conn: &Connection,
    principal: &str,
    scope: &str,
    limit: usize,
) -> Result<usize, String> {
    let mut stmt = conn.prepare(&format!("SELECT e.source_id,s.inline_payload FROM {} JOIN sources s ON s.source_id=e.source_id LEFT JOIN observation_projection p ON p.source_id=e.source_id WHERE e.principal=?1 AND g.scope_label=?2 AND p.source_id IS NULL AND s.availability='owned_inline' AND g.enabled=1 AND g.policy_epoch=({}) AND g.role!='delivery_only' ORDER BY s.capture_sequence,e.source_id LIMIT ?3", observation::EVENT_GRANT_JOIN, crate::db::records::POLICY_EPOCH_SELECT)).map_err(|e| e.to_string())?;
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

fn expire_need_rows(conn: &Connection, principal: &str, now: i64) -> Result<(), String> {
    for sql in [
        "DELETE FROM observation_need_cues WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND expires<=?2)",
        "DELETE FROM observation_matches WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND expires<=?2)",
        "DELETE FROM observation_needs WHERE principal=?1 AND expires<=?2",
        "DELETE FROM observation_deliveries WHERE principal=?1 AND expires<=?2",
    ] {
        conn.execute(sql, params![principal, now])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(in crate::runtime) fn register(
    conn: &Connection,
    principal: &str,
    spec: &NeedSpec,
) -> Result<(), String> {
    let mut spec = spec.clone();
    spec.scope = observation::normalize_scope(&spec.scope);
    let normalized = validate(&spec)?;
    let now = chrono::Utc::now().timestamp();
    expire_need_rows(conn, principal, now)?;
    let count: i64 = conn.query_row("SELECT count(*) FROM observation_needs WHERE principal=?1 AND expires>?2 AND need_id!=?3",params![principal,now,spec.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count >= 128 {
        return Err("active_need_quota".into());
    }
    // Only derived subscription state is replaced, never source evidence.
    for sql in [
        "DELETE FROM observation_need_cues WHERE principal=?1 AND need_id=?2",
        "DELETE FROM observation_matches WHERE principal=?1 AND need_id=?2",
    ] {
        conn.execute(sql, params![principal, spec.id])
            .map_err(|e| e.to_string())?;
    }
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

pub(in crate::runtime) fn pull_spec(
    scope: &str,
    query: &str,
    max_results: usize,
    max_bytes: usize,
    learned: bool,
) -> Result<NeedSpec, String> {
    if query.len() > 4096 {
        return Err("query_byte_limit".into());
    }
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

pub(in crate::runtime) fn event_scope(
    conn: &Connection,
    principal: &str,
    source_id: &str,
) -> Result<String, String> {
    conn.query_row(
        &format!(
            "SELECT g.scope_label FROM {} WHERE e.principal=?1 AND e.source_id=?2",
            observation::EVENT_GRANT_JOIN
        ),
        params![principal, source_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "source_not_authorized".into())
}
