use super::super::query::QueryAnchor;
use super::super::{AnchorKind, normalize_anchor_value};
use super::*;
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

fn clock_target_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClockTarget> {
    Ok(ClockTarget {
        target_type: row.get(0)?,
        target_id: row.get(1)?,
    })
}

fn absorb_clock_targets(
    rows: impl Iterator<Item = rusqlite::Result<ClockTarget>>,
    seen: &mut BTreeSet<ClockTarget>,
    limit: usize,
) -> bool {
    for target in rows.flatten() {
        seen.insert(target);
        if seen.len() >= limit {
            return true;
        }
    }
    false
}

pub fn lookup_targets_for_anchors(
    conn: &Connection,
    anchors: &[QueryAnchor],
    limit: usize,
) -> rusqlite::Result<Vec<ClockTarget>> {
    let mut seen: BTreeSet<ClockTarget> = BTreeSet::new();
    for anchor in anchors {
        if anchor.kind == AnchorKind::Path {
            // Reverse parent match is only for paths extracted from text
            // (specificity >= 3). Spec-2 ancestors are projected onto every
            // descendant; using them here pulls sibling files that share a
            // parent after FTS already chose one path.
            let mut stmt = conn.prepare_cached("SELECT e.target_type, e.target_id FROM clock_anchor_evidence e JOIN clock_anchors a ON a.id = e.anchor_id WHERE a.kind = 'path' AND (a.value = ?1 OR a.value LIKE ?2 ESCAPE '\\' OR (a.specificity >= 3 AND ?1 LIKE replace(replace(replace(a.value, '\\', '\\\\'), '%', '\\%'), '_', '\\_') || '/%' ESCAPE '\\')) ORDER BY a.specificity DESC, e.target_type ASC, e.target_id ASC LIMIT ?3")?;
            let rows = stmt.query_map(
                params![
                    anchor.value,
                    crate::protocol::like_children(&anchor.value),
                    limit as i64
                ],
                clock_target_row,
            )?;
            if absorb_clock_targets(rows, &mut seen, limit) {
                return Ok(seen.into_iter().collect());
            }
            continue;
        }
        let mut stmt = conn.prepare_cached("SELECT e.target_type, e.target_id FROM clock_anchor_evidence e JOIN clock_anchors a ON a.id = e.anchor_id WHERE a.kind = ?1 AND a.value = ?2 ORDER BY a.specificity DESC, e.target_type ASC, e.target_id ASC LIMIT ?3")?;
        let rows = stmt.query_map(
            params![anchor.kind.as_str(), anchor.value, limit as i64],
            clock_target_row,
        )?;
        if absorb_clock_targets(rows, &mut seen, limit) {
            return Ok(seen.into_iter().collect());
        }
    }
    Ok(seen.into_iter().collect())
}

/// Like `lookup_targets_for_anchors` but reports, per target, the query
/// anchors that actually matched it. Hardness of a match is decided by the
/// matched anchor's specificity, never by the strongest anchor in the query.
pub fn lookup_targets_with_matches(
    conn: &Connection,
    anchors: &[QueryAnchor],
    limit: usize,
) -> rusqlite::Result<Vec<(ClockTarget, Vec<QueryAnchor>)>> {
    let mut matched: std::collections::BTreeMap<ClockTarget, Vec<QueryAnchor>> =
        std::collections::BTreeMap::new();
    for anchor in anchors {
        let targets = lookup_targets_for_anchors(conn, std::slice::from_ref(anchor), limit)?;
        for target in targets {
            let entry = matched.entry(target).or_default();
            if !entry.contains(anchor) {
                entry.push(anchor.clone());
            }
        }
        if matched.len() >= limit {
            break;
        }
    }
    // Membership can overshoot `limit` after a late anchor; keep the strongest
    // matches, not the lowest (target_type, target_id). BTreeMap order would
    // drop a spec-3 row with a high id in favor of a spec-2 neighbor.
    let mut out: Vec<(ClockTarget, Vec<QueryAnchor>)> = matched.into_iter().collect();
    out.sort_by(|a, b| {
        b.1.iter()
            .map(|anchor| anchor.specificity)
            .max()
            .unwrap_or(0)
            .cmp(
                &a.1.iter()
                    .map(|anchor| anchor.specificity)
                    .max()
                    .unwrap_or(0),
            )
            .then_with(|| a.0.target_type.cmp(&b.0.target_type))
            .then_with(|| a.0.target_id.cmp(&b.0.target_id))
    });
    out.truncate(limit);
    Ok(out)
}

