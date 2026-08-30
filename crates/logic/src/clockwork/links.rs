use super::anchors::{extract_anchors, Anchor, AnchorKind, MAX_ANCHORS_PER_TRACE};
use super::query::QueryAnchor;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::BTreeSet;

pub const DERIVED_GENERATION_KEY: &str = "clock_projection_generation";

pub const CLOCK_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS clock_anchors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    value TEXT NOT NULL,
    display_value TEXT,
    specificity INTEGER NOT NULL,
    UNIQUE (kind, value)
);
CREATE INDEX IF NOT EXISTS idx_clock_anchors_lookup
    ON clock_anchors(kind, value, specificity);
CREATE TABLE IF NOT EXISTS clock_anchor_evidence (
    anchor_id INTEGER NOT NULL,
    target_type TEXT NOT NULL,
    target_id INTEGER NOT NULL,
    origin TEXT NOT NULL,
    evidence_count INTEGER NOT NULL DEFAULT 1,
    first_trace_id INTEGER,
    last_trace_id INTEGER,
    PRIMARY KEY (anchor_id, target_type, target_id, origin),
    FOREIGN KEY (anchor_id) REFERENCES clock_anchors(id)
);
CREATE INDEX IF NOT EXISTS idx_clock_anchor_evidence_target
    ON clock_anchor_evidence(target_type, target_id);
CREATE INDEX IF NOT EXISTS idx_clock_anchor_evidence_last_trace
    ON clock_anchor_evidence(last_trace_id);
CREATE TABLE IF NOT EXISTS clock_links (
    src_type TEXT NOT NULL,
    src_id INTEGER NOT NULL,
    dst_type TEXT NOT NULL,
    dst_id INTEGER NOT NULL,
    relation TEXT NOT NULL,
    evidence_count INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'derived',
    last_trace_id INTEGER,
    last_observed_at TEXT,
    PRIMARY KEY (src_type, src_id, dst_type, dst_id, relation)
);
CREATE INDEX IF NOT EXISTS idx_clock_links_reverse
    ON clock_links(dst_type, dst_id, relation);
CREATE TABLE IF NOT EXISTS clock_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockOrigin {
    Explicit,
    ToolReceipt,
    DeterministicExtract,
    EntityProjection,
    Feedback,
}

impl ClockOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::ToolReceipt => "tool_receipt",
            Self::DeterministicExtract => "deterministic_extract",
            Self::EntityProjection => "entity_projection",
            Self::Feedback => "feedback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockRelation {
    Updates,
    Extends,
    SameGoal,
    SamePath,
    SameSymbol,
    ObservedWith,
    UsedWith,
    CausedBy,
    Supports,
}

impl ClockRelation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Updates => "updates",
            Self::Extends => "extends",
            Self::SameGoal => "same_goal",
            Self::SamePath => "same_path",
            Self::SameSymbol => "same_symbol",
            Self::ObservedWith => "observed_with",
            Self::UsedWith => "used_with",
            Self::CausedBy => "caused_by",
            Self::Supports => "supports",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClockTarget {
    pub target_type: String,
    pub target_id: i64,
}

pub fn migrate_clock_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(CLOCK_DDL)?;
    if current_generation(conn).unwrap_or(0) == 0 {
        set_generation(conn, 1)?;
    }
    Ok(())
}

pub fn current_generation(conn: &Connection) -> rusqlite::Result<i64> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM clock_meta WHERE key = ?1",
            params![DERIVED_GENERATION_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value.and_then(|v| v.parse().ok()).unwrap_or(0))
}

fn set_generation(conn: &Connection, generation: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO clock_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![DERIVED_GENERATION_KEY, generation.to_string()],
    )?;
    Ok(())
}

pub fn project_target(
    conn: &Connection,
    text: &str,
    extra: &[QueryAnchor],
    target_type: &str,
    target_id: i64,
    origin: ClockOrigin,
    trace_id: Option<i64>,
) -> rusqlite::Result<Vec<Anchor>> {
    let mut anchors = extract_anchors(text, extra, MAX_ANCHORS_PER_TRACE);
    for mention in crate::graph::extract_mentions(text) {
        if let Some(anchor) = Anchor::new(AnchorKind::Entity, mention.surface, 2) {
            if !anchors
                .iter()
                .any(|existing| existing.kind == anchor.kind && existing.value == anchor.value)
            {
                anchors.push(anchor);
            }
        }
    }
    persist_anchors(conn, &anchors, target_type, target_id, origin, trace_id)?;
    link_shared_strong_anchors(conn, &anchors, target_type, target_id, trace_id)?;
    Ok(anchors)
}

