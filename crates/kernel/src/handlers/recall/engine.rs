use crate::handlers::contains_ascii_ignore_case;
use crate::state::RuntimeState;
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueryShapeProfile {
    pub exactish: bool,
    pub naturalish: bool,
}
pub fn query_shape_profile(query_text: &str, source_prefix: Option<&str>) -> QueryShapeProfile {
    let trimmed = query_text.trim();
    let token_count = trimmed.split_whitespace().count();
    let char_count = trimmed.chars().count();
    let has_exact_markers = ["\"", "`", "::", "/", "\\"]
        .iter()
        .any(|m| trimmed.contains(m))
        || [".rs", ".ts", ".tsx", ".js", ".py"]
            .iter()
            .any(|ext| contains_ascii_ignore_case(trimmed, ext));
    QueryShapeProfile {
        exactish: has_exact_markers
            || token_count <= 3
            || char_count <= 24
            || source_prefix.is_some(),
        naturalish: token_count >= 8 || char_count >= 56 || trimmed.ends_with('?'),
    }
}
pub const MIN_BUDGET_HEADROOM_TOKENS: usize = 8;
pub const MIN_EXCERPT_CHARS: usize = 24;
pub const MEMORIES_BM25_TEXT_WEIGHT: f64 = 4.6;
pub const MEMORIES_BM25_SOURCE_WEIGHT: f64 = 1.7;
pub const MEMORIES_BM25_TAGS_WEIGHT: f64 = 2.2;
pub const DECISIONS_BM25_DECISION_WEIGHT: f64 = 6.6;
pub const DECISIONS_BM25_CONTEXT_WEIGHT: f64 = 1.0;
pub const BM25_WEIGHT_MIN: f64 = 0.1;
pub const BM25_WEIGHT_MAX: f64 = 12.0;
pub const ENTITY_SIGNAL_OVERLAP_WEIGHT: f64 = 0.10;
pub const ENTITY_SIGNAL_MATCH_WEIGHT: f64 = 0.01;
pub const ENTITY_SIGNAL_MAX_BOOST: f64 = 0.12;
pub const ALIGNMENT_EXACT_BONUS_MAX: f64 = 0.08;
pub const ALIGNMENT_COVERAGE_BONUS_MAX: f64 = 0.07;
pub const ALIGNMENT_BOOST_MAX: f64 = 0.15;
pub const TEMPORAL_INTENT_MULTIPLIER_RANGE: f64 = 0.16;
pub const BENCHMARK_SOURCE_SCOPE_PREFIX: &str = "amb::";
pub const DEFAULT_RECALL_BUDGET_FAST: usize = 180;
pub const DEFAULT_RECALL_BUDGET_BALANCED: usize = 320;
pub const DEFAULT_RECALL_BUDGET_DEEP: usize = 560;
pub const DEFAULT_RECALL_LATENCY_FAST_MS: u128 = 900;
pub const DEFAULT_RECALL_LATENCY_BALANCED_MS: u128 = 1800;
pub const DEFAULT_RECALL_LATENCY_DEEP_MS: u128 = 3500;
pub const BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD: f64 = 0.84;
pub const BUDGET_PRESSURE_EARLY_STOP_THRESHOLD: f64 = 0.82;
#[derive(Clone, Copy, Debug)]
pub struct Bm25Weights {
    pub memories_text: f64,
    pub memories_source: f64,
    pub memories_tags: f64,
    pub decisions_text: f64,
    pub decisions_context: f64,
}
pub static BM25_WEIGHTS: OnceLock<Bm25Weights> = OnceLock::new();
pub fn parse_bm25_weight(raw: Option<String>, default: f64) -> f64 {
    raw.and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
        .clamp(BM25_WEIGHT_MIN, BM25_WEIGHT_MAX)
}
pub fn bm25_weights_from_resolver(
    mut resolve_env: impl FnMut(&str) -> Option<String>,
) -> Bm25Weights {
    let mut w = |key, default| parse_bm25_weight(resolve_env(key), default);
    Bm25Weights {
        memories_text: w("CORTEX_BM25_MEM_TEXT_WEIGHT", MEMORIES_BM25_TEXT_WEIGHT),
        memories_source: w("CORTEX_BM25_MEM_SOURCE_WEIGHT", MEMORIES_BM25_SOURCE_WEIGHT),
        memories_tags: w("CORTEX_BM25_MEM_TAGS_WEIGHT", MEMORIES_BM25_TAGS_WEIGHT),
        decisions_text: w(
            "CORTEX_BM25_DECISION_WEIGHT",
            DECISIONS_BM25_DECISION_WEIGHT,
        ),
        decisions_context: w("CORTEX_BM25_CONTEXT_WEIGHT", DECISIONS_BM25_CONTEXT_WEIGHT),
    }
}
pub fn bm25_weights() -> &'static Bm25Weights {
    BM25_WEIGHTS.get_or_init(|| bm25_weights_from_resolver(|name| std::env::var(name).ok()))
}
#[derive(Clone, Debug)]
pub struct RecallItem {
    pub source: String,
    pub relevance: f64,
    pub excerpt: String,
    pub method: String,
    pub tokens: Option<usize>,
    pub entropy: Option<f64>,
    pub family_members: Vec<String>,
    pub collapsed_sources: Vec<String>,
    pub collapsed_source_scores: Vec<(String, f64)>,
    pub base_relevance: f64,
    pub alignment_boost: f64,
    pub entity_boost: f64,
    pub temporal_multiplier: f64,
    pub entropy_boost: f64,
    pub feedback_boost: f64,
    pub crystal_boost: f64,
    pub clock_why: Option<Value>,
    pub status: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}
