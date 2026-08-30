//! Deterministic Clock-Quorum Recall types and derived projections.
//!
//! Ranking and admission live here. SQL candidate collection stays in the daemon.

mod anchors;
mod bridge;
mod evidence;
mod links;
mod morph;
mod query;
mod quorum;

pub use anchors::{
    extract_anchors, normalize_anchor_value, Anchor, AnchorKind, MAX_ANCHORS_PER_QUERY,
    MAX_ANCHORS_PER_TRACE,
};
pub use bridge::expand_query_frame;
pub use evidence::{ClockEvidence, ClockWhy, FilterEvidence, LinkHit, TieBreak, WhyAnchor};
pub use links::{
    current_generation, lookup_targets_for_anchors, migrate_clock_tables, project_target,
    rebuild_clock_projections, record_used_with, reject_used_with, traverse_hops, ClockOrigin,
    ClockRelation, ClockTarget, CLOCK_DDL, DERIVED_GENERATION_KEY,
};
pub use morph::{hay_has_lexical, morph_stem, morph_variants, stems_match};
pub use query::{parse_query_frame, query_signature, QueryAnchor, QueryFrame, TemporalMode};
pub use quorum::{admit, compare_rank_keys, RankKey, Rankable};

pub const FTS_CANDIDATE_CAP: usize = 64;
pub const STRONG_ANCHOR_CAP: usize = 64;
pub const ENTITY_GRAPH_CAP: usize = 64;
pub const TASK_CANDIDATE_CAP: usize = 32;
pub const HISTORY_CANDIDATE_CAP: usize = 32;
pub const GRAPH_HOP_CAP: usize = 64;
pub const MAX_GRAPH_HOPS: u8 = 2;