fn persist_anchors(
    conn: &Connection,
    anchors: &[Anchor],
    target_type: &str,
    target_id: i64,
    origin: ClockOrigin,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    for anchor in anchors {
        conn.execute(
            "INSERT INTO clock_anchors (kind, value, display_value, specificity)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, value) DO UPDATE SET
                specificity = MAX(clock_anchors.specificity, excluded.specificity),
                display_value = COALESCE(clock_anchors.display_value, excluded.display_value)",
            params![
                anchor.kind.as_str(),
                anchor.value,
                anchor.display_value,
                anchor.specificity
            ],
        )?;
        let anchor_id: i64 = conn.query_row(
            "SELECT id FROM clock_anchors WHERE kind = ?1 AND value = ?2",
            params![anchor.kind.as_str(), anchor.value],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO clock_anchor_evidence (anchor_id, target_type, target_id, origin, evidence_count, first_trace_id, last_trace_id)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
             ON CONFLICT(anchor_id, target_type, target_id, origin) DO UPDATE SET
                evidence_count = clock_anchor_evidence.evidence_count + 1,
                last_trace_id = COALESCE(excluded.last_trace_id, clock_anchor_evidence.last_trace_id)",
            params![anchor_id, target_type, target_id, origin.as_str(), trace_id],
        )?;
        if anchor.kind == AnchorKind::Path {
            for ancestor in path_ancestors(&anchor.value) {
                if let Some(parent) = Anchor::new(AnchorKind::Path, ancestor, 2) {
                    conn.execute(
                            "INSERT INTO clock_anchors (kind, value, display_value, specificity)
                             VALUES (?1, ?2, ?3, ?4)
                             ON CONFLICT(kind, value) DO UPDATE SET
                                specificity = MAX(clock_anchors.specificity, excluded.specificity),
                                display_value = COALESCE(clock_anchors.display_value, excluded.display_value)",
                            params![parent.kind.as_str(), parent.value, parent.display_value, parent.specificity],
                        )?;
                    let parent_id: i64 = conn.query_row(
                        "SELECT id FROM clock_anchors WHERE kind = ?1 AND value = ?2",
                        params![parent.kind.as_str(), parent.value],
                        |row| row.get(0),
                    )?;
                    conn.execute(
                            "INSERT INTO clock_anchor_evidence (anchor_id, target_type, target_id, origin, evidence_count, first_trace_id, last_trace_id)
                             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)
                             ON CONFLICT(anchor_id, target_type, target_id, origin) DO UPDATE SET
                                evidence_count = clock_anchor_evidence.evidence_count + 1,
                                last_trace_id = COALESCE(excluded.last_trace_id, clock_anchor_evidence.last_trace_id)",
                            params![parent_id, target_type, target_id, origin.as_str(), trace_id],
                        )?;
                }
            }
        }
    }
    Ok(())
}