impl RecallItem {
    pub fn new_with_why(source: String, relevance: f64, excerpt: String, method: String) -> Self {
        Self {
            source,
            relevance,
            excerpt,
            method,
            tokens: None,
            entropy: None,
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
            base_relevance: relevance,
            alignment_boost: 0.0,
            entity_boost: 0.0,
            temporal_multiplier: 1.0,
            entropy_boost: 0.0,
            feedback_boost: 0.0,
            crystal_boost: 0.0,
            clock_why: None,
            status: None,
            valid_from: None,
            valid_until: None,
        }
    }
    pub fn why_json(&self) -> serde_json::Value {
        if let Some(why) = &self.clock_why {
            return why.clone();
        }
        serde_json::json!({"method":self.method,"baseRelevance":round4(self.base_relevance),"finalRelevance":round4(self.relevance),"boosts":{"alignment":round4(self.alignment_boost),"entity":round4(self.entity_boost),"temporal":round4(self.temporal_multiplier),"entropy":round4(self.entropy_boost),"feedback":round4(self.feedback_boost),"crystal":round4(self.crystal_boost)},"statusFilters":["archived","superseded"],"temporalFilters":["validFrom","validUntil","expiresAt"]})
    }
}
pub fn shannon_entropy(text: &str) -> f64 {
    if text.is_empty() {
        return 0.0;
    }
    let mut freq = [0u32; 256];
    let len = text.len() as f64;
    for &b in text.as_bytes() {
        freq[b as usize] += 1;
    }
    let mut h = 0.0f64;
    for &count in &freq {
        if count > 0 {
            let p = count as f64 / len;
            h -= p * p.log2();
        }
    }
    h
}
#[derive(Clone)]
pub struct SearchCandidate {
    pub source: String,
    pub excerpt: String,
    pub alignment: (usize, usize),
    pub relevance: f64,
    pub matched_keywords: i64,
    pub score: f64,
    pub ts: i64,
    pub owner_id: Option<i64>,
    pub visibility: Option<String>,
}
#[derive(Clone)]
pub struct SemanticCandidate {
    pub source: String,
    pub excerpt: String,
    pub relevance: f64,
    pub importance: f64,
    pub ts: i64,
}
pub struct RecallWithVectorTrace {
    pub ranked: Vec<RecallItem>,
    pub semantic_route: Value,
}
#[derive(Clone)]
pub struct RecallContext {
    pub caller_id: Option<i64>,
    pub team_mode: bool,
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
    pub goal_id: Option<i64>,
    pub session_id: Option<String>,
    pub as_of: Option<String>,
    /// Search the cold/archived partition too (history/audit profiles).
    pub include_cold: bool,
}
impl RecallContext {
    pub fn new(caller_id: Option<i64>, team_mode: bool) -> Self {
        Self {
            caller_id,
            team_mode,
            paths: Vec::new(),
            symbols: Vec::new(),
            goal_id: None,
            session_id: None,
            as_of: None,
            include_cold: false,
        }
    }
    pub fn from_caller(caller_id: Option<i64>, state: &RuntimeState) -> Self {
        Self::new(caller_id, state.team_mode)
    }
    #[allow(dead_code)]
    pub fn from_state(state: &RuntimeState) -> Self {
        Self::new(state.default_owner_id, state.team_mode)
    }
    #[allow(dead_code)]
    pub fn solo() -> Self {
        Self::new(None, false)
    }
}
#[path = "engine_text.rs"]
mod text;
/// Prefix pattern for `LIKE ? ESCAPE '\'`. MCP `source_prefix` is attacker-
/// controlled; `%` `_` `\` must be literals or the FTS `LIMIT` fills from
/// every source.
pub(crate) use crate::db::like_prefix;
pub use text::{
    dedup_preserve_order, is_missing_team_visibility_columns, is_visible,
    normalize_collapsed_source_rank, source_matches_prefix,
};
pub(crate) use text::{quote_fts_match_term, strip_fts_operators};
#[path = "engine_policy.rs"]
mod policy;
pub use policy::*;
#[derive(Deserialize, Default)]
pub struct RecallQuery {
    pub q: Option<String>,
    pub k: Option<usize>,
    pub budget: Option<usize>,
    pub agent: Option<String>,
    pub source_prefix: Option<String>,
    pub pool_k: Option<usize>,
    #[serde(alias = "policyMode")]
    pub policy_mode: Option<String>,
}
pub fn apply_recall_ranking_boosts(
    items: &mut [RecallItem],
    query_text: &str,
    entropy_mult: f64,
    entropy_cap: f64,
) {
    let query_entities = query_entity_terms(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let query_focus_term_count = alignment_profile.term_count;
    for item in items {
        let h = shannon_entropy(&item.excerpt);
        item.entropy = Some(round4(h));
        bump_relevance(item, ((h - 3.5).max(0.0) * entropy_mult).min(entropy_cap));
        if !query_entities.is_empty() {
            let haystack = format!("{} {}", item.source, item.excerpt);
            let (entity_matches, entity_overlap) =
                entity_alignment_metrics_with_terms(&haystack, &query_entities);
            let entity_boost = entity_signal_boost(entity_matches, entity_overlap);
            if entity_boost > 0.0 {
                bump_relevance(item, entity_boost);
            }
        }
        let alignment_boost = query_alignment_boost_with_profile(
            &item.source,
            &item.excerpt,
            &alignment_profile,
            query_focus_term_count,
        );
        if alignment_boost > 0.0 {
            bump_relevance(item, alignment_boost);
        }
    }
}

fn bump_relevance(item: &mut RecallItem, boost: f64) {
    item.relevance = round4(item.relevance * (1.0 + boost));
}
#[path = "engine_rank.rs"]
mod rank;
pub use rank::*;

#[path = "engine_search.rs"]
mod search;
pub(crate) use search::{SearchTableKind, search_source_key};
pub use search::{search_decisions, search_memories};

#[path = "engine_support.rs"]
mod support;
pub use support::*;

#[path = "engine_unfold.rs"]
mod unfold;
pub use unfold::*;

#[path = "engine_execution.rs"]
mod execution;
pub use execution::*;

#[path = "engine_as_of.rs"]
mod as_of;
pub use as_of::*;
#[path = "engine_clockwork.rs"]
mod clockwork;
pub use clockwork::{
    ROUTE_QUOTAS, RouteTrace, clock_health_payload, run_clock_quorum_recall, take_last_route_trace,
};
pub(crate) use clockwork::{
    explicit_paths_by_target, jaccard_path_sets, normalize_query_paths, read_path_sets,
};
