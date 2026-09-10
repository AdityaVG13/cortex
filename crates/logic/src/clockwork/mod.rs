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
mod scope;

pub use anchors::{
    extract_anchors, normalize_anchor_value, Anchor, AnchorKind, MAX_ANCHORS_PER_QUERY,
    MAX_ANCHORS_PER_TRACE,
};
pub use bridge::expand_query_frame;
pub use evidence::{
    direct_domains, independent_support, ClockEvidence, ClockWhy, FilterEvidence, LinkHit,
    TieBreak, WhyAnchor, Witness, WitnessDomain,
};
pub use links::{
    current_generation, lookup_targets_for_anchors, lookup_targets_with_matches,
    migrate_clock_tables, project_target, rebuild_clock_projections, record_used_with,
    reject_used_with, target_anchor_values, traverse_hops, ClockOrigin, ClockRelation, ClockTarget,
    CLOCK_DDL, DERIVED_GENERATION_KEY,
};
pub use morph::{hay_has_lexical, morph_stem, morph_variants, stems_match};
pub use query::{parse_query_frame, query_signature, QueryAnchor, QueryFrame, TemporalMode};
pub use quorum::{
    admit, admit_with_lineage, compare_rank_keys, RankKey, Rankable, RANK_TUPLE_VERSION,
};
pub use scope::{qualify_matches, AnchorScope, RowNamespace};

pub const FTS_CANDIDATE_CAP: usize = 64;
pub const STRONG_ANCHOR_CAP: usize = 64;
pub const ENTITY_GRAPH_CAP: usize = 64;
pub const TASK_CANDIDATE_CAP: usize = 32;
pub const HISTORY_CANDIDATE_CAP: usize = 32;
pub const GRAPH_HOP_CAP: usize = 64;
pub const MAX_GRAPH_HOPS: u8 = 2;
