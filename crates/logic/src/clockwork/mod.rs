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

pub(crate) use anchors::is_stop_word;
pub use anchors::{
    Anchor, AnchorKind, MAX_ANCHORS_PER_QUERY, MAX_ANCHORS_PER_TRACE, extract_anchors,
    normalize_anchor_value, strip_path_globs,
};
pub use bridge::expand_query_frame;
pub use evidence::{
    ClockEvidence, ClockWhy, FilterEvidence, LinkHit, TieBreak, WhyAnchor, Witness, WitnessDomain,
    direct_domains, independent_support,
};
pub use links::{
    CLOCK_DDL, ClockOrigin, ClockRelation, ClockTarget, DERIVED_GENERATION_KEY, current_generation,
    lookup_targets_for_anchors, lookup_targets_with_matches, migrate_clock_tables, project_target,
    rebuild_clock_projections, record_used_with, reject_used_with, target_anchor_values,
    traverse_hops,
};
pub use morph::{hay_has_lexical, morph_stem, morph_variants, stems_match};
pub use query::{
    MAX_QUERY_BYTES, MAX_QUERY_TOKENS, QueryAnchor, QueryFrame, TemporalMode, bound_query_text,
    parse_query_frame, query_signature,
};
pub use quorum::{
    RANK_TUPLE_VERSION, RankKey, Rankable, admit, admit_with_lineage, compare_rank_keys,
};
pub use scope::{AnchorScope, RowNamespace, qualify_matches};

pub const FTS_CANDIDATE_CAP: usize = 64;
pub const STRONG_ANCHOR_CAP: usize = 64;
pub const ENTITY_GRAPH_CAP: usize = 64;
pub const TASK_CANDIDATE_CAP: usize = 32;
pub const HISTORY_CANDIDATE_CAP: usize = 32;
pub const GRAPH_HOP_CAP: usize = 64;
pub const MAX_GRAPH_HOPS: u8 = 2;
