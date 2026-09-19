use super::anchors::{Anchor, AnchorKind, MAX_ANCHORS_PER_TRACE, extract_anchors};
use super::query::QueryAnchor;
use rusqlite::{Connection, OptionalExtension, params};

pub const DERIVED_GENERATION_KEY: &str = "clock_projection_generation";

pub const CLOCK_DDL: &str = "CREATE TABLE IF NOT EXISTS clock_anchors (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, value TEXT NOT NULL, display_value TEXT, specificity INTEGER NOT NULL, UNIQUE (kind, value)); CREATE INDEX IF NOT EXISTS idx_clock_anchors_lookup ON clock_anchors(kind, value, specificity); CREATE TABLE IF NOT EXISTS clock_anchor_evidence (anchor_id INTEGER NOT NULL, target_type TEXT NOT NULL, target_id INTEGER NOT NULL, origin TEXT NOT NULL, evidence_count INTEGER NOT NULL DEFAULT 1, first_trace_id INTEGER, last_trace_id INTEGER, PRIMARY KEY (anchor_id, target_type, target_id, origin), FOREIGN KEY (anchor_id) REFERENCES clock_anchors(id)); CREATE INDEX IF NOT EXISTS idx_clock_anchor_evidence_target ON clock_anchor_evidence(target_type, target_id); CREATE INDEX IF NOT EXISTS idx_clock_anchor_evidence_last_trace ON clock_anchor_evidence(last_trace_id); CREATE TABLE IF NOT EXISTS clock_links (src_type TEXT NOT NULL, src_id INTEGER NOT NULL, dst_type TEXT NOT NULL, dst_id INTEGER NOT NULL, relation TEXT NOT NULL, evidence_count INTEGER NOT NULL DEFAULT 1, status TEXT NOT NULL DEFAULT 'derived', last_trace_id INTEGER, last_observed_at TEXT, PRIMARY KEY (src_type, src_id, dst_type, dst_id, relation)); CREATE INDEX IF NOT EXISTS idx_clock_links_reverse ON clock_links(dst_type, dst_id, relation); CREATE TABLE IF NOT EXISTS clock_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

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
    match current_generation(conn)? {
        0 => set_generation(conn, 1)?,
        _ => {}
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
    conn.execute("INSERT INTO clock_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![DERIVED_GENERATION_KEY, generation.to_string()])?;
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
    persist::persist_anchors(conn, &anchors, target_type, target_id, origin, trace_id)?;
    persist::link_shared_strong_anchors(conn, &anchors, target_type, target_id, trace_id)?;
    Ok(anchors)
}

mod persist;
pub use persist::{record_used_with, reject_used_with};

mod lookup;
pub use lookup::{
    lookup_targets_for_anchors, lookup_targets_with_matches, rebuild_clock_projections,
    target_anchor_values, traverse_hops,
};
