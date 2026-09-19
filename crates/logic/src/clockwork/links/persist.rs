use super::{ClockOrigin, ClockRelation, ClockTarget};
use crate::clockwork::anchors::{Anchor, AnchorKind};
use rusqlite::{Connection, params};

pub(super) fn persist_anchors(
    conn: &Connection,
    anchors: &[Anchor],
    target_type: &str,
    target_id: i64,
    origin: ClockOrigin,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    for anchor in anchors {
        persist_anchor_row(conn, anchor, target_type, target_id, origin, trace_id)?;
        if anchor.kind == AnchorKind::Path {
            for ancestor in path_ancestors(&anchor.value) {
                if let Some(parent) = Anchor::new(AnchorKind::Path, ancestor, 2) {
                    persist_anchor_row(conn, &parent, target_type, target_id, origin, trace_id)?;
                }
            }
        }
    }
    Ok(())
}

fn persist_anchor_row(
    conn: &Connection,
    anchor: &Anchor,
    target_type: &str,
    target_id: i64,
    origin: ClockOrigin,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    conn.execute("INSERT INTO clock_anchors (kind, value, display_value, specificity) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(kind, value) DO UPDATE SET specificity = MAX(clock_anchors.specificity, excluded.specificity), display_value = COALESCE(clock_anchors.display_value, excluded.display_value)", params![anchor.kind.as_str(), anchor.value, anchor.display_value, anchor.specificity])?;
    let anchor_id: i64 = conn.query_row(
        "SELECT id FROM clock_anchors WHERE kind = ?1 AND value = ?2",
        params![anchor.kind.as_str(), anchor.value],
        |row| row.get(0),
    )?;
    conn.execute("INSERT INTO clock_anchor_evidence (anchor_id, target_type, target_id, origin, evidence_count, first_trace_id, last_trace_id) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5) ON CONFLICT(anchor_id, target_type, target_id, origin) DO UPDATE SET evidence_count = clock_anchor_evidence.evidence_count + 1, last_trace_id = COALESCE(excluded.last_trace_id, clock_anchor_evidence.last_trace_id)", params![anchor_id, target_type, target_id, origin.as_str(), trace_id])?;
    Ok(())
}

pub(super) fn link_shared_strong_anchors(
    conn: &Connection,
    anchors: &[Anchor],
    target_type: &str,
    target_id: i64,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    for anchor in anchors.iter().filter(|a| a.specificity >= 2) {
        // cortex-7db determinism contract: this LIMIT decides which clock
        // links exist, so the cut must be data-defined, never SQLite-plan
        // defined. The order is total: specificity is constant for a fixed
        // (kind, value) because clock_anchors has UNIQUE(kind, value), so
        // evidence_count is the strength signal, and (target_type, target_id,
        // origin) is the clock_anchor_evidence PK tail.
        let mut stmt = conn.prepare_cached("SELECT e.target_type, e.target_id FROM clock_anchor_evidence e JOIN clock_anchors a ON a.id = e.anchor_id WHERE a.kind = ?1 AND a.value = ?2 AND NOT (e.target_type = ?3 AND e.target_id = ?4) ORDER BY e.evidence_count DESC, e.target_type ASC, e.target_id ASC, e.origin ASC LIMIT 8")?;
        let rows = stmt.query_map(
            params![anchor.kind.as_str(), anchor.value, target_type, target_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        for row in rows.flatten() {
            let (other_type, other_id) = row;
            upsert_link(
                conn,
                target_type,
                target_id,
                &other_type,
                other_id,
                relation_for_anchor(anchor.kind),
                trace_id,
            )?;
        }
    }
    Ok(())
}

fn path_ancestors(value: &str) -> Vec<String> {
    let parts: Vec<&str> = value
        .split('/')
        .filter(|part| !part.is_empty() && *part != "*" && *part != "**")
        .collect();
    let mut out = Vec::new();
    if parts.len() >= 2 {
        for end in 2..parts.len() {
            out.push(parts[..end].join("/"));
        }
    }
    out
}

fn relation_for_anchor(kind: AnchorKind) -> ClockRelation {
    match kind {
        AnchorKind::Path => ClockRelation::SamePath,
        AnchorKind::Symbol => ClockRelation::SameSymbol,
        AnchorKind::Goal => ClockRelation::SameGoal,
        _ => ClockRelation::ObservedWith,
    }
}

fn ordered_pair<'a>(
    a_type: &'a str,
    a_id: i64,
    b_type: &'a str,
    b_id: i64,
) -> (&'a str, i64, &'a str, i64) {
    if (a_type, a_id) <= (b_type, b_id) {
        (a_type, a_id, b_type, b_id)
    } else {
        (b_type, b_id, a_type, a_id)
    }
}

fn upsert_link(
    conn: &Connection,
    src_type: &str,
    src_id: i64,
    dst_type: &str,
    dst_id: i64,
    relation: ClockRelation,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    if src_type == dst_type && src_id == dst_id {
        return Ok(());
    }
    let (left_type, left_id, right_type, right_id) =
        ordered_pair(src_type, src_id, dst_type, dst_id);
    conn.execute("INSERT INTO clock_links (src_type, src_id, dst_type, dst_id, relation, evidence_count, status, last_trace_id, last_observed_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, 'derived', ?6, datetime('now')) ON CONFLICT(src_type, src_id, dst_type, dst_id, relation) DO UPDATE SET evidence_count = clock_links.evidence_count + 1, last_trace_id = COALESCE(excluded.last_trace_id, clock_links.last_trace_id), last_observed_at = excluded.last_observed_at WHERE clock_links.status != 'rejected'", params![left_type, left_id, right_type, right_id, relation.as_str(), trace_id])?;
    Ok(())
}

pub fn record_used_with(
    conn: &Connection,
    left: &ClockTarget,
    right: &ClockTarget,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    upsert_link(
        conn,
        &left.target_type,
        left.target_id,
        &right.target_type,
        right.target_id,
        ClockRelation::UsedWith,
        trace_id,
    )
}

pub fn reject_used_with(
    conn: &Connection,
    left: &ClockTarget,
    right: &ClockTarget,
) -> rusqlite::Result<()> {
    let (src_type, src_id, dst_type, dst_id) = ordered_pair(
        left.target_type.as_str(),
        left.target_id,
        right.target_type.as_str(),
        right.target_id,
    );
    conn.execute("UPDATE clock_links SET status = 'rejected' WHERE src_type = ?1 AND src_id = ?2 AND dst_type = ?3 AND dst_id = ?4 AND relation = 'used_with'", params![src_type, src_id, dst_type, dst_id])?;
    Ok(())
}