/// The projected anchor values of one target for a kind: the row's own
/// namespace evidence (paths it names, hosts it cites).
pub fn target_anchor_values(
    conn: &Connection,
    target: &ClockTarget,
    kind: AnchorKind,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT a.value FROM clock_anchor_evidence e JOIN clock_anchors a ON a.id = e.anchor_id WHERE e.target_type = ?1 AND e.target_id = ?2 AND a.kind = ?3 ORDER BY a.value ASC LIMIT 64")?;
    let rows = stmt.query_map(
        params![target.target_type, target.target_id, kind.as_str()],
        |row| row.get::<_, String>(0),
    )?;
    Ok(rows.flatten().collect())
}

pub fn traverse_hops(
    conn: &Connection,
    seeds: &[ClockTarget],
    hops: u8,
    limit: usize,
) -> rusqlite::Result<Vec<(ClockTarget, u8)>> {
    let mut frontier: Vec<(ClockTarget, u8)> = seeds.iter().cloned().map(|t| (t, 0)).collect();
    let mut seen: BTreeSet<ClockTarget> = seeds.iter().cloned().collect();
    let mut out = frontier.clone();
    let max_hops = hops.min(super::super::MAX_GRAPH_HOPS);
    for _ in 0..max_hops {
        let mut next = Vec::new();
        for (seed, depth) in &frontier {
            if *depth >= max_hops {
                continue;
            }
            // cortex-7db determinism contract: this LIMIT decides BFS frontier
            // membership, so the cut must be data-defined. It mirrors the
            // quorum compare_rank_keys shape: strongest link first, then
            // target_type/target_id ASC. The order is total over the union:
            // relation completes the clock_links PK tail, and a row cannot
            // appear in both halves because src == dst rows are never written
            // (upsert_link refuses self-links).
            let mut stmt = conn.prepare_cached("SELECT dst_type AS t_type, dst_id AS t_id, evidence_count AS strength, relation AS rel FROM clock_links WHERE src_type = ?1 AND src_id = ?2 AND status != 'rejected' UNION ALL SELECT src_type, src_id, evidence_count, relation FROM clock_links WHERE dst_type = ?1 AND dst_id = ?2 AND status != 'rejected' ORDER BY strength DESC, t_type ASC, t_id ASC, rel ASC LIMIT 16")?;
            let rows =
                stmt.query_map(params![seed.target_type, seed.target_id], clock_target_row)?;
            for target in rows.flatten() {
                if seen.insert(target.clone()) {
                    let hop = depth + 1;
                    next.push((target.clone(), hop));
                    out.push((target, hop));
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    Ok(out)
}

const REBUILD_SCANS: &[(&str, &str)] = &[
    (
        "SELECT id, decision, context FROM decisions WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        "decision",
    ),
    (
        "SELECT id, text, source FROM memories WHERE id > ?1 ORDER BY id ASC LIMIT ?2",
        "memory",
    ),
];

fn rebuild_scan(
    conn: &Connection,
    sql: &str,
    target_type: &str,
    batch_size: usize,
) -> rusqlite::Result<usize> {
    let mut projected = 0usize;
    let mut last_id: i64 = 0;
    loop {
        let mut stmt = conn.prepare(sql)?;
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map(params![last_id, batch_size as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<_, _>>()?;
        if rows.is_empty() {
            break;
        }
        for (id, text, extra) in rows {
            last_id = id;
            let mut extras = Vec::new();
            if let Some(val) = extra.as_deref().filter(|c| !c.is_empty()) {
                extras.push(QueryAnchor {
                    kind: AnchorKind::Source,
                    value: normalize_anchor_value(AnchorKind::Source, val),
                    specificity: 1,
                });
            }
            project_target(
                conn,
                &text,
                &extras,
                target_type,
                id,
                ClockOrigin::DeterministicExtract,
                None,
            )?;
            crate::graph::ingest_for_target(conn, &text, target_type, Some(id), None, None);
            projected += 1;
        }
    }
    Ok(projected)
}

pub fn rebuild_clock_projections(conn: &Connection, batch_size: usize) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM clock_anchor_evidence", [])?;
    conn.execute("DELETE FROM clock_links", [])?;
    conn.execute("DELETE FROM clock_anchors", [])?;
    let mut projected = 0usize;
    for (sql, target_type) in REBUILD_SCANS {
        projected += rebuild_scan(conn, sql, target_type, batch_size)?;
    }
    let next = current_generation(conn)?.saturating_add(1).max(1);
    super::set_generation(conn, next)?;
    Ok(projected)
}