fn link_shared_strong_anchors(
    conn: &Connection,
    anchors: &[Anchor],
    target_type: &str,
    target_id: i64,
    trace_id: Option<i64>,
) -> rusqlite::Result<()> {
    for anchor in anchors.iter().filter(|a| a.specificity >= 2) {
        let mut stmt = conn.prepare_cached(
            "SELECT e.target_type, e.target_id
             FROM clock_anchor_evidence e
             JOIN clock_anchors a ON a.id = e.anchor_id
             WHERE a.kind = ?1 AND a.value = ?2
               AND NOT (e.target_type = ?3 AND e.target_id = ?4)
             LIMIT 8",
        )?;
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
    let (left_type, left_id, right_type, right_id) = if (src_type, src_id) <= (dst_type, dst_id) {
        (src_type, src_id, dst_type, dst_id)
    } else {
        (dst_type, dst_id, src_type, src_id)
    };
    conn.execute(
        "INSERT INTO clock_links (src_type, src_id, dst_type, dst_id, relation, evidence_count, status, last_trace_id, last_observed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, 'derived', ?6, datetime('now'))
         ON CONFLICT(src_type, src_id, dst_type, dst_id, relation) DO UPDATE SET
            evidence_count = clock_links.evidence_count + 1,
            last_trace_id = COALESCE(excluded.last_trace_id, clock_links.last_trace_id),
            last_observed_at = excluded.last_observed_at
         WHERE clock_links.status != 'rejected'",
        params![left_type, left_id, right_type, right_id, relation.as_str(), trace_id],
    )?;
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
    let (src_type, src_id, dst_type, dst_id) = if (left.target_type.as_str(), left.target_id)
        <= (right.target_type.as_str(), right.target_id)
    {
        (
            left.target_type.as_str(),
            left.target_id,
            right.target_type.as_str(),
            right.target_id,
        )
    } else {
        (
            right.target_type.as_str(),
            right.target_id,
            left.target_type.as_str(),
            left.target_id,
        )
    };
    conn.execute(
        "UPDATE clock_links SET status = 'rejected'
         WHERE src_type = ?1 AND src_id = ?2 AND dst_type = ?3 AND dst_id = ?4 AND relation = 'used_with'",
        params![src_type, src_id, dst_type, dst_id],
    )?;
    Ok(())
}

pub fn lookup_targets_for_anchors(
    conn: &Connection,
    anchors: &[QueryAnchor],
    limit: usize,
) -> rusqlite::Result<Vec<ClockTarget>> {
    let mut seen: BTreeSet<ClockTarget> = BTreeSet::new();
    for anchor in anchors {
        if anchor.kind == AnchorKind::Path {
            let mut stmt = conn.prepare_cached(
                "SELECT e.target_type, e.target_id
                 FROM clock_anchor_evidence e
                 JOIN clock_anchors a ON a.id = e.anchor_id
                 WHERE a.kind = 'path'
                   AND (a.value = ?1 OR a.value LIKE ?1 || '/%' OR ?1 LIKE a.value || '/%')
                 ORDER BY a.specificity DESC, e.target_type ASC, e.target_id ASC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![anchor.value, limit as i64], |row| {
                Ok(ClockTarget {
                    target_type: row.get(0)?,
                    target_id: row.get(1)?,
                })
            })?;
            for target in rows.flatten() {
                seen.insert(target);
                if seen.len() >= limit {
                    return Ok(seen.into_iter().collect());
                }
            }
            continue;
        }
        let mut stmt = conn.prepare_cached(
            "SELECT e.target_type, e.target_id
             FROM clock_anchor_evidence e
             JOIN clock_anchors a ON a.id = e.anchor_id
             WHERE a.kind = ?1 AND a.value = ?2
             ORDER BY a.specificity DESC, e.target_type ASC, e.target_id ASC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![anchor.kind.as_str(), anchor.value, limit as i64],
            |row| {
                Ok(ClockTarget {
                    target_type: row.get(0)?,
                    target_id: row.get(1)?,
                })
            },
        )?;
        for target in rows.flatten() {
            seen.insert(target);
            if seen.len() >= limit {
                return Ok(seen.into_iter().collect());
            }
        }
    }
    Ok(seen.into_iter().collect())
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
    let max_hops = hops.min(super::MAX_GRAPH_HOPS);
    for _ in 0..max_hops {
        let mut next = Vec::new();
        for (seed, depth) in &frontier {
            if *depth >= max_hops {
                continue;
            }
            let mut stmt = conn.prepare_cached(
                "SELECT dst_type, dst_id FROM clock_links
                 WHERE src_type = ?1 AND src_id = ?2 AND status != 'rejected'
                 UNION ALL
                 SELECT src_type, src_id FROM clock_links
                 WHERE dst_type = ?1 AND dst_id = ?2 AND status != 'rejected'
                 LIMIT 16",
            )?;
            let rows = stmt.query_map(params![seed.target_type, seed.target_id], |row| {
                Ok(ClockTarget {
                    target_type: row.get(0)?,
                    target_id: row.get(1)?,
                })
            })?;
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

pub fn rebuild_clock_projections(conn: &Connection, batch_size: usize) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM clock_anchor_evidence", [])?;
    conn.execute("DELETE FROM clock_links", [])?;
    conn.execute("DELETE FROM clock_anchors", [])?;
    let mut projected = 0usize;
    let mut last_id: i64 = 0;
    loop {
        let mut stmt = conn.prepare(
            "SELECT id, decision, context FROM decisions
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map(params![last_id, batch_size as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(Result::ok)
            .collect();
        if rows.is_empty() {
            break;
        }
        for (id, decision, context) in rows {
            last_id = id;
            let mut extra = Vec::new();
            if let Some(ctx) = context.as_deref().filter(|c| !c.is_empty()) {
                extra.push(QueryAnchor {
                    kind: AnchorKind::Source,
                    value: super::normalize_anchor_value(AnchorKind::Source, ctx),
                    specificity: 1,
                });
            }
            project_target(
                conn,
                &decision,
                &extra,
                "decision",
                id,
                ClockOrigin::DeterministicExtract,
                None,
            )?;
            crate::graph::ingest_for_target(conn, &decision, "decision", Some(id), None, None);
            projected += 1;
        }
    }
    last_id = 0;
    loop {
        let mut stmt = conn.prepare(
            "SELECT id, text, source FROM memories
             WHERE id > ?1
             ORDER BY id ASC
             LIMIT ?2",
        )?;
        let rows: Vec<(i64, String, Option<String>)> = stmt
            .query_map(params![last_id, batch_size as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .filter_map(Result::ok)
            .collect();
        if rows.is_empty() {
            break;
        }
        for (id, text, source) in rows {
            last_id = id;
            let mut extra = Vec::new();
            if let Some(src) = source.as_deref().filter(|c| !c.is_empty()) {
                extra.push(QueryAnchor {
                    kind: AnchorKind::Source,
                    value: super::normalize_anchor_value(AnchorKind::Source, src),
                    specificity: 1,
                });
            }
            project_target(
                conn,
                &text,
                &extra,
                "memory",
                id,
                ClockOrigin::DeterministicExtract,
                None,
            )?;
            crate::graph::ingest_for_target(conn, &text, "memory", Some(id), None, None);
            projected += 1;
        }
    }
    let next = current_generation(conn)
        .unwrap_or(0)
        .saturating_add(1)
        .max(1);
    set_generation(conn, next)?;
    Ok(projected)
}
