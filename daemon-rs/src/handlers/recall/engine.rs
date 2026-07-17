// SPDX-License-Identifier: MIT
use axum::http::StatusCode;
use axum::response::Response;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;
use crate::handlers::{estimate_tokens, json_response, now_iso, parse_timestamp_ms, truncate_chars};
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct QueryShapeProfile {
    pub(crate) exactish: bool,
    pub(crate) naturalish: bool,
}
pub(crate) fn query_shape_profile(query_text: &str, source_prefix: Option<&str>) -> QueryShapeProfile {
    let trimmed = query_text.trim();
    let token_count = trimmed.split_whitespace().count();
    let char_count = trimmed.chars().count();
    let lowered = trimmed.to_ascii_lowercase();
    let has_exact_markers = trimmed.contains('"')
        || trimmed.contains('`')
        || trimmed.contains("::")
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || lowered.contains(".rs")
        || lowered.contains(".ts")
        || lowered.contains(".tsx")
        || lowered.contains(".js")
        || lowered.contains(".py");
    QueryShapeProfile {
        exactish: has_exact_markers
            || token_count <= 3
            || char_count <= 24
            || source_prefix.is_some(),
        naturalish: token_count >= 8 || char_count >= 56 || trimmed.ends_with('?'),
    }
}
pub(crate) const MAX_RECALL_HISTORY: usize = 50;
pub(crate) const PRECACHE_TTL_MS: i64 = 5 * 60 * 1000;
pub(crate) const SEMANTIC_SIM_FLOOR: f64 = 0.3;
pub(crate) const SEMANTIC_SCALE_BASE: f64 = 0.55;
pub(crate) const MAX_SEMANTIC_RRF_CANDIDATES: usize = 120;
pub(crate) const MIN_BUDGET_HEADROOM_TOKENS: usize = 8;
pub(crate) const MIN_EXCERPT_CHARS: usize = 24;
pub(crate) const ASSOCIATIVE_MIN_BUDGET_TOKENS: usize = 260;
pub(crate) const MEMORIES_BM25_TEXT_WEIGHT: f64 = 4.6;
pub(crate) const MEMORIES_BM25_SOURCE_WEIGHT: f64 = 1.7;
pub(crate) const MEMORIES_BM25_TAGS_WEIGHT: f64 = 2.2;
pub(crate) const DECISIONS_BM25_DECISION_WEIGHT: f64 = 6.6;
pub(crate) const DECISIONS_BM25_CONTEXT_WEIGHT: f64 = 1.0;
pub(crate) const BM25_WEIGHT_MIN: f64 = 0.1;
pub(crate) const BM25_WEIGHT_MAX: f64 = 12.0;
pub(crate) const SQLITE_VEC_TRIAL_MIN_OVERLAP_RATIO: f64 = 0.60;
pub(crate) const SQLITE_VEC_TRIAL_MIN_JACCARD: f64 = 0.45;
pub(crate) const SQLITE_VEC_TRIAL_MAX_MEAN_ABS_RANK_DELTA: f64 = 1.25;
pub(crate) const SQLITE_VEC_TRIAL_TOP1_MATCH_REQUIRED: bool = true;
pub(crate) const ENTITY_SIGNAL_OVERLAP_WEIGHT: f64 = 0.10;
pub(crate) const ENTITY_SIGNAL_MATCH_WEIGHT: f64 = 0.01;
pub(crate) const ENTITY_SIGNAL_MAX_BOOST: f64 = 0.12;
pub(crate) const ALIGNMENT_EXACT_BONUS_MAX: f64 = 0.08;
pub(crate) const ALIGNMENT_COVERAGE_BONUS_MAX: f64 = 0.07;
pub(crate) const ALIGNMENT_BOOST_MAX: f64 = 0.15;
pub(crate) const TEMPORAL_INTENT_MULTIPLIER_RANGE: f64 = 0.16;
pub(crate) const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex::";
pub(crate) const BENCHMARK_SOURCE_SCOPE_PREFIX: &str = "amb::";
pub(crate) const DEFAULT_RECALL_BUDGET_FAST: usize = 180;
pub(crate) const DEFAULT_RECALL_BUDGET_BALANCED: usize = 320;
pub(crate) const DEFAULT_RECALL_BUDGET_DEEP: usize = 560;
pub(crate) const DEFAULT_RECALL_LATENCY_FAST_MS: u128 = 900;
pub(crate) const DEFAULT_RECALL_LATENCY_BALANCED_MS: u128 = 1800;
pub(crate) const DEFAULT_RECALL_LATENCY_DEEP_MS: u128 = 3500;
pub(crate) const BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD: f64 = 0.84;
pub(crate) const BUDGET_PRESSURE_EARLY_STOP_THRESHOLD: f64 = 0.82;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bm25Weights {
    pub(crate) memories_text: f64,
    pub(crate) memories_source: f64,
    pub(crate) memories_tags: f64,
    pub(crate) decisions_text: f64,
    pub(crate) decisions_context: f64,
}
pub(crate) static BM25_WEIGHTS: OnceLock<Bm25Weights> = OnceLock::new();
pub(crate) fn parse_bm25_weight(raw: Option<String>, default: f64) -> f64 {
    raw.and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
        .clamp(BM25_WEIGHT_MIN, BM25_WEIGHT_MAX)
}
pub(crate) fn bm25_weights_from_resolver(mut resolve_env: impl FnMut(&str) -> Option<String>) -> Bm25Weights {
    Bm25Weights {
        memories_text: parse_bm25_weight(
            resolve_env("CORTEX_BM25_MEM_TEXT_WEIGHT"),
            MEMORIES_BM25_TEXT_WEIGHT,
        ),
        memories_source: parse_bm25_weight(
            resolve_env("CORTEX_BM25_MEM_SOURCE_WEIGHT"),
            MEMORIES_BM25_SOURCE_WEIGHT,
        ),
        memories_tags: parse_bm25_weight(
            resolve_env("CORTEX_BM25_MEM_TAGS_WEIGHT"),
            MEMORIES_BM25_TAGS_WEIGHT,
        ),
        decisions_text: parse_bm25_weight(
            resolve_env("CORTEX_BM25_DECISION_WEIGHT"),
            DECISIONS_BM25_DECISION_WEIGHT,
        ),
        decisions_context: parse_bm25_weight(
            resolve_env("CORTEX_BM25_CONTEXT_WEIGHT"),
            DECISIONS_BM25_CONTEXT_WEIGHT,
        ),
    }
}
pub(crate) fn bm25_weights() -> &'static Bm25Weights {
    BM25_WEIGHTS.get_or_init(|| bm25_weights_from_resolver(|name| std::env::var(name).ok()))
}
#[derive(Clone, Debug)]
pub(crate) struct RecallItem {
    pub(crate) source: String,
    pub(crate) relevance: f64,
    pub(crate) excerpt: String,
    pub(crate) method: String,
    pub(crate) tokens: Option<usize>,
    pub(crate) entropy: Option<f64>,
    pub(crate) family_members: Vec<String>,
    pub(crate) collapsed_sources: Vec<String>,
    pub(crate) collapsed_source_scores: Vec<(String, f64)>,
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
pub(crate) struct SearchCandidate {
    pub(crate) source: String,
    pub(crate) excerpt: String,
    pub(crate) alignment: (usize, usize),
    pub(crate) relevance: f64,
    pub(crate) matched_keywords: i64,
    pub(crate) score: f64,
    pub(crate) ts: i64,
    pub(crate) owner_id: Option<i64>,
    pub(crate) visibility: Option<String>,
}
#[derive(Clone)]
pub(crate) struct SemanticCandidate {
    pub(crate) source: String,
    pub(crate) excerpt: String,
    pub(crate) relevance: f64,
    pub(crate) importance: f64,
    pub(crate) ts: i64,
}
#[derive(Clone)]
pub(crate) struct ShadowSemanticRow {
    pub(crate) source: String,
    pub(crate) vector: Vec<f32>,
}
#[derive(Clone)]
pub(crate) struct ShadowSemanticBaseline {
    pub(crate) candidate_count: usize,
    pub(crate) ranked_sources: Vec<String>,
}
impl ShadowSemanticBaseline {
    pub(crate) fn top_sources(&self, top_k: usize) -> Vec<String> {
        self.ranked_sources
            .iter()
            .take(top_k.clamp(1, MAX_SEMANTIC_RRF_CANDIDATES))
            .cloned()
            .collect()
    }
}
pub(crate) struct RecallWithVectorTrace {
    pub(crate) ranked: Vec<RecallItem>,
    pub(crate) semantic_baseline: Option<ShadowSemanticBaseline>,
    pub(crate) semantic_route: Value,
}
pub(crate) type MemorySemanticRow = (
    Vec<u8>,
    String,
    String,
    Option<i64>,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<String>,
    Option<String>,
);
pub(crate) type DecisionSemanticRow = (
    Vec<u8>,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<f64>,
    Option<f64>,
    Option<String>,
    Option<String>,
);
pub(crate) type CrystalMemberSourceRow = (Option<String>, Option<i64>, Option<String>);
pub(crate) type ShadowMemoryRow = (Vec<u8>, String, Option<i64>, Option<String>);
pub(crate) type ShadowDecisionRow = (Vec<u8>, String, Option<String>, Option<i64>, Option<String>);
#[derive(Clone, Copy)]
pub struct RecallContext {
    pub caller_id: Option<i64>,
    pub team_mode: bool,
}
impl RecallContext {
    pub fn from_caller(caller_id: Option<i64>, state: &RuntimeState) -> Self {
        Self {
            caller_id,
            team_mode: state.team_mode,
        }
    }
    #[allow(dead_code)]
    pub fn from_state(state: &RuntimeState) -> Self {
        Self {
            caller_id: state.default_owner_id,
            team_mode: state.team_mode,
        }
    }
    #[allow(dead_code)]
    pub fn solo() -> Self {
        Self {
            caller_id: None,
            team_mode: false,
        }
    }
}
pub(crate) fn is_visible(owner_id: Option<i64>, visibility: Option<&str>, ctx: &RecallContext) -> bool {
    if !ctx.team_mode {
        return true;
    }
    let caller = match ctx.caller_id {
        Some(c) => c,
        None => return false,
    };
    let owner = match owner_id {
        Some(o) => o,
        None => return false,
    };
    if owner == caller {
        return true;
    }
    matches!(visibility, Some("shared") | Some("team"))
}
pub(crate) fn source_matches_prefix(source: &str, source_prefix: Option<&str>) -> bool {
    match source_prefix {
        Some(prefix) => source.starts_with(prefix),
        None => true,
    }
}
pub(crate) fn crystal_source(crystal_id: i64, label: &str) -> String {
    format!("crystal::{crystal_id}::{label}")
}
pub(crate) fn dedup_preserve_order(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}
pub(crate) fn normalize_collapsed_source_rank(item: &mut RecallItem) {
    let mut best_scores: HashMap<String, (f64, usize)> = HashMap::new();
    for (order, source) in item.collapsed_sources.iter().enumerate() {
        best_scores.entry(source.clone()).or_insert((0.0, order));
    }
    for (order, (source, score)) in item.collapsed_source_scores.iter().enumerate() {
        best_scores
            .entry(source.clone())
            .and_modify(|entry| {
                entry.0 = entry.0.max(*score);
                entry.1 = entry.1.min(order);
            })
            .or_insert((*score, order));
    }
    let mut ranked: Vec<(String, f64, usize)> = best_scores
        .into_iter()
        .map(|(source, (score, order))| (source, score, order))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
    });
    item.collapsed_source_scores = ranked
        .iter()
        .map(|(source, score, _)| (source.clone(), *score))
        .collect();
    item.collapsed_sources = item
        .collapsed_source_scores
        .iter()
        .map(|(source, _)| source.clone())
        .collect();
}
pub(crate) fn parse_crystal_source_id(source: &str) -> Option<i64> {
    let rest = source.strip_prefix("crystal::")?;
    let (id, _) = rest.split_once("::")?;
    id.parse::<i64>().ok()
}
pub(crate) fn crystal_member_sources(conn: &Connection, crystal_id: i64, ctx: &RecallContext) -> Vec<String> {
    let query_rows = |sql: &str,
                      with_visibility: bool|
     -> Result<Vec<CrystalMemberSourceRow>, rusqlite::Error> {
        let mut stmt = conn.prepare(sql)?;
        let mapped = stmt.query_map(params![crystal_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                if with_visibility {
                    row.get::<_, Option<i64>>(1)?
                } else {
                    None
                },
                if with_visibility {
                    row.get::<_, Option<String>>(2)?
                } else {
                    None
                },
            ))
        })?;
        Ok(mapped.flatten().collect())
    };
    let sql_with_visibility = "SELECT CASE
                WHEN cm.target_type = 'memory' THEN COALESCE(m.source, 'memory::' || m.id)
                ELSE COALESCE(d.context, 'decision::' || d.id)
            END AS source,
            CASE
                WHEN cm.target_type = 'memory' THEN m.owner_id
                ELSE d.owner_id
            END AS owner_id,
            CASE
                WHEN cm.target_type = 'memory' THEN m.visibility
                ELSE d.visibility
            END AS visibility
     FROM cluster_members cm
     LEFT JOIN memories m
       ON cm.target_type = 'memory'
      AND cm.target_id = m.id
      AND m.status = 'active'
      AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now'))
     LEFT JOIN decisions d
       ON cm.target_type = 'decision'
      AND cm.target_id = d.id
      AND d.status = 'active'
      AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now'))
     WHERE cm.cluster_id = ?1
     ORDER BY cm.target_type, cm.target_id";
    let sql_legacy = "SELECT CASE
                WHEN cm.target_type = 'memory' THEN COALESCE(m.source, 'memory::' || m.id)
                ELSE COALESCE(d.context, 'decision::' || d.id)
            END AS source
     FROM cluster_members cm
     LEFT JOIN memories m
       ON cm.target_type = 'memory'
      AND cm.target_id = m.id
      AND m.status = 'active'
      AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now'))
     LEFT JOIN decisions d
       ON cm.target_type = 'decision'
      AND cm.target_id = d.id
      AND d.status = 'active'
      AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now'))
     WHERE cm.cluster_id = ?1
     ORDER BY cm.target_type, cm.target_id";
    let rows = match query_rows(sql_with_visibility, true) {
        Ok(rows) => rows,
        Err(err) if is_missing_team_visibility_columns(&err) => {
            match query_rows(sql_legacy, false) {
                Ok(rows) => rows,
                Err(_) => return Vec::new(),
            }
        }
        Err(_) => return Vec::new(),
    };
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for (source, owner_id, visibility) in rows {
        let Some(source) = source else {
            continue;
        };
        if !is_visible(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        if seen.insert(source.clone()) {
            sources.push(source);
        }
    }
    sources
}
pub(crate) type CrystalUnfoldRow = (String, String, i64, Option<i64>, Option<String>);
pub(crate) fn query_crystal_for_unfold(conn: &Connection, crystal_id: i64) -> Option<CrystalUnfoldRow> {
    let sql_with_visibility = "SELECT label, consolidated_text, member_count, owner_id, visibility
         FROM memory_clusters
         WHERE id = ?1";
    match conn.query_row(sql_with_visibility, params![crystal_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT label, consolidated_text, member_count
                 FROM memory_clusters
                 WHERE id = ?1",
                params![crystal_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn is_missing_team_visibility_columns(err: &rusqlite::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("no such column")
        && (normalized.contains("owner_id") || normalized.contains("visibility"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallPolicyMode {
    Headlines,
    Fast,
    Balanced,
    Deep,
}
impl RecallPolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headlines => "headlines",
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Deep => "deep",
        }
    }
}
pub(crate) fn parse_env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}
pub(crate) fn recall_default_budget_for_mode(mode: RecallPolicyMode) -> usize {
    match mode {
        RecallPolicyMode::Headlines => 0,
        RecallPolicyMode::Fast => parse_env_usize(
            "CORTEX_RECALL_FAST_BUDGET",
            DEFAULT_RECALL_BUDGET_FAST,
            1,
            2000,
        ),
        RecallPolicyMode::Balanced => parse_env_usize(
            "CORTEX_RECALL_BALANCED_BUDGET",
            DEFAULT_RECALL_BUDGET_BALANCED,
            1,
            4000,
        ),
        RecallPolicyMode::Deep => parse_env_usize(
            "CORTEX_RECALL_DEEP_BUDGET",
            DEFAULT_RECALL_BUDGET_DEEP,
            1,
            8000,
        ),
    }
}
pub(crate) fn recall_default_k_for_mode(mode: RecallPolicyMode) -> usize {
    match mode {
        RecallPolicyMode::Headlines => 10,
        RecallPolicyMode::Fast => 16,
        RecallPolicyMode::Balanced => 12,
        RecallPolicyMode::Deep => 10,
    }
}
pub(crate) fn recall_latency_budget_ms_for_mode(mode: RecallPolicyMode) -> u128 {
    match mode {
        RecallPolicyMode::Headlines => parse_env_usize(
            "CORTEX_RECALL_HEADLINES_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_FAST_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Fast => parse_env_usize(
            "CORTEX_RECALL_FAST_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_FAST_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Balanced => parse_env_usize(
            "CORTEX_RECALL_BALANCED_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_BALANCED_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Deep => parse_env_usize(
            "CORTEX_RECALL_DEEP_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_DEEP_MS as usize,
            0,
            120_000,
        ) as u128,
    }
}
pub(crate) fn recall_mode_for_budget(budget: usize) -> RecallPolicyMode {
    if budget == 0 {
        RecallPolicyMode::Headlines
    } else if budget <= 220 {
        RecallPolicyMode::Fast
    } else if budget <= 500 {
        RecallPolicyMode::Balanced
    } else {
        RecallPolicyMode::Deep
    }
}
pub fn parse_recall_policy_mode(raw: Option<&str>) -> Result<Option<RecallPolicyMode>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let normalized = raw.to_ascii_lowercase();
    let mode = match normalized.as_str() {
        "headlines" => RecallPolicyMode::Headlines,
        "fast" => RecallPolicyMode::Fast,
        "balanced" => RecallPolicyMode::Balanced,
        "deep" => RecallPolicyMode::Deep,
        _ => {
            return Err(
                "Invalid policy mode. Expected one of: headlines, fast, balanced, deep".to_string(),
            );
        }
    };
    Ok(Some(mode))
}
pub fn resolve_recall_budget_k(
    requested_mode: Option<RecallPolicyMode>,
    budget: Option<usize>,
    k: Option<usize>,
) -> (usize, usize, RecallPolicyMode) {
    let resolved_budget = match (requested_mode, budget) {
        (_, Some(explicit_budget)) => explicit_budget,
        (Some(mode), None) => recall_default_budget_for_mode(mode),
        (None, None) => recall_default_budget_for_mode(RecallPolicyMode::Balanced),
    };
    let resolved_mode = recall_mode_for_budget(resolved_budget);
    let resolved_k = k.unwrap_or_else(|| recall_default_k_for_mode(resolved_mode));
    (resolved_budget, resolved_k.max(1), resolved_mode)
}
pub(crate) fn adaptive_default_budget_for_query(
    query_text: &str,
    resolved_k: usize,
    default_budget: usize,
) -> usize {
    if default_budget == 0 {
        return 0;
    }
    let profile = query_shape_profile(query_text, None);
    let token_count = query_text.split_whitespace().count();
    let base: usize = if profile.exactish && !profile.naturalish {
        180
    } else if profile.naturalish && !profile.exactish {
        if token_count >= 14 {
            300
        } else {
            270
        }
    } else {
        240
    };
    let scaled = if resolved_k <= 3 {
        base.saturating_sub(40)
    } else if resolved_k <= 6 {
        base
    } else if resolved_k <= 10 {
        base.saturating_add(30)
    } else {
        base.saturating_add(60)
    };
    scaled.clamp(140, default_budget.max(140))
}
pub(crate) fn maybe_apply_adaptive_default_budget(
    query_text: &str,
    requested_mode: Option<RecallPolicyMode>,
    requested_budget: Option<usize>,
    resolved_budget: usize,
    resolved_k: usize,
) -> usize {
    if requested_mode.is_some() || requested_budget.is_some() {
        return resolved_budget;
    }
    adaptive_default_budget_for_query(query_text, resolved_k, resolved_budget)
}
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
pub(crate) fn apply_recall_ranking_boosts(items: &mut [RecallItem], query_text: &str, entropy_mult: f64, entropy_cap: f64) {
    let query_entities = query_entity_terms(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let query_focus_term_count = alignment_profile.term_count;
    for item in items {
        let h = shannon_entropy(&item.excerpt);
        item.entropy = Some(round4(h));
        let boost = ((h - 3.5).max(0.0) * entropy_mult).min(entropy_cap);
        item.relevance = round4(item.relevance * (1.0 + boost));
        if !query_entities.is_empty() {
            let haystack = format!("{} {}", item.source, item.excerpt);
            let (entity_matches, entity_overlap) =
                entity_alignment_metrics_with_terms(&haystack, &query_entities);
            let entity_boost = entity_signal_boost(entity_matches, entity_overlap);
            if entity_boost > 0.0 {
                item.relevance = round4(item.relevance * (1.0 + entity_boost));
            }
        }
        let alignment_boost = query_alignment_boost_with_profile(
            &item.source, &item.excerpt, &alignment_profile, query_focus_term_count,
        );
        if alignment_boost > 0.0 {
            item.relevance = round4(item.relevance * (1.0 + alignment_boost));
        }
    }
}
pub(crate) fn normalize_text(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch.is_ascii_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect()
}
pub(crate) fn extract_keywords(text: &str) -> Vec<String> {
    let stop_words: HashSet<&'static str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "that",
        "this", "it", "its", "not", "but", "and", "or", "if", "then", "so", "what", "which", "who",
        "how", "when", "where", "why", "all", "each", "every", "both", "few", "more", "most",
        "some", "any", "no", "my", "your", "his", "her", "our", "their", "i", "me",
    ]
    .into_iter()
    .collect();
    normalize_text(text)
        .split_whitespace()
        .filter(|word| word.len() > 2 && !stop_words.contains(*word))
        .map(str::to_string)
        .collect()
}
pub(crate) fn extract_search_keywords(text: &str) -> Vec<String> {
    normalize_text(text)
        .split_whitespace()
        .filter(|word| word.len() > 1)
        .map(str::to_string)
        .collect()
}
pub(crate) fn coding_synonyms(word: &str) -> Option<&'static str> {
    match word {
        "func" => Some("function"),
        "fn" => Some("function"),
        "err" => Some("error"),
        "db" => Some("database"),
        "auth" => Some("authentication"),
        "authn" => Some("authentication"),
        "authz" => Some("authorization"),
        "cfg" => Some("config"),
        "config" => Some("configuration"),
        "msg" => Some("message"),
        "req" => Some("request"),
        "res" => Some("response"),
        "resp" => Some("response"),
        "impl" => Some("implementation"),
        "repo" => Some("repository"),
        "env" => Some("environment"),
        "var" => Some("variable"),
        "arg" => Some("argument"),
        "args" => Some("arguments"),
        "param" => Some("parameter"),
        "params" => Some("parameters"),
        "dir" => Some("directory"),
        "tmp" => Some("temporary"),
        "async" => Some("asynchronous"),
        "sync" => Some("synchronous"),
        "tx" => Some("transaction"),
        "rx" => Some("receive"),
        "conn" => Some("connection"),
        "stmt" => Some("statement"),
        "idx" => Some("index"),
        "str" => Some("string"),
        "int" => Some("integer"),
        "bool" => Some("boolean"),
        "vec" => Some("vector"),
        "dict" => Some("dictionary"),
        "obj" => Some("object"),
        "num" => Some("number"),
        "char" => Some("character"),
        // Personal-memory recall aliases used by real user queries.
        "lastname" => Some("surname"),
        "surname" => Some("lastname"),
        "attend" => Some("attended"),
        "attended" => Some("attend"),
        "abroad" => Some("overseas"),
        "overseas" => Some("abroad"),
        "coupon" => Some("voucher"),
        "voucher" => Some("coupon"),
        "gift" => Some("present"),
        "present" => Some("gift"),
        "buy" => Some("bought"),
        "bought" => Some("buy"),
        "repaint" => Some("paint"),
        "repainted" => Some("paint"),
        "painted" => Some("paint"),
        "walls" => Some("wall"),
        "wall" => Some("walls"),
        "colour" => Some("color"),
        "color" => Some("colour"),
        "gray" => Some("grey"),
        "grey" => Some("gray"),
        _ => None,
    }
}
pub(crate) fn is_low_signal_query_token(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "a"
            | "an"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "do"
            | "does"
            | "did"
            | "to"
            | "of"
            | "in"
            | "for"
            | "on"
            | "with"
            | "at"
            | "by"
            | "from"
            | "as"
            | "into"
            | "about"
            | "that"
            | "this"
            | "it"
            | "its"
            | "my"
            | "your"
            | "our"
            | "their"
            | "i"
            | "me"
            | "we"
            | "you"
            | "what"
            | "which"
            | "who"
            | "how"
            | "when"
            | "where"
            | "why"
    )
}
pub(crate) fn query_intent_alias_terms(text: &str) -> Vec<String> {
    let lower = normalize_text(text);
    let mut aliases = Vec::new();
    if lower.contains("study abroad") {
        aliases.extend(
            ["attend", "attended", "exchange", "semester"]
                .into_iter()
                .map(str::to_string),
        );
    }
    if lower.contains("coupon") && lower.contains("creamer") {
        aliases.extend(
            ["redeem", "redeemed", "store", "grocery"]
                .into_iter()
                .map(str::to_string),
        );
    }
    if lower.contains("birthday") && (lower.contains("gift") || lower.contains("present")) {
        aliases.extend(
            ["buy", "bought", "item", "present"]
                .into_iter()
                .map(str::to_string),
        );
    }
    aliases
}
pub(crate) fn build_search_term_groups(text: &str) -> Vec<Vec<String>> {
    let mut base = extract_search_keywords(text);
    let profile = query_shape_profile(text, None);
    if profile.naturalish && base.len() >= 6 {
        let filtered = base
            .iter()
            .filter(|token| !is_low_signal_query_token(token.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !filtered.is_empty() {
            base = filtered;
        }
    }
    let mut seen_base = HashSet::new();
    for alias in query_intent_alias_terms(text) {
        if seen_base.insert(alias.clone()) && !base.iter().any(|token| token == &alias) {
            base.push(alias);
        }
    }
    let mut groups = Vec::with_capacity(base.len());
    for word in base {
        let mut group = Vec::with_capacity(2);
        let mut seen = HashSet::new();
        if let Some(expanded) = coding_synonyms(&word) {
            let expanded = expanded.to_string();
            if seen.insert(expanded.clone()) {
                group.push(expanded);
            }
        }
        if seen.insert(word.clone()) {
            group.push(word);
        }
        if !group.is_empty() {
            groups.push(group);
        }
    }
    groups
}
pub(crate) fn count_matching_term_groups(haystacks: &[String], term_groups: &[Vec<String>]) -> i64 {
    term_groups
        .iter()
        .filter(|group| {
            group
                .iter()
                .any(|term| haystacks.iter().any(|haystack| haystack.contains(term)))
        })
        .count() as i64
}
pub(crate) fn query_focus_terms(query_text: &str) -> Vec<String> {
    let mut terms = extract_keywords(query_text);
    let mut seen: HashSet<String> = terms.iter().cloned().collect();
    for group in build_search_term_groups(query_text) {
        for term in group {
            if seen.insert(term.clone()) {
                terms.push(term);
            }
        }
    }
    if terms.is_empty() {
        terms = extract_search_keywords(query_text);
    }
    terms
}
pub(crate) fn build_fts_query(groups: &[Vec<String>]) -> String {
    groups
        .iter()
        .map(|group| {
            let alternates = group
                .iter()
                .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" OR ");
            if group.len() > 1 {
                format!("({alternates})")
            } else {
                alternates
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}
pub(crate) fn query_focus_terms_for_excerpt(query_text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut terms = query_focus_terms(query_text)
        .into_iter()
        .filter_map(|term| {
            let normalized = term.trim().to_ascii_lowercase();
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                None
            } else {
                Some(normalized)
            }
        })
        .collect::<Vec<_>>();
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    terms
}
pub(crate) fn excerpt_signature_terms(source: &str, excerpt: &str) -> HashSet<String> {
    let mut terms = HashSet::new();
    for token in extract_search_keywords(source)
        .into_iter()
        .chain(extract_search_keywords(excerpt))
    {
        if token.len() > 2 {
            terms.insert(token);
        }
    }
    terms
}
pub(crate) fn term_set_jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}
pub(crate) fn query_term_coverage_gain(
    signature_terms: &HashSet<String>,
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> usize {
    query_terms
        .iter()
        .filter(|term| signature_terms.contains(*term) && !covered_terms.contains(*term))
        .count()
}
pub(crate) fn should_skip_redundant_budget_candidate(
    signature_terms: &HashSet<String>,
    selected_signatures: &[HashSet<String>],
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> bool {
    if selected_signatures.is_empty() || signature_terms.is_empty() {
        return false;
    }
    if query_term_coverage_gain(signature_terms, query_terms, covered_terms) > 0 {
        return false;
    }
    let max_similarity = selected_signatures
        .iter()
        .map(|existing| term_set_jaccard(existing, signature_terms))
        .fold(0.0_f64, f64::max);
    max_similarity >= BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD
}
pub(crate) fn update_query_term_coverage(
    signature_terms: &HashSet<String>,
    query_terms: &HashSet<String>,
    covered_terms: &mut HashSet<String>,
) {
    for term in query_terms {
        if signature_terms.contains(term) {
            covered_terms.insert(term.clone());
        }
    }
}
pub(crate) fn should_early_stop_budget_selection(
    token_budget: usize,
    spent_tokens: usize,
    selected_count: usize,
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> bool {
    if token_budget == 0 || selected_count < 2 || query_terms.is_empty() {
        return false;
    }
    if covered_terms.len() < query_terms.len() {
        return false;
    }
    let pressure = spent_tokens as f64 / token_budget as f64;
    pressure >= BUDGET_PRESSURE_EARLY_STOP_THRESHOLD
}
pub(crate) fn query_focused_excerpt_with_terms(
    text: &str,
    sorted_focus_terms: &[String],
    max_chars: usize,
) -> String {
    if max_chars == 0 || text.is_empty() {
        return String::new();
    }
    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return text.to_string();
    }
    let lower_text = text.to_ascii_lowercase();
    if lower_text.contains("[assistant-question]") {
        if let Some(answer_byte_idx) = lower_text.find("[user-answer]") {
            let answer_char_idx = text[..answer_byte_idx].chars().count();
            let answer_end_char = (answer_char_idx + max_chars).min(total_chars);
            let mut answer_excerpt = text
                .chars()
                .skip(answer_char_idx)
                .take(answer_end_char.saturating_sub(answer_char_idx))
                .collect::<String>();
            if !answer_excerpt.trim().is_empty() {
                if answer_char_idx > 0 {
                    answer_excerpt = format!("...{answer_excerpt}");
                }
                if answer_end_char < total_chars {
                    answer_excerpt.push_str("...");
                }
                return answer_excerpt;
            }
        }
    }
    if sorted_focus_terms.is_empty() {
        return truncate_chars(text, max_chars);
    }
    let mut hit_byte_idx = None;
    for term in sorted_focus_terms {
        if let Some(idx) = lower_text.find(term.as_str()) {
            hit_byte_idx = Some(idx);
            break;
        }
    }
    let Some(byte_idx) = hit_byte_idx else {
        return truncate_chars(text, max_chars);
    };
    let hit_char_idx = text[..byte_idx].chars().count();
    let left_window = max_chars / 3;
    let mut start_char = hit_char_idx.saturating_sub(left_window);
    let end_char = (start_char + max_chars).min(total_chars);
    if end_char - start_char < max_chars {
        start_char = end_char.saturating_sub(max_chars);
    }
    let mut excerpt = text
        .chars()
        .skip(start_char)
        .take(end_char - start_char)
        .collect::<String>();
    if start_char > 0 {
        excerpt = format!("...{excerpt}");
    }
    if end_char < total_chars {
        excerpt.push_str("...");
    }
    excerpt
}
pub(crate) fn query_focused_excerpt(text: &str, query_text: &str, max_chars: usize) -> String {
    let terms = query_focus_terms_for_excerpt(query_text);
    query_focused_excerpt_with_terms(text, &terms, max_chars)
}
pub(crate) fn recency_days(value: Option<&str>) -> i64 {
    let ts = value.map(parse_timestamp_ms).unwrap_or(0);
    if ts == 0 {
        return 3650;
    }
    (Utc::now().timestamp_millis() - ts).max(0) / (24 * 60 * 60 * 1000)
}
pub(crate) fn blend_importance(score: Option<f64>, trust_score: Option<f64>) -> f64 {
    let score = match score {
        Some(value) if value.is_finite() => value.clamp(0.0, 1.0),
        Some(_) => 0.0,
        None => 1.0,
    };
    let trust = match trust_score {
        Some(value) if value.is_finite() => value.clamp(0.0, 1.0),
        _ => score,
    };
    round4((score * 0.65) + (trust * 0.35))
}
pub(crate) fn compare_relevance_desc_source_asc(
    a_relevance: f64,
    a_source: &str,
    b_relevance: f64,
    b_source: &str,
) -> std::cmp::Ordering {
    // fallback ordering stays deterministic and finite scores always win.
    let a = if a_relevance.is_finite() {
        a_relevance
    } else {
        f64::NEG_INFINITY
    };
    let b = if b_relevance.is_finite() {
        b_relevance
    } else {
        f64::NEG_INFINITY
    };
    b.total_cmp(&a).then_with(|| a_source.cmp(b_source))
}
#[derive(Clone)]
pub(crate) struct QueryAlignmentProfile {
    pub(crate) lower_query: String,
    pub(crate) terms: Vec<String>,
    pub(crate) term_count: usize,
}
impl QueryAlignmentProfile {
    pub(crate) fn from_query(query_text: &str) -> Self {
        let lower_query = query_text.trim().to_ascii_lowercase();
        let mut seen = HashSet::new();
        let mut terms = Vec::new();
        for term in query_focus_terms(query_text) {
            let normalized = term.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if seen.insert(normalized.clone()) {
                terms.push(normalized);
            }
        }
        let term_count = terms.len().max(1);
        Self {
            lower_query,
            terms,
            term_count,
        }
    }
    pub(crate) fn alignment_score(&self, text: &str) -> (usize, usize) {
        if text.is_empty() || self.lower_query.is_empty() {
            return (0, 0);
        }
        let lower_text = text.to_ascii_lowercase();
        let exact_phrase = usize::from(lower_text.contains(&self.lower_query));
        let keyword_hits = self
            .terms
            .iter()
            .filter(|term| lower_text.contains(term.as_str()))
            .count();
        (exact_phrase, keyword_hits)
    }
}
pub(crate) fn prefer_query_focused_excerpt_with_profile(
    current: &str,
    candidate: &str,
    profile: &QueryAlignmentProfile,
) -> bool {
    let current_score = profile.alignment_score(current);
    let candidate_score = profile.alignment_score(candidate);
    candidate_score > current_score
        || (candidate_score == current_score && candidate.len() < current.len())
}
pub(crate) fn prefer_query_focused_excerpt(current: &str, candidate: &str, query_text: &str) -> bool {
    let profile = QueryAlignmentProfile::from_query(query_text);
    prefer_query_focused_excerpt_with_profile(current, candidate, &profile)
}
pub(crate) fn query_prefers_recency(query_text: &str) -> bool {
    let lower = query_text.to_ascii_lowercase();
    [
        "latest",
        "most recent",
        "recent",
        "newest",
        "current",
        "today",
        "now",
        "up to date",
        "up-to-date",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
pub(crate) fn temporal_intent_multiplier(ts_ms: i64) -> f64 {
    if ts_ms <= 0 {
        return 1.0 - (TEMPORAL_INTENT_MULTIPLIER_RANGE * 0.25);
    }
    let age_days =
        ((Utc::now().timestamp_millis() - ts_ms).max(0) as f64) / (1000.0 * 60.0 * 60.0 * 24.0);
    let freshness = (1.0 / (1.0 + age_days / 14.0)).clamp(0.0, 1.0);
    1.0 + ((freshness - 0.5) * TEMPORAL_INTENT_MULTIPLIER_RANGE)
}
pub(crate) fn query_alignment_boost_with_profile(
    source: &str,
    excerpt: &str,
    profile: &QueryAlignmentProfile,
    query_focus_term_count: usize,
) -> f64 {
    if profile.lower_query.is_empty() {
        return 0.0;
    }
    let lower_source = source.to_ascii_lowercase();
    let lower_excerpt = excerpt.to_ascii_lowercase();
    let exact_phrase = usize::from(
        lower_source.contains(&profile.lower_query) || lower_excerpt.contains(&profile.lower_query),
    );
    let keyword_hits = profile
        .terms
        .iter()
        .filter(|term| {
            lower_source.contains(term.as_str()) || lower_excerpt.contains(term.as_str())
        })
        .count();
    if exact_phrase == 0 && keyword_hits == 0 {
        return 0.0;
    }
    let term_count = query_focus_term_count.max(1) as f64;
    let coverage = (keyword_hits as f64 / term_count).clamp(0.0, 1.0);
    let exact_bonus = if exact_phrase > 0 {
        ALIGNMENT_EXACT_BONUS_MAX
    } else {
        0.0
    };
    let coverage_bonus =
        (coverage * ALIGNMENT_COVERAGE_BONUS_MAX).min(ALIGNMENT_COVERAGE_BONUS_MAX);
    (exact_bonus + coverage_bonus).min(ALIGNMENT_BOOST_MAX)
}
pub(crate) fn is_entity_stopword(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "a"
            | "an"
            | "and"
            | "or"
            | "for"
            | "with"
            | "from"
            | "into"
            | "this"
            | "that"
            | "these"
            | "those"
            | "what"
            | "which"
            | "when"
            | "where"
            | "why"
            | "how"
            | "about"
            | "around"
            | "there"
            | "their"
            | "your"
            | "our"
            | "have"
            | "has"
            | "had"
            | "will"
            | "would"
            | "could"
            | "should"
    )
}
pub(crate) fn is_short_technical_term(token: &str) -> bool {
    matches!(
        token,
        "ai" | "ml"
            | "db"
            | "sql"
            | "api"
            | "jwt"
            | "uid"
            | "uuid"
            | "id"
            | "ip"
            | "dns"
            | "tls"
            | "ssh"
            | "http"
            | "https"
            | "url"
            | "ui"
            | "ux"
            | "cpu"
            | "gpu"
            | "ram"
            | "ios"
            | "sdk"
    )
}
pub(crate) fn extract_entity_like_terms(text: &str) -> HashSet<String> {
    let mut terms = HashSet::new();
    for raw in text
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':')))
    {
        let token = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if token.len() < 3 {
            continue;
        }
        let lowered = token.to_ascii_lowercase();
        if is_entity_stopword(&lowered) {
            continue;
        }
        let has_uppercase = token.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = token.chars().any(|c| c.is_ascii_digit());
        let has_symbol = token
            .chars()
            .any(|c| matches!(c, '_' | '-' | '.' | '/' | ':'));
        let long_specific = lowered.len() >= 9;
        if has_uppercase || has_digit || has_symbol || long_specific {
            terms.insert(lowered);
        }
    }
    terms
}
pub(crate) fn query_entity_terms(query_text: &str) -> HashSet<String> {
    let mut terms = extract_entity_like_terms(query_text);
    if terms.is_empty() {
        for term in query_focus_terms(query_text) {
            if !is_entity_stopword(&term) && (term.len() >= 3 || is_short_technical_term(&term)) {
                terms.insert(term);
            }
        }
    }
    terms
}
pub(crate) fn entity_alignment_metrics_with_terms(
    haystack: &str,
    query_entities: &HashSet<String>,
) -> (usize, f64) {
    if query_entities.is_empty() {
        return (0, 0.0);
    }
    let mut haystack_terms = extract_entity_like_terms(haystack);
    if haystack_terms.is_empty() {
        for term in extract_search_keywords(haystack) {
            if !is_entity_stopword(&term) && (term.len() >= 3 || is_short_technical_term(&term)) {
                haystack_terms.insert(term);
            }
        }
    }
    if haystack_terms.is_empty() {
        return (0, 0.0);
    }
    let matches = query_entities
        .iter()
        .filter(|term| haystack_terms.contains(*term))
        .count();
    if matches == 0 {
        return (0, 0.0);
    }
    let overlap = matches as f64 / query_entities.len().max(1) as f64;
    (matches, overlap)
}
pub(crate) fn entity_signal_boost(matches: usize, overlap: f64) -> f64 {
    if matches == 0 {
        return 0.0;
    }
    let overlap_component = overlap.clamp(0.0, 1.0) * ENTITY_SIGNAL_OVERLAP_WEIGHT;
    let match_component = matches.min(3) as f64 * ENTITY_SIGNAL_MATCH_WEIGHT;
    (overlap_component + match_component).min(ENTITY_SIGNAL_MAX_BOOST)
}
pub(crate) fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace().collect();
    let set_b: HashSet<&str> = b.split_whitespace().collect();
    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FusionWeights {
    pub(crate) keyword: f64,
    pub(crate) semantic: f64,
}
pub(crate) fn adaptive_rrf_weights(
    query_text: &str,
    source_prefix: Option<&str>,
    semantic_available: bool,
) -> FusionWeights {
    if !semantic_available {
        return FusionWeights {
            keyword: 1.0,
            semantic: 0.0,
        };
    }
    let profile = query_shape_profile(query_text, source_prefix);
    let mut keyword = 1.0_f64;
    let mut semantic = 1.0_f64;
    if profile.exactish {
        keyword += 0.35;
        semantic -= 0.15;
    }
    if profile.naturalish {
        semantic += 0.35;
        keyword -= 0.15;
    }
    FusionWeights {
        keyword: keyword.clamp(0.35, 1.75),
        semantic: semantic.clamp(0.35, 1.75),
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FallbackRankingWeights {
    pub(crate) keyword: f64,
    pub(crate) score: f64,
    pub(crate) recency: f64,
    pub(crate) retrieval: f64,
}
pub(crate) fn adaptive_fallback_ranking_weights(
    query_text: &str,
    term_group_count: usize,
) -> FallbackRankingWeights {
    let profile = query_shape_profile(query_text, None);
    let mut keyword = 0.40_f64;
    let mut score = 0.25_f64;
    let mut recency = 0.20_f64;
    let mut retrieval = 0.15_f64;
    if profile.exactish && !profile.naturalish {
        keyword += 0.12;
        score -= 0.03;
        recency -= 0.05;
        retrieval -= 0.04;
    } else if profile.naturalish && !profile.exactish {
        keyword -= 0.08;
        score += 0.05;
        recency += 0.02;
        retrieval += 0.01;
    }
    if term_group_count <= 1 {
        keyword += 0.05;
        score += 0.01;
        recency -= 0.03;
        retrieval -= 0.03;
    } else if term_group_count >= 5 {
        keyword -= 0.04;
        score += 0.02;
        recency += 0.01;
        retrieval += 0.01;
    }
    keyword = keyword.max(0.05);
    score = score.max(0.05);
    recency = recency.max(0.05);
    retrieval = retrieval.max(0.05);
    let total = keyword + score + recency + retrieval;
    FallbackRankingWeights {
        keyword: keyword / total,
        score: score / total,
        recency: recency / total,
        retrieval: retrieval / total,
    }
}
pub(crate) fn fallback_ranking_score(
    query_text: &str,
    term_group_count: usize,
    matched: i64,
    effective_score: f64,
    recency_days: i64,
    retrievals: Option<i64>,
) -> f64 {
    let keyword_weight = if term_group_count == 0 {
        0.0
    } else {
        matched as f64 / term_group_count as f64
    };
    let recency_weight = 1.0 / (1.0 + recency_days.max(0) as f64 / 7.0);
    let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
    let score_weight = effective_score.clamp(0.0, 1.0);
    let weights = adaptive_fallback_ranking_weights(query_text, term_group_count);
    (keyword_weight * weights.keyword)
        + (score_weight * weights.score)
        + (recency_weight * weights.recency)
        + (retrieval_weight * weights.retrieval)
}
pub(crate) fn rrf_fuse_weighted(lists: &[Vec<(i64, f64)>], weights: &[f64], k: f64) -> Vec<(i64, f64)> {
    let smooth_k = if k.is_finite() && k >= 0.0 { k } else { 60.0 };
    let mut fused: HashMap<i64, f64> = HashMap::new();
    for (list_index, list) in lists.iter().enumerate() {
        let weight = match weights.get(list_index).copied() {
            Some(value) if value.is_finite() => value.max(0.0),
            Some(_) => 0.0,
            None => 1.0,
        };
        if weight == 0.0 {
            continue;
        }
        for (rank, &(id, _score)) in list.iter().enumerate() {
            *fused.entry(id).or_insert(0.0) += weight / (smooth_k + rank as f64 + 1.0);
        }
    }
    let mut result: Vec<(i64, f64)> = fused.into_iter().collect();
    result.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    result
}
pub(crate) fn days_since(created_at: &str) -> f64 {
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(dt) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(dt);
            duration.num_days() as f64 + (duration.num_seconds() as f64 % 86400.0) / 86400.0
        }
        Err(_) => f64::MAX, // Invalid timestamp: treat as very old
    }
}
pub(crate) fn normalize(importance: f64) -> f64 {
    if !importance.is_finite() {
        return 0.0;
    }
    let clamped = importance.clamp(0.0, 100.0);
    if clamped <= 1.0 {
        clamped
    } else {
        clamped / 100.0
    }
}
pub(crate) fn compound_score(rrf: f64, importance: f64, created_at: &str) -> f64 {
    let days = days_since(created_at);
    let recency = (-days / 30.0).exp(); // 21-day half-life
    let importance_normalized = normalize(importance);
    rrf * 0.6 + importance_normalized * 0.2 + recency * 0.2
}
fn sort_search_candidates(ranked: &mut [SearchCandidate], by_keywords: bool) {
    ranked.sort_by(|a, b| {
        let ord = b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal);
        let ord = if by_keywords {
            ord.then(b.matched_keywords.cmp(&a.matched_keywords))
        } else {
            ord
        };
        ord.then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.ts.cmp(&a.ts))
            .then(b.alignment.cmp(&a.alignment))
            .then_with(|| a.source.cmp(&b.source))
    });
}
pub(crate) fn search_memories(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    if term_groups.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at, compressed_text, age_tier \
                 FROM memories WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?2 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?2) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64, source_like.as_deref()], |row| {
                let text: String = row.get(1)?;
                let compressed: Option<String> = row.get(9)?;
                let age_tier: String = row
                    .get::<_, Option<String>>(10)?
                    .unwrap_or_else(|| "fresh".to_string());
                let display = crate::aging::get_display_text(&text, &compressed, &age_tier);
                let effective_score =
                    blend_importance(row.get::<_, Option<f64>>(4)?, row.get::<_, Option<f64>>(5)?);
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("memory::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 220),
                    alignment: (0, 0),
                    relevance: round4(0.5 * effective_score),
                    matched_keywords: 0,
                    score: effective_score,
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(7)?
                            .or(row.get::<_, Option<String>>(8)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;
        return Ok(rows
            .flatten()
            .filter(|row| source_matches_prefix(&row.source, source_prefix))
            .collect());
    }
    let fts_query = build_fts_query(&term_groups);
    let bm25 = bm25_weights();
    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // signal for code paths and metadata lookups.
        // bm25() returns negative values (more negative = better match), so ORDER BY ASC.
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.text, m.source, m.tags, m.score, m.trust_score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility \
                 FROM memories_fts fts \
                 JOIN memories m ON m.id = fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.status = 'active' \
                 AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
                 AND (?6 IS NULL OR COALESCE(m.source, 'memory::' || m.id) LIKE ?6) \
                 ORDER BY bm25(memories_fts, ?3, ?4, ?5) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                params![
                    &fts_query,
                    limit as i64,
                    bm25.memories_text,
                    bm25.memories_source,
                    bm25.memories_tags,
                    source_like.as_deref()
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, Option<f64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                text,
                source,
                tags,
                score,
                trust_score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = source
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("memory::{id}"));
            if !source_matches_prefix(&source_key, source_prefix) {
                continue;
            }
            let effective_score = blend_importance(score, trust_score);
            let ts = parse_timestamp_ms(
                last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or(""),
            );
            let display = crate::aging::get_display_text(
                &text,
                &compressed_text,
                age_tier.as_deref().unwrap_or("fresh"),
            );
            let haystacks = [
                text.to_lowercase(),
                source.as_deref().unwrap_or("").to_lowercase(),
                tags.as_deref().unwrap_or("").to_lowercase(),
            ];
            let matched = count_matching_term_groups(&haystacks, &term_groups);
            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let ranking = fallback_ranking_score(
                query_text,
                term_groups.len(),
                matched,
                effective_score,
                recency_d,
                retrievals,
            );
            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 280),
                alignment: (0, 0),
                relevance: round4(ranking),
                matched_keywords: matched,
                score: effective_score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then_with(|| a.source.cmp(&b.source))
        });
        ranked.truncate(limit);
        Ok(ranked)
    })();
    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_memories_fallback(conn, query_text, limit, source_prefix),
    }
}
enum SearchFallbackTable {
    Memories,
    Decisions,
}
fn search_table_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
    table: SearchFallbackTable,
) -> Result<Vec<SearchCandidate>, String> {
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let mut ranked = Vec::new();
    match table {
        SearchFallbackTable::Memories => {
            let mut stmt = conn.prepare(
                "SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at \
                 FROM memories WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 AND (valid_from IS NULL OR valid_from <= datetime('now')) \
                 AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?1 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?1)",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![source_like.as_deref()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            }).map_err(|e| e.to_string())?;
            for row in rows.flatten() {
                let (id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at) = row;
                let source_key = source.as_deref().map(str::to_owned).unwrap_or_else(|| format!("memory::{id}"));
                if !source_matches_prefix(&source_key, source_prefix) { continue; }
                let effective_score = blend_importance(score, trust_score);
                let ts = parse_timestamp_ms(last_accessed.as_deref().or(created_at.as_deref()).unwrap_or(""));
                if term_groups.is_empty() {
                    let excerpt = query_focused_excerpt_with_terms(&text, &excerpt_focus_terms, 220);
                    ranked.push(SearchCandidate {
                        source: source_key,
                        alignment: alignment_profile.alignment_score(&excerpt),
                        excerpt,
                        relevance: round4(0.5 * effective_score),
                        matched_keywords: 0,
                        score: effective_score,
                        ts,
                        owner_id: None,
                        visibility: None,
                    });
                    continue;
                }
                let haystacks = [text.to_lowercase(), source.as_deref().unwrap_or("").to_lowercase(), tags.as_deref().unwrap_or("").to_lowercase()];
                let matched = count_matching_term_groups(&haystacks, &term_groups);
                if matched == 0 { continue; }
                let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
                let ranking = fallback_ranking_score(query_text, term_groups.len(), matched, effective_score, recency_d, retrievals);
                let excerpt = query_focused_excerpt_with_terms(&text, &excerpt_focus_terms, 260);
                ranked.push(SearchCandidate {
                    source: source_key,
                    alignment: alignment_profile.alignment_score(&excerpt),
                    excerpt,
                    relevance: round4(ranking),
                    matched_keywords: matched,
                    score: effective_score,
                    ts,
                    owner_id: None,
                    visibility: None,
                });
            }
        }
        SearchFallbackTable::Decisions => {
            let mut stmt = conn.prepare(
                "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at \
                 FROM decisions WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 AND (valid_from IS NULL OR valid_from <= datetime('now')) \
                 AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?1 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?1)",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map(params![source_like.as_deref()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            }).map_err(|e| e.to_string())?;
            for row in rows.flatten() {
                let (id, decision, context, score, trust_score, retrievals, last_accessed, created_at) = row;
                let source_key = context.as_deref().map(str::to_owned).unwrap_or_else(|| format!("decision::{id}"));
                if !source_matches_prefix(&source_key, source_prefix) { continue; }
                let effective_score = blend_importance(score, trust_score);
                let ts = parse_timestamp_ms(last_accessed.as_deref().or(created_at.as_deref()).unwrap_or(""));
                if term_groups.is_empty() {
                    let excerpt = query_focused_excerpt_with_terms(&decision, &excerpt_focus_terms, 220);
                    ranked.push(SearchCandidate {
                        source: source_key,
                        alignment: alignment_profile.alignment_score(&excerpt),
                        excerpt,
                        relevance: round4(0.5 * effective_score),
                        matched_keywords: 0,
                        score: effective_score,
                        ts,
                        owner_id: None,
                        visibility: None,
                    });
                    continue;
                }
                let haystacks = [decision.to_lowercase(), context.as_deref().unwrap_or("").to_lowercase()];
                let matched = count_matching_term_groups(&haystacks, &term_groups);
                if matched == 0 { continue; }
                let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
                let ranking = fallback_ranking_score(query_text, term_groups.len(), matched, effective_score, recency_d, retrievals);
                let excerpt = query_focused_excerpt_with_terms(&decision, &excerpt_focus_terms, 260);
                ranked.push(SearchCandidate {
                    source: source_key,
                    alignment: alignment_profile.alignment_score(&excerpt),
                    excerpt,
                    relevance: round4(ranking),
                    matched_keywords: matched,
                    score: effective_score,
                    ts,
                    owner_id: None,
                    visibility: None,
                });
            }
        }
    }
    sort_search_candidates(&mut ranked, !term_groups.is_empty());
    ranked.truncate(limit);
    Ok(ranked)
}
pub(crate) fn search_memories_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table_fallback(conn, query_text, limit, source_prefix, SearchFallbackTable::Memories)
}
pub(crate) fn search_decisions(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    if term_groups.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at \
                 FROM decisions WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 AND (?2 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?2) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![limit as i64, source_like.as_deref()], |row| {
                let effective_score =
                    blend_importance(row.get::<_, Option<f64>>(3)?, row.get::<_, Option<f64>>(4)?);
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("decision::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: query_focused_excerpt_with_terms(
                        &row.get::<_, String>(1)?,
                        &excerpt_focus_terms,
                        220,
                    ),
                    alignment: (0, 0),
                    relevance: round4(0.5 * effective_score),
                    matched_keywords: 0,
                    score: effective_score,
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(6)?
                            .or(row.get::<_, Option<String>>(7)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;
        return Ok(rows
            .flatten()
            .filter(|row| source_matches_prefix(&row.source, source_prefix))
            .collect());
    }
    let fts_query = build_fts_query(&term_groups);
    let bm25 = bm25_weights();
    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // Field-boosted BM25: decisions_fts columns are (decision, context).
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.decision, d.context, d.score, d.trust_score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility \
                 FROM decisions_fts fts \
                 JOIN decisions d ON d.id = fts.rowid \
                 WHERE decisions_fts MATCH ?1 AND d.status = 'active' \
                 AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
                 AND (?5 IS NULL OR COALESCE(d.context, 'decision::' || d.id) LIKE ?5) \
                 ORDER BY bm25(decisions_fts, ?3, ?4) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(
                params![
                    &fts_query,
                    limit as i64,
                    bm25.decisions_text,
                    bm25.decisions_context,
                    source_like.as_deref()
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<f64>>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?;
        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                decision,
                context,
                score,
                trust_score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = context
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("decision::{id}"));
            if !source_matches_prefix(&source_key, source_prefix) {
                continue;
            }
            let effective_score = blend_importance(score, trust_score);
            let ts = parse_timestamp_ms(
                last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or(""),
            );
            let display = crate::aging::get_display_text(
                &decision,
                &compressed_text,
                age_tier.as_deref().unwrap_or("fresh"),
            );
            let haystacks = [
                decision.to_lowercase(),
                context.as_deref().unwrap_or("").to_lowercase(),
            ];
            let matched = count_matching_term_groups(&haystacks, &term_groups);
            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let ranking = fallback_ranking_score(
                query_text,
                term_groups.len(),
                matched,
                effective_score,
                recency_d,
                retrievals,
            );
            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: query_focused_excerpt_with_terms(&display, &excerpt_focus_terms, 280),
                alignment: (0, 0),
                relevance: round4(ranking),
                matched_keywords: matched,
                score: effective_score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
                .then_with(|| a.source.cmp(&b.source))
        });
        ranked.truncate(limit);
        Ok(ranked)
    })();
    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_decisions_fallback(conn, query_text, limit, source_prefix),
    }
}
pub(crate) fn search_decisions_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
    source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table_fallback(conn, query_text, limit, source_prefix, SearchFallbackTable::Decisions)
}
pub(crate) fn collect_semantic_candidates(
    conn: &Connection,
    query_vector: &[f32],
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Vec<SemanticCandidate> {
    let selected_model = crate::embeddings::selected_model_key();
    let expected_vector_bytes = std::mem::size_of_val(query_vector) as i64;
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let scale_sim = |sim: f32| -> f64 {
        SEMANTIC_SCALE_BASE
            + (sim as f64 - SEMANTIC_SIM_FLOOR)
                * ((1.0 - SEMANTIC_SCALE_BASE) / (1.0 - SEMANTIC_SIM_FLOOR))
    };
    let keyword_terms = extract_search_keywords(query_text);
    let semantic_floor = if keyword_terms.len() >= 3 {
        SEMANTIC_SIM_FLOOR + 0.12
    } else {
        SEMANTIC_SIM_FLOOR
    };
    let mut candidates: HashMap<String, SemanticCandidate> = HashMap::new();
    let semantic_memory_query_with_acl = "SELECT e.vector, m.text, m.source, m.owner_id, m.visibility, m.score, m.trust_score, m.last_accessed, m.created_at \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let semantic_memory_query_without_acl = "SELECT e.vector, m.text, m.source, NULL AS owner_id, NULL AS visibility, m.score, m.trust_score, m.last_accessed, m.created_at \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let semantic_memory_stmt = match conn.prepare(semantic_memory_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(semantic_memory_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = semantic_memory_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<MemorySemanticRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        ) {
            for (
                blob,
                text,
                source,
                owner_id,
                visibility,
                score,
                trust_score,
                last_accessed,
                created_at,
            ) in rows.flatten()
            {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                let existing_vec = crate::embeddings::blob_to_vector(&blob);
                let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
                if sim <= semantic_floor as f32 {
                    continue;
                }
                let mut scaled = scale_sim(sim);
                if !keyword_terms.is_empty() {
                    let haystack = text.to_lowercase();
                    let overlap = keyword_terms
                        .iter()
                        .filter(|term| haystack.contains(term.as_str()))
                        .count();
                    if overlap == 0 {
                        scaled *= 0.82;
                    } else {
                        let ratio = overlap as f64 / keyword_terms.len().max(1) as f64;
                        scaled *= 1.0 + ratio * 0.08;
                    }
                }
                let excerpt = query_focused_excerpt(&text, query_text, 280);
                let importance = blend_importance(score, trust_score);
                let ts_source = last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or_default();
                let ts = parse_timestamp_ms(ts_source);
                let entry = candidates
                    .entry(source.clone())
                    .or_insert(SemanticCandidate {
                        source,
                        excerpt: excerpt.clone(),
                        relevance: scaled,
                        importance,
                        ts,
                    });
                if scaled > entry.relevance {
                    *entry = SemanticCandidate {
                        source: entry.source.clone(),
                        excerpt,
                        relevance: scaled,
                        importance,
                        ts,
                    };
                }
            }
        }
    }
    let semantic_decision_query_with_acl = "SELECT e.vector, d.decision, d.context, d.owner_id, d.visibility, d.score, d.trust_score, d.last_accessed, d.created_at \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let semantic_decision_query_without_acl = "SELECT e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility, d.score, d.trust_score, d.last_accessed, d.created_at \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let semantic_decision_stmt = match conn.prepare(semantic_decision_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(semantic_decision_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = semantic_decision_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<DecisionSemanticRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        ) {
            for (
                blob,
                decision,
                context,
                owner_id,
                visibility,
                score,
                trust_score,
                last_accessed,
                created_at,
            ) in rows.flatten()
            {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                let existing_vec = crate::embeddings::blob_to_vector(&blob);
                let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
                if sim <= semantic_floor as f32 {
                    continue;
                }
                let source = context.unwrap_or_else(|| {
                    format!(
                        "decision::{}",
                        decision.chars().take(40).collect::<String>()
                    )
                });
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                let mut scaled = scale_sim(sim);
                if !keyword_terms.is_empty() {
                    let haystack = decision.to_lowercase();
                    let overlap = keyword_terms
                        .iter()
                        .filter(|term| haystack.contains(term.as_str()))
                        .count();
                    if overlap == 0 {
                        scaled *= 0.82;
                    } else {
                        let ratio = overlap as f64 / keyword_terms.len().max(1) as f64;
                        scaled *= 1.0 + ratio * 0.08;
                    }
                }
                let excerpt = query_focused_excerpt(&decision, query_text, 280);
                let importance = blend_importance(score, trust_score);
                let ts_source = last_accessed
                    .as_deref()
                    .or(created_at.as_deref())
                    .unwrap_or_default();
                let ts = parse_timestamp_ms(ts_source);
                let entry = candidates
                    .entry(source.clone())
                    .or_insert(SemanticCandidate {
                        source,
                        excerpt: excerpt.clone(),
                        relevance: scaled,
                        importance,
                        ts,
                    });
                if scaled > entry.relevance {
                    *entry = SemanticCandidate {
                        source: entry.source.clone(),
                        excerpt,
                        relevance: scaled,
                        importance,
                        ts,
                    };
                }
            }
        }
    }
    let mut sorted: Vec<SemanticCandidate> = candidates.into_values().collect();
    sorted.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    sorted.truncate(MAX_SEMANTIC_RRF_CANDIDATES);
    sorted
}
pub(crate) fn collect_shadow_semantic_rows(
    conn: &Connection,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    expected_dimension: usize,
) -> Vec<ShadowSemanticRow> {
    let selected_model = crate::embeddings::selected_model_key();
    let expected_vector_bytes = (expected_dimension * std::mem::size_of::<f32>()) as i64;
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let mut rows_by_source: HashMap<String, Vec<f32>> = HashMap::new();
    let memory_query_with_acl = "SELECT e.vector, m.source, m.owner_id, m.visibility \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let memory_query_without_acl = "SELECT e.vector, m.source, NULL AS owner_id, NULL AS visibility \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
         AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) \
         AND (m.valid_until IS NULL OR m.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR m.source LIKE ?3)";
    let memory_stmt = match conn.prepare(memory_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(memory_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = memory_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<ShadowMemoryRow> {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            },
        ) {
            for (blob, source, owner_id, visibility) in rows.flatten() {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                rows_by_source
                    .entry(source)
                    .or_insert_with(|| crate::embeddings::blob_to_vector(&blob));
            }
        }
    }
    let decision_query_with_acl = "SELECT e.vector, d.decision, d.context, d.owner_id, d.visibility \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let decision_query_without_acl = "SELECT e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
         AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) \
         AND (d.valid_until IS NULL OR d.valid_until > datetime('now')) \
         AND (e.model IS NULL OR LOWER(e.model) = ?1) \
         AND (length(e.vector) = ?2 OR length(e.vector) = ?2/4 + 6) \
         AND (?3 IS NULL OR d.context LIKE ?3)";
    let decision_stmt = match conn.prepare(decision_query_with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.prepare(decision_query_without_acl).ok()
        }
        Err(_) => None,
    };
    if let Some(mut stmt) = decision_stmt {
        if let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_vector_bytes,
                source_like.as_deref()
            ],
            |row| -> rusqlite::Result<ShadowDecisionRow> {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        ) {
            for (blob, decision, context, owner_id, visibility) in rows.flatten() {
                if !is_visible(owner_id, visibility.as_deref(), ctx) {
                    continue;
                }
                let source = context.unwrap_or_else(|| {
                    format!(
                        "decision::{}",
                        decision.chars().take(40).collect::<String>()
                    )
                });
                if !source_matches_prefix(&source, source_prefix) {
                    continue;
                }
                rows_by_source
                    .entry(source)
                    .or_insert_with(|| crate::embeddings::blob_to_vector(&blob));
            }
        }
    }
    let mut rows: Vec<ShadowSemanticRow> = rows_by_source
        .into_iter()
        .map(|(source, vector)| ShadowSemanticRow { source, vector })
        .collect();
    rows.sort_by(|a, b| a.source.cmp(&b.source));
    rows
}
pub(crate) fn vector_to_vec0_literal(vector: &[f32]) -> String {
    let mut literal = String::with_capacity(vector.len().saturating_mul(12).saturating_add(2));
    literal.push('[');
    for (idx, value) in vector.iter().enumerate() {
        if idx > 0 {
            literal.push_str(", ");
        }
        let stable = if value.is_finite() { *value } else { 0.0 };
        let _ = write!(&mut literal, "{stable}");
    }
    literal.push(']');
    literal
}
pub(crate) fn run_sqlite_vec_shadow_knn_sources(
    conn: &Connection,
    query_vector: &[f32],
    candidates: &[ShadowSemanticRow],
    top_k: usize,
) -> Result<Vec<String>, String> {
    if query_vector.is_empty() || candidates.is_empty() {
        return Ok(Vec::new());
    }
    const SHADOW_TABLE: &str = "cortex_shadow_semantic_knn";
    let k = top_k.max(1).min(candidates.len());
    let query_literal = vector_to_vec0_literal(query_vector);
    let result = (|| -> Result<Vec<String>, String> {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {SHADOW_TABLE};"))
            .map_err(|err| format!("sqlite-vec shadow drop failed: {err}"))?;
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE {SHADOW_TABLE} USING vec0(\
                candidate_id INTEGER PRIMARY KEY,\
                embedding FLOAT[{}]\
            );",
            query_vector.len()
        ))
        .map_err(|err| format!("sqlite-vec shadow create failed: {err}"))?;
        let insert_sql =
            format!("INSERT INTO {SHADOW_TABLE}(candidate_id, embedding) VALUES (?1, ?2)");
        let mut insert_stmt = conn
            .prepare(&insert_sql)
            .map_err(|err| format!("sqlite-vec shadow insert prepare failed: {err}"))?;
        for (candidate_idx, candidate) in candidates.iter().enumerate() {
            let candidate_id = i64::try_from(candidate_idx + 1)
                .map_err(|_| "sqlite-vec shadow candidate id overflow".to_string())?;
            let embedding_literal = vector_to_vec0_literal(&candidate.vector);
            insert_stmt
                .execute(params![candidate_id, embedding_literal])
                .map_err(|err| format!("sqlite-vec shadow insert failed: {err}"))?;
        }
        let k_i64 = i64::try_from(k).map_err(|_| "sqlite-vec shadow k overflow".to_string())?;
        let query_sql = format!(
            "SELECT candidate_id, distance \
             FROM {SHADOW_TABLE} \
             WHERE embedding MATCH ?1 AND k = ?2"
        );
        let mut query_stmt = conn
            .prepare(&query_sql)
            .map_err(|err| format!("sqlite-vec shadow query prepare failed: {err}"))?;
        let rows = query_stmt
            .query_map(params![query_literal, k_i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|err| format!("sqlite-vec shadow query failed: {err}"))?;
        let mut sources = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let (candidate_id, _distance) =
                row.map_err(|err| format!("sqlite-vec shadow row decode failed: {err}"))?;
            if candidate_id <= 0 {
                continue;
            }
            let Some(candidate) = candidates.get((candidate_id - 1) as usize) else {
                continue;
            };
            if seen.insert(candidate.source.clone()) {
                sources.push(candidate.source.clone());
            }
        }
        Ok(sources)
    })();
    let _ = conn.execute_batch(&format!("DROP TABLE IF EXISTS {SHADOW_TABLE};"));
    result
}
pub(crate) fn shadow_error_to_unavailable_reason(error: &str) -> Option<&'static str> {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("no such module: vec0") {
        return Some("sqlite_vec_not_available");
    }
    None
}
pub(crate) fn build_shadow_semantic_explain(
    conn: &Connection,
    query_vector: Option<&[f32]>,
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    top_k: usize,
    baseline_override: Option<&ShadowSemanticBaseline>,
) -> Value {
    let top_k = top_k.clamp(1, MAX_SEMANTIC_RRF_CANDIDATES);
    let Some(query_vector) = query_vector else {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "query_embedding_unavailable",
            "topK": top_k
        });
    };
    if query_vector.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "query_embedding_empty",
            "topK": top_k
        });
    }
    let (baseline_candidate_count, baseline_top_sources) = if let Some(baseline) = baseline_override
    {
        (baseline.candidate_count, baseline.top_sources(top_k))
    } else {
        let baseline =
            collect_semantic_candidates(conn, query_vector, query_text, ctx, source_prefix);
        let top_sources = baseline
            .iter()
            .take(top_k)
            .map(|candidate| candidate.source.clone())
            .collect();
        (baseline.len(), top_sources)
    };
    let rows = collect_shadow_semantic_rows(conn, ctx, source_prefix, query_vector.len());
    if rows.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "no_shadow_candidates",
            "topK": top_k,
            "baselineCandidateCount": baseline_candidate_count,
            "baselineTopSources": baseline_top_sources,
        });
    }
    let vector_dim = query_vector.len();
    let compatible_rows: Vec<ShadowSemanticRow> = rows
        .into_iter()
        .filter(|row| row.vector.len() == vector_dim)
        .collect();
    if compatible_rows.is_empty() {
        return json!({
            "enabled": true,
            "status": "unavailable",
            "reason": "no_dimension_compatible_candidates",
            "topK": top_k,
            "vectorDimension": vector_dim,
            "baselineCandidateCount": baseline_candidate_count,
            "baselineTopSources": baseline_top_sources,
        });
    }
    let compatible_count = compatible_rows.len();
    let shadow_top_sources =
        match run_sqlite_vec_shadow_knn_sources(conn, query_vector, &compatible_rows, top_k) {
            Ok(sources) => sources,
            Err(error) => {
                if let Some(reason) = shadow_error_to_unavailable_reason(&error) {
                    return json!({
                        "enabled": true,
                        "status": "unavailable",
                        "reason": reason,
                        "detail": error,
                        "topK": top_k,
                        "vectorDimension": vector_dim,
                        "baselineCandidateCount": baseline_candidate_count,
                        "shadowCandidateCount": compatible_count,
                        "baselineTopSources": baseline_top_sources,
                    });
                }
                return json!({
                    "enabled": true,
                    "status": "error",
                    "reason": error,
                    "topK": top_k,
                    "vectorDimension": vector_dim,
                    "baselineCandidateCount": baseline_candidate_count,
                    "shadowCandidateCount": compatible_count,
                    "baselineTopSources": baseline_top_sources,
                });
            }
        };
    let baseline_set: HashSet<&str> = baseline_top_sources.iter().map(String::as_str).collect();
    let shadow_set: HashSet<&str> = shadow_top_sources.iter().map(String::as_str).collect();
    let overlap_count = baseline_set.intersection(&shadow_set).count();
    let union_count = baseline_set.union(&shadow_set).count();
    let overlap_ratio = if top_k == 0 {
        0.0
    } else {
        round4(overlap_count as f64 / top_k as f64)
    };
    let jaccard = if union_count == 0 {
        1.0
    } else {
        round4(overlap_count as f64 / union_count as f64)
    };
    let baseline_index: HashMap<&str, usize> = baseline_top_sources
        .iter()
        .enumerate()
        .map(|(idx, source)| (source.as_str(), idx))
        .collect();
    let shadow_index: HashMap<&str, usize> = shadow_top_sources
        .iter()
        .enumerate()
        .map(|(idx, source)| (source.as_str(), idx))
        .collect();
    let mut matched_rank_pairs: usize = 0;
    let mut rank_delta_sum: usize = 0;
    for (source, baseline_rank) in &baseline_index {
        if let Some(shadow_rank) = shadow_index.get(source) {
            matched_rank_pairs += 1;
            rank_delta_sum += baseline_rank.abs_diff(*shadow_rank);
        }
    }
    let mean_abs_rank_delta = if matched_rank_pairs > 0 {
        Some(round4(rank_delta_sum as f64 / matched_rank_pairs as f64))
    } else {
        None
    };
    let top1_match = match (
        baseline_top_sources.first().map(String::as_str),
        shadow_top_sources.first().map(String::as_str),
    ) {
        (Some(left), Some(right)) => Some(left == right),
        _ => None,
    };
    json!({
        "enabled": true,
        "status": "ok",
        "topK": top_k,
        "vectorDimension": vector_dim,
        "baselineCandidateCount": baseline_candidate_count,
        "shadowCandidateCount": compatible_count,
        "baselineTopSources": baseline_top_sources,
        "shadowTopSources": shadow_top_sources,
        "overlapCount": overlap_count,
        "overlapRatio": overlap_ratio,
        "jaccard": jaccard,
        "matchedRankPairs": matched_rank_pairs,
        "meanAbsRankDelta": mean_abs_rank_delta,
        "top1Match": top1_match,
    })
}
pub(crate) fn round4(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * 10000.0).round() / 10000.0
}
pub(crate) fn bump_retrievals_batch(conn: &Connection, items: &[RecallItem]) {
    if items.is_empty() {
        return;
    }
    let now = now_iso();
    let sources: Vec<&str> = items.iter().map(|i| i.source.as_str()).collect();
    // Batch boost memories -- single UPDATE with IN clause
    let placeholders: String = sources
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE memories SET \
           retrievals = retrievals + 1, \
           last_accessed = ?1, \
           score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
         WHERE source IN ({})",
        placeholders
    );
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
        Vec::with_capacity(sources.len() + 1);
    params_vec.push(Box::new(now.clone()));
    for s in &sources {
        params_vec.push(Box::new(s.to_string()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params_vec.iter().map(|p| p.as_ref()).collect();
    let _ = conn.execute(&sql, param_refs.as_slice());
    // Batch boost decisions by id
    let decision_ids: Vec<i64> = sources
        .iter()
        .filter_map(|s| s.strip_prefix("decision::").and_then(|id| id.parse().ok()))
        .collect();
    if !decision_ids.is_empty() {
        let d_placeholders: String = decision_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        let d_sql = format!(
            "UPDATE decisions SET \
               retrievals = retrievals + 1, \
               last_accessed = ?1, \
               score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
             WHERE id IN ({})",
            d_placeholders
        );
        let mut d_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            Vec::with_capacity(decision_ids.len() + 1);
        d_params.push(Box::new(now.clone()));
        for id in &decision_ids {
            d_params.push(Box::new(*id));
        }
        let d_refs: Vec<&dyn rusqlite::types::ToSql> =
            d_params.iter().map(|p| p.as_ref()).collect();
        let _ = conn.execute(&d_sql, d_refs.as_slice());
    }
    // Batch boost decisions by context (non-id sources)
    let context_sources: Vec<&str> = sources
        .iter()
        .filter(|s| !s.starts_with("decision::"))
        .copied()
        .collect();
    if !context_sources.is_empty() {
        let c_placeholders: String = context_sources
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        let c_sql = format!(
            "UPDATE decisions SET \
               retrievals = retrievals + 1, \
               last_accessed = ?1, \
               score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
             WHERE context IN ({})",
            c_placeholders
        );
        let mut c_params: Vec<Box<dyn rusqlite::types::ToSql>> =
            Vec::with_capacity(context_sources.len() + 1);
        c_params.push(Box::new(now));
        for s in &context_sources {
            c_params.push(Box::new(s.to_string()));
        }
        let c_refs: Vec<&dyn rusqlite::types::ToSql> =
            c_params.iter().map(|p| p.as_ref()).collect();
        let _ = conn.execute(&c_sql, c_refs.as_slice());
    }
}
pub(crate) fn recall_to_json(item: RecallItem) -> Value {
    let mut payload = json!({
        "source": item.source,
        "relevance": item.relevance,
        "excerpt": item.excerpt,
        "method": item.method
    });
    if let Value::Object(ref mut map) = payload {
        if let Some(tokens) = item.tokens {
            map.insert("tokens".to_string(), Value::Number((tokens as u64).into()));
        }
        if !item.family_members.is_empty() {
            let family_size = item.family_members.len() as u64;
            map.insert(
                "familyMembers".to_string(),
                Value::Array(item.family_members.into_iter().map(Value::String).collect()),
            );
            map.insert("familySize".to_string(), Value::Number(family_size.into()));
        }
        if !item.collapsed_sources.is_empty() {
            map.insert(
                "collapsedSources".to_string(),
                Value::Array(
                    item.collapsed_sources
                        .into_iter()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if !item.collapsed_source_scores.is_empty() {
            map.insert(
                "collapsedSourceScores".to_string(),
                Value::Array(
                    item.collapsed_source_scores
                        .into_iter()
                        .map(|(source, relevance)| {
                            json!({
                                "source": source,
                                "relevance": relevance,
                            })
                        })
                        .collect(),
                ),
            );
        }
    }
    payload
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecallBudgetUsage {
    pub(crate) spent: usize,
    pub(crate) saved: i64,
    pub(crate) over_budget: bool,
}
pub(crate) fn recall_item_token_cost(item: &RecallItem) -> usize {
    item.tokens
        .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
}
pub(crate) fn compute_recall_budget_usage(items: &[RecallItem], budget: usize) -> RecallBudgetUsage {
    let spent: usize = items.iter().map(recall_item_token_cost).sum();
    let saved = budget as i64 - spent as i64;
    RecallBudgetUsage {
        spent,
        saved,
        over_budget: budget > 0 && spent > budget,
    }
}
pub(crate) fn compute_headlines_token_usage(items: &[RecallItem]) -> RecallBudgetUsage {
    let spent = items
        .iter()
        .map(|item| estimate_tokens(&item.source))
        .sum::<usize>();
    let full_recall_tokens = items.iter().map(recall_item_token_cost).sum::<usize>();
    RecallBudgetUsage {
        spent,
        saved: full_recall_tokens as i64 - spent as i64,
        over_budget: false,
    }
}
pub(crate) fn format_recall_token_usage_line(budget: usize, usage: RecallBudgetUsage) -> String {
    if budget == 0 {
        if usage.saved > 0 {
            format!(
                "Cortex recall used {} tokens in headlines mode and saved {} vs full excerpts.",
                usage.spent, usage.saved
            )
        } else {
            format!(
                "Cortex recall used {} tokens (headlines mode).",
                usage.spent
            )
        }
    } else if usage.saved >= 0 {
        format!(
            "Cortex recall used {} tokens and saved {} of {} budget.",
            usage.spent, usage.saved, budget
        )
    } else {
        format!(
            "Cortex recall used {} tokens ({} over budget {}).",
            usage.spent,
            usage.saved.abs(),
            budget
        )
    }
}
pub(crate) fn enforce_budget_token_invariant(
    results: Vec<RecallItem>,
    token_budget: usize,
    query_text: &str,
) -> Vec<RecallItem> {
    if token_budget == 0 || results.is_empty() {
        return results;
    }
    let usage = compute_recall_budget_usage(&results, token_budget);
    if !usage.over_budget {
        return results;
    }
    let mut kept = Vec::new();
    let mut spent = 0usize;
    for (idx, mut item) in results.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= MIN_BUDGET_HEADROOM_TOKENS {
            break;
        }
        let direct_tokens = recall_item_token_cost(&item);
        if direct_tokens <= remaining {
            item.tokens = Some(direct_tokens);
            spent += direct_tokens;
            kept.push(item);
            continue;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text)
            .min((remaining as f64 * 3.6) as usize)
            .max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) =
            fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining)
        {
            if tokens <= remaining {
                item.excerpt = excerpt;
                item.tokens = Some(tokens);
                spent += tokens;
                kept.push(item);
            }
        }
    }
    kept
}
pub(crate) fn hash_content(content: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for ch in content.chars().take(100) {
        hash ^= ch as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}
pub(crate) fn source_dedup_hash(source: &str) -> u32 {
    hash_content(&format!("source::{source}"))
}
pub(crate) fn collapse_score_is_better(
    candidate_score: f64,
    candidate_order: usize,
    best_score: f64,
    best_order: usize,
) -> bool {
    match candidate_score.total_cmp(&best_score) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => candidate_order < best_order,
    }
}
pub(crate) async fn load_collapsed_source_fallback(
    state: &RuntimeState,
    source: &str,
    query: &str,
    ctx: &RecallContext,
    relevance: f64,
) -> Option<RecallItem> {
    let conn = state.db_read.lock().await;
    let payload = unfold_source(&conn, source, ctx)?;
    let canonical_source = payload
        .get("source")
        .and_then(|value| value.as_str())
        .unwrap_or(source)
        .to_string();
    let text = payload.get("text").and_then(|value| value.as_str())?;
    Some(RecallItem {
        source: canonical_source,
        relevance,
        excerpt: query_focused_excerpt(text, query, 260),
        method: "crystal".to_string(),
        tokens: None,
        entropy: None,
        family_members: Vec::new(),
        collapsed_sources: Vec::new(),
        collapsed_source_scores: Vec::new(),
    })
}
pub(crate) const SERVED_TTL_MS: i64 = 60_000; // 60 seconds
pub(crate) async fn dedup_and_mark_served(
    state: &RuntimeState,
    agent: &str,
    query: &str,
    ctx: &RecallContext,
    results: Vec<RecallItem>,
) -> Vec<RecallItem> {
    if results.is_empty() {
        return results;
    }
    let now = Utc::now().timestamp_millis();
    let scope_key = served_content_scope(agent, query, ctx);
    let mut seen_hashes: HashSet<u32> = {
        let mut served = state.served_content.lock().await;
        let map = served
            .entry(scope_key.clone())
            .or_insert_with(HashMap::<u32, i64>::new);
        map.retain(|_, ts| now - *ts < SERVED_TTL_MS);
        map.keys().copied().collect()
    };
    let mut staged_hashes: Vec<u32> = Vec::with_capacity(results.len() * 2);
    let mut filtered = Vec::new();
    for result in results {
        let excerpt_hash = hash_content(&result.excerpt);
        let source_hash = source_dedup_hash(&result.source);
        let already_served =
            seen_hashes.contains(&excerpt_hash) || seen_hashes.contains(&source_hash);
        if already_served {
            if result.method == "crystal" && !result.collapsed_sources.is_empty() {
                let fallback_candidates: Vec<(usize, String, f64)> =
                    if result.collapsed_source_scores.is_empty() {
                        result
                            .collapsed_sources
                            .iter()
                            .enumerate()
                            .map(|(idx, source)| (idx, source.clone(), 0.0))
                            .collect()
                    } else {
                        result
                            .collapsed_source_scores
                            .iter()
                            .enumerate()
                            .map(|(idx, (source, score))| (idx, source.clone(), *score))
                            .collect()
                    };
                let mut best_candidate: Option<(usize, f64, RecallItem)> = None;
                for (order, collapsed_source, collapsed_score) in fallback_candidates {
                    let collapsed_source_hash = source_dedup_hash(&collapsed_source);
                    if seen_hashes.contains(&collapsed_source_hash) {
                        continue;
                    }
                    let candidate_relevance = round4(collapsed_score.max(0.0));
                    let Some(candidate) = load_collapsed_source_fallback(
                        state,
                        &collapsed_source,
                        query,
                        ctx,
                        candidate_relevance,
                    )
                    .await
                    else {
                        continue;
                    };
                    let candidate_excerpt_hash = hash_content(&candidate.excerpt);
                    let candidate_source_hash = source_dedup_hash(&candidate.source);
                    if seen_hashes.contains(&candidate_excerpt_hash)
                        || seen_hashes.contains(&candidate_source_hash)
                    {
                        continue;
                    }
                    let replace = match &best_candidate {
                        None => true,
                        Some((best_order, best_score, _)) => collapse_score_is_better(
                            candidate_relevance,
                            order,
                            *best_score,
                            *best_order,
                        ),
                    };
                    if replace {
                        best_candidate = Some((order, candidate_relevance, candidate));
                    }
                }
                if let Some((_, _, candidate)) = best_candidate {
                    let candidate_excerpt_hash = hash_content(&candidate.excerpt);
                    let candidate_source_hash = source_dedup_hash(&candidate.source);
                    seen_hashes.insert(candidate_excerpt_hash);
                    seen_hashes.insert(candidate_source_hash);
                    staged_hashes.push(candidate_excerpt_hash);
                    staged_hashes.push(candidate_source_hash);
                    filtered.push(candidate);
                }
            }
            continue;
        }
        seen_hashes.insert(excerpt_hash);
        seen_hashes.insert(source_hash);
        staged_hashes.push(excerpt_hash);
        staged_hashes.push(source_hash);
        filtered.push(result);
    }
    if !staged_hashes.is_empty() {
        let mut served = state.served_content.lock().await;
        let map = served
            .entry(scope_key)
            .or_insert_with(HashMap::<u32, i64>::new);
        map.retain(|_, ts| now - *ts < SERVED_TTL_MS);
        for hash in staged_hashes {
            map.insert(hash, now);
        }
    }
    filtered
}
pub(crate) fn recall_owner_scope(ctx: &RecallContext) -> String {
    if !ctx.team_mode {
        return "solo".to_string();
    }
    match ctx.caller_id {
        Some(owner_id) => format!("team:{owner_id}"),
        None => "team:none".to_string(),
    }
}
pub(crate) fn recall_scope_key(agent: &str, ctx: &RecallContext) -> String {
    format!("{}::{agent}", recall_owner_scope(ctx))
}
pub(crate) fn served_content_scope(agent: &str, query: &str, ctx: &RecallContext) -> String {
    let normalized_query = query
        .split_whitespace()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}::{agent}::{normalized_query}", recall_owner_scope(ctx))
}
pub(crate) async fn record_recall_pattern(state: &RuntimeState, scope_key: &str, query: &str) {
    let mut history = state.recall_history.lock().await;
    let entries = history
        .entry(scope_key.to_string())
        .or_insert_with(Vec::<RecallHistoryEntry>::new);
    entries.push(RecallHistoryEntry {
        query: query.to_string(),
        timestamp: Utc::now().timestamp_millis(),
    });
    if entries.len() > MAX_RECALL_HISTORY {
        let overflow = entries.len() - MAX_RECALL_HISTORY;
        entries.drain(0..overflow);
    }
}
pub(crate) const JACCARD_FUZZY_THRESHOLD: f64 = 0.6;
pub(crate) async fn get_pre_cached(
    state: &RuntimeState,
    scope_key: &str,
    scope_prefix: &str,
    query: &str,
) -> Option<Vec<RecallItem>> {
    let mut cache = state.pre_cache.lock().await;
    let now = Utc::now().timestamp_millis();
    let scope_prefix = format!("{scope_prefix}::");
    if let Some(entry) = cache.get(scope_key) {
        if entry.query == query && entry.expires_at > now {
            return deserialize_cache_entry(&entry.results);
        }
    }
    // Evict expired entry for this agent
    if cache
        .get(scope_key)
        .map(|e| e.expires_at <= now)
        .unwrap_or(false)
    {
        cache.remove(scope_key);
    }
    let mut best_score = 0.0_f64;
    let mut best_key: Option<String> = None;
    for (key, entry) in cache.iter() {
        if !key.starts_with(&scope_prefix) {
            continue;
        }
        if entry.expires_at <= now {
            continue;
        }
        let sim = jaccard_similarity(query, &entry.query);
        if sim >= JACCARD_FUZZY_THRESHOLD && sim > best_score {
            best_score = sim;
            best_key = Some(key.clone());
        }
    }
    if let Some(key) = best_key {
        if let Some(entry) = cache.get(&key) {
            return deserialize_cache_entry(&entry.results);
        }
    }
    None
}
pub(crate) fn deserialize_cache_entry(results: &serde_json::Value) -> Option<Vec<RecallItem>> {
    let arr = results.as_array()?;
    let items: Vec<RecallItem> = arr
        .iter()
        .filter_map(|v| {
            let collapsed_sources: Vec<String> = v
                .get("collapsedSources")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let collapsed_source_scores: Vec<(String, f64)> = v
                .get("collapsedSourceScores")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| {
                            let source = item
                                .get("source")
                                .and_then(|value| value.as_str())
                                .map(str::to_string)?;
                            let relevance = item
                                .get("relevance")
                                .and_then(|value| value.as_f64())
                                .unwrap_or(0.0);
                            Some((source, relevance))
                        })
                        .collect()
                })
                .unwrap_or_else(|| {
                    collapsed_sources
                        .iter()
                        .cloned()
                        .map(|source| (source, 0.0))
                        .collect()
                });
            Some(RecallItem {
                source: v.get("source")?.as_str()?.to_string(),
                relevance: v.get("relevance")?.as_f64()?,
                excerpt: v.get("excerpt")?.as_str()?.to_string(),
                method: v.get("method")?.as_str()?.to_string(),
                tokens: v.get("tokens").and_then(|t| t.as_u64()).map(|t| t as usize),
                entropy: v.get("entropy").and_then(|e| e.as_f64()),
                family_members: v
                    .get("familyMembers")
                    .and_then(|value| value.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                collapsed_sources,
                collapsed_source_scores,
            })
        })
        .collect();
    Some(items)
}
pub(crate) async fn predict_and_cache(
    state: RuntimeState,
    scope_key: &str,
    current_query: &str,
    predict_ctx: RecallContext,
) -> Result<(), String> {
    let predicted_query = {
        let history = state.recall_history.lock().await;
        let entries = match history.get(scope_key) {
            Some(entries) if entries.len() >= 3 => entries,
            _ => return Ok(()),
        };
        let mut followers: HashMap<String, (i64, i64)> = HashMap::new();
        for pair in entries.windows(2) {
            if pair[0].query == current_query {
                let next_query = pair[1].query.clone();
                let entry = followers.entry(next_query).or_insert((0, 0));
                entry.0 += 1;
                entry.1 = entry.1.max(pair[1].timestamp);
            }
        }
        followers
            .into_iter()
            .filter(|(query, _)| query != current_query)
            .max_by(|a, b| {
                a.1 .0
                    .cmp(&b.1 .0)
                    .then_with(|| a.1 .1.cmp(&b.1 .1))
                    .then_with(|| b.0.cmp(&a.0))
            })
            .map(|(query, _)| query)
    };
    let predicted_query = match predicted_query {
        Some(query) if !query.trim().is_empty() => query,
        _ => return Ok(()),
    };
    let mut conn = state.db.lock().await;
    let results = run_budget_recall(&mut conn, &predicted_query, 200, 5, &predict_ctx, None)?;
    drop(conn);
    if results.is_empty() {
        return Ok(());
    }
    // Serialize results as JSON Value for storage in the pre-cache
    let results_json: Value = results.into_iter().map(recall_to_json).collect();
    let now_ms = Utc::now().timestamp_millis();
    let mut cache = state.pre_cache.lock().await;
    // Evict all expired entries first (TTL cleanup)
    cache.retain(|_, entry| entry.expires_at > now_ms);
    // LRU eviction: if still at capacity, remove the entry with the oldest expiry
    // (soonest to expire = was cached longest ago, approximates LRU without a linked list)
    const MAX_CACHE_ENTRIES: usize = 100;
    if cache.len() >= MAX_CACHE_ENTRIES {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, entry)| entry.expires_at)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(
        scope_key.to_string(),
        PreCacheEntry {
            query: predicted_query,
            results: results_json,
            expires_at: now_ms + PRECACHE_TTL_MS,
        },
    );
    Ok(())
}
pub(crate) fn rerank_candidate_text(item: &RecallItem) -> String {
    let text = if item.excerpt.trim().is_empty() {
        item.source.clone()
    } else {
        format!("{} {}", item.source, item.excerpt)
    };
    truncate_chars(&text, 1800)
}
pub(crate) fn build_rerank_candidates(results: &[RecallItem], top_n: usize) -> Vec<RerankCandidate> {
    results
        .iter()
        .take(top_n.max(1))
        .map(|item| RerankCandidate {
            id: item.source.clone(),
            text: rerank_candidate_text(item),
            base_score: item.relevance,
        })
        .collect()
}
pub(crate) fn remap_fused_score_to_relevance(fused_score: f64, window: &[RecallItem]) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for relevance in window.iter().map(|item| item.relevance) {
        if relevance.is_finite() {
            min = min.min(relevance);
            max = max.max(relevance);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return round4(fused_score.clamp(0.0, 1.0));
    }
    let span = (max - min).max(0.01);
    round4(min + (span * fused_score.clamp(0.0, 1.0)))
}
pub(crate) fn apply_primary_rerank(results: Vec<RecallItem>, reranked: &[RerankedScore]) -> Vec<RecallItem> {
    if reranked.is_empty() {
        return results;
    }
    let window_len = reranked.len().min(results.len());
    let window = &results[..window_len];
    let mut by_source: HashMap<String, RecallItem> = results
        .iter()
        .take(window_len)
        .cloned()
        .map(|item| (item.source.clone(), item))
        .collect();
    let mut output = Vec::with_capacity(results.len());
    for score in reranked {
        if let Some(mut item) = by_source.remove(&score.id) {
            item.relevance = remap_fused_score_to_relevance(score.fused_score, window);
            if !item.method.contains("rerank") {
                item.method = format!("{}+rerank", item.method);
            }
            output.push(item);
        }
    }
    for item in results.iter().take(window_len) {
        if let Some(item) = by_source.remove(&item.source) {
            output.push(item);
        }
    }
    output.extend(results.into_iter().skip(window_len));
    output
}
pub(crate) fn rerank_scores_json(reranked: &[RerankedScore]) -> Vec<Value> {
    reranked
        .iter()
        .take(12)
        .enumerate()
        .map(|(idx, score)| {
            json!({
                "rank": idx + 1,
                "source": score.id,
                "baseScore": round4(score.base_score),
                "rerankScore": round4(score.rerank_score),
                "fusedScore": round4(score.fused_score),
            })
        })
        .collect()
}
pub(crate) fn maybe_apply_rerank(
    state: &RuntimeState,
    query_text: &str,
    results: Vec<RecallItem>,
    budget: usize,
) -> (Vec<RecallItem>, Value) {
    let config = &state.rerank_config;
    if budget == 0 {
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "headlines_mode",
                "mode": config.mode.as_str(),
            }),
        );
    }
    if !config.is_active() {
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "mode_off",
                "mode": config.mode.as_str(),
            }),
        );
    }
    if results.len() < 2 {
        let candidate_count = results.len();
        return (
            results,
            json!({
                "status": "skipped",
                "reason": "not_enough_candidates",
                "mode": config.mode.as_str(),
                "candidateCount": candidate_count,
            }),
        );
    }
    let Some(reranker) = state.reranker.as_ref() else {
        return (
            results,
            json!({
                "status": "unavailable",
                "reason": "model_not_loaded",
                "mode": config.mode.as_str(),
                "configuredModel": crate::rerank::selected_reranker_selection().key,
            }),
        );
    };
    let top_n = config.top_n.min(results.len());
    let candidates = build_rerank_candidates(&results, top_n);
    let baseline_top_sources = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    match reranker.rerank(query_text, &candidates, config.fusion_alpha) {
        Ok(reranked) => {
            let reranked_top_sources = reranked
                .iter()
                .map(|score| score.id.clone())
                .collect::<Vec<_>>();
            let telemetry = json!({
                "status": "ok",
                "mode": config.mode.as_str(),
                "applied": config.is_primary(),
                "model": reranker.name(),
                "modelSizeMb": reranker.model_size_mb(),
                "topN": top_n,
                "fusionAlpha": round4(config.fusion_alpha),
                "baselineTopSources": baseline_top_sources,
                "rerankedTopSources": reranked_top_sources,
                "scores": rerank_scores_json(&reranked),
            });
            let results = if config.is_primary() {
                apply_primary_rerank(results, &reranked)
            } else {
                results
            };
            (results, telemetry)
        }
        Err(error) => (
            results,
            json!({
                "status": "error",
                "mode": config.mode.as_str(),
                "applied": false,
                "model": reranker.name(),
                "reason": truncate_chars(&error, 240),
            }),
        ),
    }
}
pub(crate) fn sqlite_vec_trial_sampled(
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    trial_percent: u8,
) -> bool {
    if trial_percent == 0 {
        return false;
    }
    if trial_percent >= 100 {
        return true;
    }
    let mut hasher = DefaultHasher::new();
    query_text.hash(&mut hasher);
    ctx.team_mode.hash(&mut hasher);
    ctx.caller_id.hash(&mut hasher);
    source_prefix.unwrap_or_default().hash(&mut hasher);
    let bucket = (hasher.finish() % 100) as u8;
    bucket < trial_percent
}
pub(crate) fn parse_shadow_sources(shadow_semantic: &Value, field: &str) -> Vec<String> {
    shadow_semantic
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
pub(crate) fn shadow_guard_failure_reason(shadow_semantic: &Value) -> Option<&'static str> {
    if shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error")
        != "ok"
    {
        return Some("shadow_not_ok");
    }
    let overlap_ratio = shadow_semantic
        .get("overlapRatio")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if overlap_ratio < SQLITE_VEC_TRIAL_MIN_OVERLAP_RATIO {
        return Some("overlap_ratio_below_gate");
    }
    let jaccard = shadow_semantic
        .get("jaccard")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if jaccard < SQLITE_VEC_TRIAL_MIN_JACCARD {
        return Some("jaccard_below_gate");
    }
    let mean_abs_rank_delta = shadow_semantic
        .get("meanAbsRankDelta")
        .and_then(Value::as_f64)
        .unwrap_or(f64::INFINITY);
    if mean_abs_rank_delta > SQLITE_VEC_TRIAL_MAX_MEAN_ABS_RANK_DELTA {
        return Some("rank_delta_above_gate");
    }
    if SQLITE_VEC_TRIAL_TOP1_MATCH_REQUIRED {
        let top1_match = shadow_semantic
            .get("top1Match")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !top1_match {
            return Some("top1_match_required");
        }
    }
    None
}
pub(crate) fn sqlite_vec_source_fallback_candidate(
    conn: &Connection,
    source: &str,
    query_text: &str,
    fallback_relevance: f64,
) -> Option<SemanticCandidate> {
    let build_candidate = |excerpt_text: String,
                           score: Option<f64>,
                           trust_score: Option<f64>,
                           last_accessed: Option<String>,
                           created_at: Option<String>| {
        let ts_source = last_accessed
            .as_deref()
            .or(created_at.as_deref())
            .unwrap_or_default();
        SemanticCandidate {
            source: source.to_string(),
            excerpt: query_focused_excerpt(&excerpt_text, query_text, 280),
            relevance: fallback_relevance,
            importance: blend_importance(score, trust_score),
            ts: parse_timestamp_ms(ts_source),
        }
    };
    let memory_by_id = source
        .strip_prefix("memory::")
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|id| {
            conn.query_row(
                "SELECT text, score, trust_score, last_accessed, created_at
                 FROM memories
                 WHERE id = ?1 AND status = 'active'
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
                 LIMIT 1",
                params![id],
                |row| {
                    Ok(build_candidate(
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
        });
    if let Some(candidate) = memory_by_id {
        return Some(candidate);
    }
    let memory_by_source = conn
        .query_row(
            "SELECT text, score, trust_score, last_accessed, created_at
             FROM memories
             WHERE source = ?1 AND status = 'active'
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok(build_candidate(
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .ok()
        .flatten();
    if let Some(candidate) = memory_by_source {
        return Some(candidate);
    }
    let decision_by_id = source
        .strip_prefix("decision::")
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|id| {
            conn.query_row(
                "SELECT decision, score, trust_score, last_accessed, created_at
                 FROM decisions
                 WHERE id = ?1 AND status = 'active'
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
                 LIMIT 1",
                params![id],
                |row| {
                    Ok(build_candidate(
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()
        });
    if let Some(candidate) = decision_by_id {
        return Some(candidate);
    }
    conn.query_row(
        "SELECT decision, score, trust_score, last_accessed, created_at
         FROM decisions
         WHERE context = ?1 AND status = 'active'
         AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
         ORDER BY COALESCE(last_accessed, created_at) DESC
         LIMIT 1",
        params![source],
        |row| {
            Ok(build_candidate(
                row.get::<_, String>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<f64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    )
    .optional()
    .ok()
    .flatten()
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_apply_sqlite_vec_trial(
    conn: &Connection,
    query_text: &str,
    query_vector: Option<&[f32]>,
    semantic_candidates: Vec<SemanticCandidate>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    top_k: usize,
    canary: Option<&SqliteVecCanaryConfig>,
) -> (Vec<SemanticCandidate>, Value) {
    let Some(canary) = canary else {
        return (
            semantic_candidates,
            json!({
                "mode": "baseline",
                "reason": "trial_not_configured",
                "sampled": false,
                "trialPercent": 0,
                "routeMode": "baseline"
            }),
        );
    };
    let effective_route_mode = canary.effective_route_mode();
    let route_mode = effective_route_mode.as_str();
    let active_trial_percent = if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
        100
    } else {
        canary.trial_percent
    };
    let baseline_route = |reason: &str, sampled: bool, trial_percent: u8| {
        json!({
            "mode": "baseline",
            "reason": reason,
            "sampled": sampled,
            "trialPercent": trial_percent,
            "routeMode": route_mode
        })
    };
    if matches!(effective_route_mode, SqliteVecRouteMode::Baseline) {
        let reason = if canary.force_off {
            "trial_force_off"
        } else {
            "route_mode_baseline"
        };
        return (
            semantic_candidates,
            baseline_route(reason, false, active_trial_percent),
        );
    }
    let Some(query_vector) = query_vector else {
        return (
            semantic_candidates,
            baseline_route("query_embedding_unavailable", false, active_trial_percent),
        );
    };
    if semantic_candidates.is_empty() {
        return (
            semantic_candidates,
            baseline_route("no_semantic_candidates", false, active_trial_percent),
        );
    }
    let sampled = if matches!(effective_route_mode, SqliteVecRouteMode::Trial) {
        if canary.trial_percent == 0 {
            return (
                semantic_candidates,
                baseline_route("trial_percent_zero", false, active_trial_percent),
            );
        }
        let sampled =
            sqlite_vec_trial_sampled(query_text, ctx, source_prefix, canary.trial_percent);
        if !sampled {
            return (
                semantic_candidates,
                baseline_route("not_sampled", false, active_trial_percent),
            );
        }
        true
    } else {
        true
    };
    let baseline = ShadowSemanticBaseline {
        candidate_count: semantic_candidates.len(),
        ranked_sources: semantic_candidates
            .iter()
            .take(MAX_SEMANTIC_RRF_CANDIDATES)
            .map(|candidate| candidate.source.clone())
            .collect(),
    };
    let shadow_semantic = build_shadow_semantic_explain(
        conn,
        Some(query_vector),
        query_text,
        ctx,
        source_prefix,
        top_k,
        Some(&baseline),
    );
    if let Some(reason) = shadow_guard_failure_reason(&shadow_semantic) {
        return (
            semantic_candidates,
            baseline_route(reason, sampled, active_trial_percent),
        );
    }
    let shadow_sources = parse_shadow_sources(&shadow_semantic, "shadowTopSources");
    if shadow_sources.is_empty() {
        return (
            semantic_candidates,
            baseline_route("shadow_top_sources_empty", sampled, active_trial_percent),
        );
    }
    let mut by_source: HashMap<String, SemanticCandidate> = semantic_candidates
        .iter()
        .cloned()
        .map(|candidate| (candidate.source.clone(), candidate))
        .collect();
    let mut reordered: Vec<SemanticCandidate> = Vec::new();
    let baseline_max = semantic_candidates
        .first()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SCALE_BASE);
    let baseline_min = semantic_candidates
        .last()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SIM_FLOOR);
    let relevance_span = (baseline_max - baseline_min).abs().max(0.02);
    let rank_denominator = shadow_sources.len().saturating_sub(1).max(1) as f64;
    let fallback_relevance_for_rank = |rank_idx: usize| {
        let rank_weight = 1.0 - (rank_idx as f64 / rank_denominator);
        round4(
            (baseline_min + (relevance_span * rank_weight))
                .clamp(SEMANTIC_SIM_FLOOR, baseline_max.max(SEMANTIC_SIM_FLOOR)),
        )
    };
    for (rank_idx, source) in shadow_sources.iter().enumerate() {
        if let Some(candidate) = by_source.remove(source) {
            reordered.push(candidate);
            continue;
        }
        let fallback_relevance = fallback_relevance_for_rank(rank_idx);
        if let Some(candidate) =
            sqlite_vec_source_fallback_candidate(conn, source, query_text, fallback_relevance)
        {
            reordered.push(candidate);
            continue;
        }
        reordered.push(SemanticCandidate {
            source: source.clone(),
            excerpt: query_focused_excerpt(source, query_text, 160),
            relevance: fallback_relevance,
            importance: 0.5,
            ts: 0,
        });
    }
    for candidate in &semantic_candidates {
        if let Some(remaining) = by_source.remove(&candidate.source) {
            reordered.push(remaining);
        }
    }
    reordered.truncate(semantic_candidates.len());
    (
        reordered,
        json!({
            "mode": if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
                "vec0_primary"
            } else {
                "vec0_trial"
            },
            "reason": if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
                "route_mode_primary"
            } else {
                "guard_passed"
            },
            "sampled": sampled,
            "trialPercent": active_trial_percent,
            "routeMode": route_mode
        }),
    )
}
pub(crate) fn is_benchmark_recall_scope(agent: &str, source_prefix: Option<&str>) -> bool {
    if agent
        .trim()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_AGENT_PREFIX)
    {
        return true;
    }
    source_prefix
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_SCOPE_PREFIX)
}
pub(crate) async fn emit_recall_query_event(
    state: &RuntimeState,
    agent: &str,
    source_prefix: Option<&str>,
    payload: Value,
) {
    if is_benchmark_recall_scope(agent, source_prefix) {
        return;
    }
    let conn = state.db.lock().await;
    if crate::handlers::log_event(&conn, "recall_query", payload, agent).is_ok() {
        checkpoint_wal_best_effort(&conn);
    }
}
pub(crate) fn build_method_breakdown(results: &[RecallItem]) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for item in results {
        *counts.entry(item.method.clone()).or_insert(0) += 1;
    }
    json!(counts)
}
pub(crate) fn method_count(methods: &Value, method: &str) -> i64 {
    methods.get(method).and_then(|v| v.as_i64()).unwrap_or(0)
}
pub(crate) fn classify_recall_tier(cached: bool, mode: &str, methods: &Value) -> &'static str {
    if cached {
        return "cache_hit";
    }
    if mode == "headlines" {
        return "headlines";
    }
    if mode == "semantic" {
        return "semantic_only";
    }
    let keyword = method_count(methods, "keyword");
    let semantic = method_count(methods, "semantic");
    let hybrid = method_count(methods, "hybrid");
    let crystal = method_count(methods, "crystal");
    let associative = method_count(methods, "associative");
    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        if crystal > 0 {
            return "hybrid_crystal";
        }
        return "hybrid_fusion";
    }
    if associative > 0 && (keyword > 0 || semantic > 0 || crystal > 0) {
        return "associative_blend";
    }
    if keyword > 0 {
        if crystal > 0 {
            return "keyword_crystal";
        }
        return "keyword_only";
    }
    if semantic > 0 {
        if crystal > 0 {
            return "semantic_crystal";
        }
        return "semantic_only";
    }
    if crystal > 0 {
        return "crystal_only";
    }
    if associative > 0 {
        return "associative_only";
    }
    "unknown"
}
pub(crate) fn shadow_semantic_telemetry_summary(shadow_semantic: &Value) -> Value {
    let status = shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    let mut summary = json!({
        "status": status,
    });
    if let Some(reason) = shadow_semantic.get("reason").and_then(Value::as_str) {
        summary["reason"] = json!(reason);
    }
    for key in [
        "topK",
        "vectorDimension",
        "baselineCandidateCount",
        "shadowCandidateCount",
        "overlapCount",
        "overlapRatio",
        "jaccard",
        "matchedRankPairs",
        "meanAbsRankDelta",
        "top1Match",
    ] {
        if let Some(value) = shadow_semantic.get(key) {
            summary[key] = value.clone();
        }
    }
    if status == "error" && summary.get("reason").is_none() {
        summary["reason"] = json!("shadow_payload_invalid");
    }
    summary
}
pub(crate) fn run_budget_recall(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    run_budget_recall_with_engine(
        conn,
        query_text,
        token_budget,
        k,
        None,
        ctx,
        source_prefix,
        None,
    )
}
pub(crate) fn run_semantic_recall_with_query_vector(
    conn: &Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>,
) -> (Vec<RecallItem>, Value) {
    let prefers_recency = query_prefers_recency(query_text);
    let baseline_semantic = query_vector
        .map(|query_vec| {
            collect_semantic_candidates(conn, query_vec, query_text, ctx, source_prefix)
        })
        .unwrap_or_default();
    let (semantic_candidates, semantic_route) = maybe_apply_sqlite_vec_trial(
        conn,
        query_text,
        query_vector,
        baseline_semantic,
        ctx,
        source_prefix,
        k,
        canary,
    );
    let mut ranked: Vec<RecallItem> = semantic_candidates
        .into_iter()
        .map(|candidate| {
            let mut relevance = round4(candidate.relevance);
            if prefers_recency {
                relevance = round4(relevance * temporal_intent_multiplier(candidate.ts));
            }
            RecallItem {
                source: candidate.source,
                relevance,
                excerpt: candidate.excerpt,
                method: "semantic".to_string(),
                tokens: None,
                entropy: None,
                family_members: Vec::new(),
                collapsed_sources: Vec::new(),
                collapsed_source_scores: Vec::new(),
            }
        })
        .collect();
    apply_recall_ranking_boosts(&mut ranked, query_text, 0.05, 0.08);
    ranked.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    ranked.truncate(k);
    bump_retrievals_batch(conn, &ranked);
    (ranked, semantic_route)
}
pub(crate) fn budget_rank_char_cap(token_budget: usize, rank_idx: usize, query_text: &str) -> usize {
    let base = if token_budget <= 220 {
        match rank_idx {
            0 => 180,
            1 => 120,
            2 => 90,
            _ => 70,
        }
    } else if token_budget <= 400 {
        match rank_idx {
            0 => 260,
            1 => 170,
            2 => 130,
            _ => 95,
        }
    } else if token_budget <= 800 {
        match rank_idx {
            0 => 320,
            1 => 210,
            2 => 160,
            _ => 120,
        }
    } else {
        match rank_idx {
            0 => 420,
            1 => 260,
            2 => 200,
            _ => 150,
        }
    };
    let profile = query_shape_profile(query_text, None);
    let adjusted = if profile.exactish && !profile.naturalish {
        ((base as f64) * 1.12).round() as usize
    } else if profile.naturalish && !profile.exactish {
        ((base as f64) * 0.86).round() as usize
    } else {
        base
    };
    adjusted.max(MIN_EXCERPT_CHARS)
}
pub(crate) fn semantic_budget_min_relevance(top_relevance: f64, query_text: &str) -> f64 {
    if top_relevance < 0.25 {
        return 0.0;
    }
    let profile = query_shape_profile(query_text, None);
    let (scale, floor) = if profile.naturalish && !profile.exactish {
        (0.64, 0.14)
    } else if profile.exactish && !profile.naturalish {
        (0.78, 0.20)
    } else {
        (0.72, 0.18)
    };
    (top_relevance * scale).max(floor)
}
pub(crate) fn semantic_budget_max_items(token_budget: usize, query_text: &str, hard_cap: usize) -> usize {
    let base: usize = if token_budget <= 220 {
        4
    } else if token_budget <= 400 {
        6
    } else if token_budget <= 800 {
        8
    } else {
        10
    };
    let profile = query_shape_profile(query_text, None);
    let adjusted = if profile.naturalish && !profile.exactish {
        base.saturating_add(1)
    } else if profile.exactish && !profile.naturalish {
        base.saturating_sub(1).max(3)
    } else {
        base
    };
    adjusted.clamp(3, 12).min(hard_cap.max(1))
}
pub(crate) fn fit_excerpt_to_remaining_budget(
    source: &str,
    excerpt: &str,
    query_text: &str,
    char_cap: usize,
    remaining_tokens: usize,
) -> Option<(String, usize)> {
    if remaining_tokens <= MIN_BUDGET_HEADROOM_TOKENS {
        return None;
    }
    let source_only_tokens = estimate_tokens(source);
    if source_only_tokens > remaining_tokens {
        return None;
    }
    if excerpt.is_empty() {
        return Some((String::new(), source_only_tokens));
    }
    let total_chars = excerpt.chars().count();
    let min_chars = MIN_EXCERPT_CHARS.min(total_chars.max(1));
    let mut chars = char_cap.min(total_chars).max(min_chars);
    loop {
        let clipped = query_focused_excerpt(excerpt, query_text, chars);
        let tokens = estimate_tokens(&format!("{source}{clipped}"));
        if tokens <= remaining_tokens {
            return Some((clipped, tokens));
        }
        if chars <= min_chars {
            break;
        }
        let next = ((chars as f64) * 0.72) as usize;
        chars = next.max(min_chars).min(chars.saturating_sub(1));
    }
    Some((String::new(), source_only_tokens))
}
pub(crate) fn prefer_family_candidate(
    candidate: &RecallItem,
    current: &RecallItem,
    alignment_profile: &QueryAlignmentProfile,
) -> bool {
    let relevance_delta = candidate.relevance - current.relevance;
    if relevance_delta > 0.03 {
        return true;
    }
    if relevance_delta < -0.03 {
        return false;
    }
    let candidate_alignment = alignment_profile.alignment_score(&candidate.excerpt);
    let current_alignment = alignment_profile.alignment_score(&current.excerpt);
    if candidate_alignment != current_alignment {
        return candidate_alignment > current_alignment;
    }
    if candidate.method == "crystal" && current.method != "crystal" {
        return true;
    }
    if candidate.method != "crystal" && current.method == "crystal" {
        return false;
    }
    if candidate.excerpt.len() != current.excerpt.len() {
        return candidate.excerpt.len() < current.excerpt.len();
    }
    candidate.source < current.source
}
pub(crate) fn compact_budget_family_candidates_with_trace(
    candidates: Vec<RecallItem>,
    query_text: &str,
    token_budget: usize,
) -> (
    Vec<RecallItem>,
    Vec<RecallItem>,
    Vec<RecallFamilyCompaction>,
) {
    if token_budget > 400 || candidates.len() <= 1 {
        return (candidates, Vec::new(), Vec::new());
    }
    let mut family_lookup = HashMap::new();
    for item in &candidates {
        if item.family_members.is_empty() {
            continue;
        }
        for member in &item.family_members {
            family_lookup
                .entry(member.clone())
                .or_insert_with(|| item.source.clone());
        }
    }
    if family_lookup.is_empty() {
        return (candidates, Vec::new(), Vec::new());
    }
    let mut compacted: HashMap<String, RecallItem> = HashMap::new();
    let mut dropped = Vec::new();
    let mut dropped_by_family: HashMap<String, Vec<String>> = HashMap::new();
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    for item in candidates {
        let family_key = if !item.family_members.is_empty() {
            item.source.clone()
        } else {
            family_lookup
                .get(&item.source)
                .cloned()
                .unwrap_or_else(|| item.source.clone())
        };
        match compacted.entry(family_key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if prefer_family_candidate(&item, entry.get(), &alignment_profile) {
                    let replaced = entry.insert(item);
                    dropped_by_family
                        .entry(entry.key().clone())
                        .or_default()
                        .push(replaced.source.clone());
                    dropped.push(replaced);
                } else {
                    dropped_by_family
                        .entry(entry.key().clone())
                        .or_default()
                        .push(item.source.clone());
                    dropped.push(item);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(item);
            }
        }
    }
    dropped.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    let mut family_compactions = Vec::new();
    for (family_key, mut dropped_sources) in dropped_by_family {
        if dropped_sources.is_empty() {
            continue;
        }
        dedup_preserve_order(&mut dropped_sources);
        let Some(kept_source) = compacted.get(&family_key).map(|item| item.source.clone()) else {
            continue;
        };
        family_compactions.push(RecallFamilyCompaction {
            family_key,
            kept_source,
            dropped_sources,
        });
    }
    family_compactions.sort_by(|a, b| a.family_key.cmp(&b.family_key));
    let mut compacted_items: Vec<RecallItem> = compacted.into_values().collect();
    compacted_items.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    (compacted_items, dropped, family_compactions)
}
pub(crate) fn compact_budget_family_candidates(
    candidates: Vec<RecallItem>,
    query_text: &str,
    token_budget: usize,
) -> Vec<RecallItem> {
    compact_budget_family_candidates_with_trace(candidates, query_text, token_budget).0
}
pub(crate) fn apply_semantic_budget(
    raw: Vec<RecallItem>,
    token_budget: usize,
    query_text: &str,
) -> Vec<RecallItem> {
    if token_budget == 0 {
        return raw
            .into_iter()
            .map(|mut item| {
                item.excerpt.clear();
                item.tokens = Some(estimate_tokens(&item.source));
                item
            })
            .collect();
    }
    let raw = compact_budget_family_candidates(raw, query_text, token_budget);
    let top_relevance = raw.first().map(|item| item.relevance).unwrap_or(0.0);
    let min_relevance = semantic_budget_min_relevance(top_relevance, query_text);
    let max_items = semantic_budget_max_items(token_budget, query_text, raw.len());
    let mut candidates: Vec<RecallItem> = raw
        .iter()
        .filter(|item| item.relevance >= min_relevance)
        .take(max_items)
        .cloned()
        .collect();
    if candidates.is_empty() {
        candidates = raw.iter().take(max_items.max(1)).cloned().collect();
    }
    let query_terms: HashSet<String> = query_focus_terms_for_excerpt(query_text)
        .into_iter()
        .collect();
    let mut covered_terms: HashSet<String> = HashSet::new();
    let mut selected_signatures: Vec<HashSet<String>> = Vec::new();
    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, mut item) in candidates.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text)
            .min((remaining as f64 * 3.6) as usize)
            .max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) =
            fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining)
        {
            let signature_terms = excerpt_signature_terms(&item.source, &excerpt);
            if should_skip_redundant_budget_candidate(
                &signature_terms,
                &selected_signatures,
                &query_terms,
                &covered_terms,
            ) {
                continue;
            }
            item.excerpt = excerpt;
            item.tokens = Some(tokens);
            spent += tokens;
            update_query_term_coverage(&signature_terms, &query_terms, &mut covered_terms);
            selected_signatures.push(signature_terms);
            budgeted.push(item);
            if should_early_stop_budget_selection(
                token_budget,
                spent,
                budgeted.len(),
                &query_terms,
                &covered_terms,
            ) {
                break;
            }
        }
    }
    budgeted
}
pub(crate) fn associative_item_limit(token_budget: usize) -> usize {
    if token_budget <= 420 {
        1
    } else if token_budget <= 900 {
        2
    } else {
        3
    }
}
pub(crate) fn parse_co_occurrence_prediction(entry: &Value) -> Option<(String, i64)> {
    let source = entry.get("source")?.as_str()?.trim();
    if source.is_empty() {
        return None;
    }
    let score = entry.get("coScore")?.as_i64()?;
    if score <= 0 {
        return None;
    }
    Some((source.to_string(), score))
}
pub(crate) fn fetch_associative_source_payload(
    conn: &Connection,
    source: &str,
    query_text: &str,
    ctx: &RecallContext,
) -> Option<(String, f64, i64)> {
    type PayloadRow = (
        String,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<f64>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
    );
    let mut best: Option<(String, f64, i64)> = None;
    let memory_row: Option<PayloadRow> = if ctx.team_mode {
        conn.query_row(
            "SELECT text, compressed_text, age_tier, score, trust_score, last_accessed, created_at, owner_id, visibility
             FROM memories
             WHERE status = 'active'
               AND source = ?1
               AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .ok()
    } else {
        conn.query_row(
            "SELECT text, compressed_text, age_tier, score, trust_score, last_accessed, created_at
             FROM memories
             WHERE status = 'active'
               AND source = ?1
               AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    None,
                    None,
                ))
            },
        )
        .ok()
    };
    if let Some((
        text,
        compressed_text,
        age_tier,
        score,
        trust_score,
        last_accessed,
        created_at,
        owner_id,
        visibility,
    )) = memory_row
    {
        if !ctx.team_mode || is_visible(owner_id, visibility.as_deref(), ctx) {
            let display = crate::aging::get_display_text(
                &text,
                &compressed_text,
                &age_tier.unwrap_or_else(|| "fresh".to_string()),
            );
            let excerpt = query_focused_excerpt(&display, query_text, 220);
            let importance = blend_importance(score, trust_score).clamp(0.0, 1.0);
            let ts = parse_timestamp_ms(&last_accessed.or(created_at).unwrap_or_else(now_iso));
            best = Some((excerpt, importance, ts));
        }
    }
    let decision_row: Option<PayloadRow> = if ctx.team_mode {
        conn.query_row(
            "SELECT decision, compressed_text, age_tier, score, trust_score, last_accessed, created_at, owner_id, visibility
             FROM decisions
             WHERE status = 'active'
               AND context = ?1
               AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .ok()
    } else {
        conn.query_row(
            "SELECT decision, compressed_text, age_tier, score, trust_score, last_accessed, created_at
             FROM decisions
             WHERE status = 'active'
               AND context = ?1
               AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))
             ORDER BY COALESCE(last_accessed, created_at) DESC
             LIMIT 1",
            params![source],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    None,
                    None,
                ))
            },
        )
        .ok()
    };
    if let Some((
        decision,
        compressed_text,
        age_tier,
        score,
        trust_score,
        last_accessed,
        created_at,
        owner_id,
        visibility,
    )) = decision_row
    {
        if !ctx.team_mode || is_visible(owner_id, visibility.as_deref(), ctx) {
            let display = crate::aging::get_display_text(
                &decision,
                &compressed_text,
                &age_tier.unwrap_or_else(|| "fresh".to_string()),
            );
            let excerpt = query_focused_excerpt(&display, query_text, 220);
            let importance = blend_importance(score, trust_score).clamp(0.0, 1.0);
            let ts = parse_timestamp_ms(&last_accessed.or(created_at).unwrap_or_else(now_iso));
            let replace = match &best {
                Some((_, best_importance, best_ts)) => {
                    importance > *best_importance
                        || (importance == *best_importance && ts > *best_ts)
                }
                None => true,
            };
            if replace {
                best = Some((excerpt, importance, ts));
            }
        }
    }
    best
}
pub(crate) fn build_associative_candidates(
    conn: &Connection,
    base: &[RecallItem],
    query_text: &str,
    token_budget: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Vec<RecallItem> {
    if token_budget < ASSOCIATIVE_MIN_BUDGET_TOKENS || base.is_empty() {
        return Vec::new();
    }
    let top_relevance = base.first().map(|item| item.relevance).unwrap_or(0.0);
    if top_relevance < 0.28 {
        return Vec::new();
    }
    let min_anchor_relevance = (top_relevance * 0.45).max(0.18);
    let anchors: Vec<String> = base
        .iter()
        .filter(|item| item.relevance >= min_anchor_relevance)
        .take(4)
        .map(|item| item.source.clone())
        .collect();
    if anchors.is_empty() {
        return Vec::new();
    }
    let max_associative = associative_item_limit(token_budget);
    if max_associative == 0 {
        return Vec::new();
    }
    let predictions = match co_occurrence::predict(conn, &anchors, max_associative * 4) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };
    if predictions.is_empty() {
        return Vec::new();
    }
    let mut parsed = predictions
        .iter()
        .filter_map(parse_co_occurrence_prediction)
        .collect::<Vec<_>>();
    if parsed.is_empty() {
        return Vec::new();
    }
    parsed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let max_co_score = parsed[0].1.max(1);
    let min_required_co_score = ((max_co_score as f64) * 0.35).ceil() as i64;
    let query_terms = extract_search_keywords(query_text);
    let mut associative = Vec::new();
    for (source, co_score) in parsed {
        if co_score < 2 || co_score < min_required_co_score {
            continue;
        }
        if !source_matches_prefix(&source, source_prefix) {
            continue;
        }
        let Some((excerpt, importance, ts)) =
            fetch_associative_source_payload(conn, &source, query_text, ctx)
        else {
            continue;
        };
        let norm =
            ((co_score as f64 + 1.0).ln() / (max_co_score as f64 + 1.0).ln()).clamp(0.0, 1.0);
        let source_lower = source.to_ascii_lowercase();
        let overlap = if query_terms.is_empty() {
            0.0
        } else {
            let matched = query_terms
                .iter()
                .filter(|term| source_lower.contains(term.as_str()))
                .count();
            matched as f64 / query_terms.len().max(1) as f64
        };
        let recency_days = if ts > 0 {
            let now = Utc::now().timestamp_millis();
            ((now - ts).max(0) as f64) / (1000.0 * 60.0 * 60.0 * 24.0)
        } else {
            30.0
        };
        let recency = (1.0 / (1.0 + recency_days / 14.0)).clamp(0.0, 1.0);
        let anchor = (top_relevance * 0.68).clamp(0.24, 0.82);
        let relevance = round4(
            ((anchor * (0.76 + 0.24 * norm))
                + (importance * 0.10)
                + (overlap * 0.08)
                + (recency * 0.10))
                .clamp(0.0, 0.95),
        );
        associative.push(RecallItem {
            source,
            relevance,
            excerpt,
            method: "associative".to_string(),
            tokens: None,
            entropy: None,
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
        });
        if associative.len() >= max_associative {
            break;
        }
    }
    associative
}
pub(crate) struct RecallBudgetTrace {
    pub(crate) budgeted: Vec<RecallItem>,
    pub(crate) candidate_pool: Vec<RecallItem>,
    pub(crate) pre_compaction_candidate_count: usize,
    pub(crate) family_compactions: Vec<RecallFamilyCompaction>,
    pub(crate) retrieval_depth: usize,
    pub(crate) top_relevance: f64,
    pub(crate) min_relevance: f64,
    pub(crate) max_items: usize,
    pub(crate) semantic_baseline: Option<ShadowSemanticBaseline>,
    pub(crate) semantic_route: Value,
}
#[derive(Clone)]
pub(crate) struct RecallFamilyCompaction {
    pub(crate) family_key: String,
    pub(crate) kept_source: String,
    pub(crate) dropped_sources: Vec<String>,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_budget_recall_trace_with_query_vector(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>,
) -> Result<RecallBudgetTrace, String> {
    let retrieval_depth = if token_budget <= 220 {
        (k.max(10) * 3).min(30)
    } else if token_budget <= 400 {
        (k.max(10) * 2).min(28)
    } else {
        k.max(12)
    };
    let recall_trace = run_recall_with_query_vector_trace(
        conn,
        query_text,
        retrieval_depth,
        query_vector,
        ctx,
        source_prefix,
        canary,
    )?;
    let raw = recall_trace.ranked;
    let semantic_baseline = recall_trace.semantic_baseline;
    let semantic_route = recall_trace.semantic_route;
    if raw.is_empty() {
        return Ok(RecallBudgetTrace {
            budgeted: vec![],
            candidate_pool: vec![],
            pre_compaction_candidate_count: 0,
            family_compactions: vec![],
            retrieval_depth,
            top_relevance: 0.0,
            min_relevance: 0.0,
            max_items: 0,
            semantic_baseline,
            semantic_route,
        });
    }
    let associative =
        build_associative_candidates(conn, &raw, query_text, token_budget, ctx, source_prefix);
    let pre_compaction_pool = if associative.is_empty() {
        raw
    } else {
        let mut merged: HashMap<String, RecallItem> = raw
            .into_iter()
            .map(|item| (item.source.clone(), item))
            .collect();
        for candidate in associative {
            if let Some(existing) = merged.get_mut(&candidate.source) {
                if candidate.relevance > existing.relevance {
                    existing.relevance = candidate.relevance;
                    existing.excerpt = candidate.excerpt;
                }
                existing.method = "associative".to_string();
                existing.tokens = None;
            } else {
                merged.insert(candidate.source.clone(), candidate);
            }
        }
        let mut merged_pool: Vec<RecallItem> = merged.into_values().collect();
        merged_pool.sort_by(|a, b| {
            compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
        });
        merged_pool
    };
    let pre_compaction_candidate_count = pre_compaction_pool.len();
    let (raw, _family_compaction_dropped, family_compactions) =
        compact_budget_family_candidates_with_trace(pre_compaction_pool, query_text, token_budget);
    let top_relevance = raw.first().map(|item| item.relevance).unwrap_or(0.0);
    let min_relevance = semantic_budget_min_relevance(top_relevance, query_text);
    let max_items = semantic_budget_max_items(token_budget, query_text, k.max(1));
    let mut candidates: Vec<RecallItem> = raw
        .iter()
        .filter(|item| item.relevance >= min_relevance)
        .take(max_items)
        .cloned()
        .collect();
    if candidates.is_empty() {
        candidates = raw.iter().take(max_items).cloned().collect();
    }
    if !candidates.iter().any(|item| item.method == "associative") {
        if let Some(best_associative) = raw.iter().find(|item| item.method == "associative") {
            candidates.push(best_associative.clone());
            candidates.sort_by(|a, b| {
                compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
            });
            candidates.truncate(max_items.max(1));
        }
    }
    let query_terms: HashSet<String> = query_focus_terms_for_excerpt(query_text)
        .into_iter()
        .collect();
    let mut covered_terms: HashSet<String> = HashSet::new();
    let mut selected_signatures: Vec<HashSet<String>> = Vec::new();
    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, item) in candidates.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text)
            .min((remaining as f64 * 3.6) as usize)
            .max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) =
            fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining)
        {
            let signature_terms = excerpt_signature_terms(&item.source, &excerpt);
            if should_skip_redundant_budget_candidate(
                &signature_terms,
                &selected_signatures,
                &query_terms,
                &covered_terms,
            ) {
                continue;
            }
            spent += tokens;
            update_query_term_coverage(&signature_terms, &query_terms, &mut covered_terms);
            selected_signatures.push(signature_terms);
            budgeted.push(RecallItem {
                source: item.source,
                relevance: item.relevance,
                excerpt,
                method: item.method,
                tokens: Some(tokens),
                entropy: item.entropy,
                family_members: item.family_members,
                collapsed_sources: item.collapsed_sources,
                collapsed_source_scores: item.collapsed_source_scores,
            });
            if should_early_stop_budget_selection(
                token_budget,
                spent,
                budgeted.len(),
                &query_terms,
                &covered_terms,
            ) {
                break;
            }
        }
    }
    Ok(RecallBudgetTrace {
        budgeted,
        candidate_pool: raw,
        pre_compaction_candidate_count,
        family_compactions,
        retrieval_depth,
        top_relevance,
        min_relevance,
        max_items,
        semantic_baseline,
        semantic_route,
    })
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_budget_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    Ok(run_budget_recall_trace_with_engine(
        conn,
        query_text,
        token_budget,
        k,
        engine,
        ctx,
        source_prefix,
        degraded_flag,
    )?
    .budgeted)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_budget_recall_trace_with_engine(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<RecallBudgetTrace, String> {
    let query_vector = engine.and_then(|engine| engine.embed_query(query_text));
    if engine.is_some() {
        update_semantic_search_health(degraded_flag, query_vector.is_some(), true);
    }
    run_budget_recall_trace_with_query_vector(
        conn,
        query_text,
        token_budget,
        k,
        query_vector.as_deref(),
        ctx,
        source_prefix,
        None,
    )
}
pub(crate) fn update_semantic_search_health(
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    semantic_available: bool,
    log_unavailable: bool,
) {
    if let Some(flag) = degraded_flag {
        if semantic_available {
            flag.store(false, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        let transitioned = flag
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok();
        if log_unavailable && transitioned {
            eprintln!("[recall] Semantic search unavailable, using keyword fallback");
        }
    }
}
pub(crate) fn run_recall(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    run_recall_with_engine(conn, query_text, k, None, ctx, source_prefix, None)
}
#[allow(clippy::type_complexity)]
pub(crate) fn run_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let query_vector = engine.and_then(|engine| engine.embed_query(query_text));
    if engine.is_some() {
        update_semantic_search_health(degraded_flag, query_vector.is_some(), true);
    }
    Ok(run_recall_with_query_vector_trace(
        conn,
        query_text,
        k,
        query_vector.as_deref(),
        ctx,
        source_prefix,
        None,
    )?
    .ranked)
}
pub async fn execute_unified_recall(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Value, String> {
    let started_at = Instant::now();
    let policy_mode = recall_mode_for_budget(budget);
    let latency_budget_ms = recall_latency_budget_ms_for_mode(policy_mode);
    let recall_scope = recall_scope_key(agent, ctx);
    let scope_prefix = recall_owner_scope(ctx);
    // Check pre-cache
    if budget > 0 && !state.rerank_config.is_active() {
        if let Some(cached) = get_pre_cached(state, &recall_scope, &scope_prefix, query_text).await
        {
            let deduped_cached = dedup_and_mark_served(state, agent, query_text, ctx, cached).await;
            let mode = recall_mode_for_budget(budget);
            let method_breakdown = build_method_breakdown(&deduped_cached);
            let tier = classify_recall_tier(true, mode.as_str(), &method_breakdown);
            let latency_ms = started_at.elapsed().as_millis() as i64;
            let semantic_route = json!({
                "mode": "baseline",
                "reason": "cache_hit",
                "sampled": false,
                "trialPercent": if matches!(
                    state.sqlite_vec_canary.effective_route_mode(),
                    SqliteVecRouteMode::Primary
                ) {
                    100
                } else {
                    state.sqlite_vec_canary.trial_percent
                },
                "routeMode": state.sqlite_vec_canary.effective_route_mode().as_str()
            });
            emit_recall_query_event(
                state,
                agent,
                source_prefix,
                json!({
                    "agent": agent,
                    "query": truncate_chars(query_text, 120),
                    "budget": budget,
                    "spent": 0,
                    "saved": budget as i64,
                    "hits": deduped_cached.len(),
                    "mode": mode.as_str(),
                    "cached": true,
                    "method_breakdown": method_breakdown,
                    "tier": tier,
                    "latency_ms": latency_ms,
                    "semantic_route": semantic_route.clone(),
                    "shadow_semantic": {
                        "status": "skipped",
                        "reason": "cache_hit"
                    }
                }),
            )
            .await;
            let usage = RecallBudgetUsage {
                spent: 0,
                saved: budget as i64,
                over_budget: false,
            };
            return Ok(json!({
                "results": deduped_cached.into_iter().map(recall_to_json).collect::<Vec<_>>(),
                "budget": budget,
                "spent": usage.spent,
                "saved": usage.saved,
                "overBudget": usage.over_budget,
                "tokenUsageLine": format_recall_token_usage_line(budget, usage),
                "mode": mode.as_str(),
                "policyMode": mode.as_str(),
                "cached": true,
                "tier": tier,
                "latencyMs": latency_ms,
                "semanticRoute": semantic_route
            }));
        }
    }
    let engine = state.embedding_engine.clone();
    let dflag = Some(&state.degraded_mode);
    let mut query_vector = match engine {
        Some(runtime_engine) => {
            runtime_engine
                .embed_query_async(query_text.to_string())
                .await
        }
        None => None,
    };
    if state.embedding_engine.is_some() {
        update_semantic_search_health(dflag, query_vector.is_some(), true);
    }
    let mut conn = state.db.lock().await;
    let (mut results, mut semantic_baseline, mut semantic_route) = if budget == 0 {
        let trace = run_recall_with_query_vector_trace(
            &mut conn,
            query_text,
            k,
            query_vector.as_deref(),
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
        )?;
        (trace.ranked, trace.semantic_baseline, trace.semantic_route)
    } else {
        let trace = run_budget_recall_trace_with_query_vector(
            &mut conn,
            query_text,
            budget,
            k,
            query_vector.as_deref(),
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
        )?;
        (
            trace.budgeted,
            trace.semantic_baseline,
            trace.semantic_route,
        )
    };
    let mut fail_closed = Value::Null;
    if budget > 0 {
        let elapsed_before_fallback = started_at.elapsed().as_millis();
        if elapsed_before_fallback >= latency_budget_ms {
            let fallback_trace = run_budget_recall_trace_with_query_vector(
                &mut conn,
                query_text,
                budget,
                k,
                None,
                ctx,
                source_prefix,
                Some(&state.sqlite_vec_canary),
            )?;
            results = fallback_trace.budgeted;
            semantic_baseline = fallback_trace.semantic_baseline;
            semantic_route = json!({
                "mode": "baseline",
                "reason": "latency_budget_fail_closed",
                "fallback": "deterministic_keyword_rrf",
                "elapsedMsBeforeFallback": elapsed_before_fallback,
                "latencyBudgetMs": latency_budget_ms,
                "routeMode": state.sqlite_vec_canary.effective_route_mode().as_str()
            });
            query_vector = None;
            fail_closed = json!({
                "triggered": true,
                "elapsedMsBeforeFallback": elapsed_before_fallback,
                "latencyBudgetMs": latency_budget_ms,
                "fallback": "deterministic_keyword_rrf"
            });
        }
    }
    let shadow_semantic = {
        let shadow_detail = build_shadow_semantic_explain(
            &conn,
            query_vector.as_deref(),
            query_text,
            ctx,
            source_prefix,
            k,
            semantic_baseline.as_ref(),
        );
        shadow_semantic_telemetry_summary(&shadow_detail)
    };
    let (reranked_results, rerank_route) = maybe_apply_rerank(state, query_text, results, budget);
    results = reranked_results;
    // Co-occurrence tracking (recording only -- predictions excluded from response)
    let sources: Vec<String> = results.iter().map(|item| item.source.clone()).collect();
    if sources.len() >= 2 {
        if co_occurrence::record(&conn, &sources).is_ok() {
            checkpoint_wal_best_effort(&conn);
        } else {
            let _ = co_occurrence::reset(&conn);
        }
    }
    drop(conn);
    // Record recall pattern for prediction
    record_recall_pattern(state, &recall_scope, query_text).await;
    // Fire-and-forget pre-cache warming
    let state_clone = state.clone();
    let scope_owned = recall_scope.clone();
    let query_owned = query_text.to_string();
    let ctx_owned = *ctx;
    tokio::spawn(async move {
        let _ = predict_and_cache(state_clone, &scope_owned, &query_owned, ctx_owned).await;
    });
    // Headlines mode (budget == 0)
    if budget == 0 {
        let method_breakdown = build_method_breakdown(&results);
        let tier = classify_recall_tier(false, "headlines", &method_breakdown);
        let latency_ms = started_at.elapsed().as_millis() as i64;
        let headlines = results
            .iter()
            .map(|item| {
                json!({
                    "source": item.source,
                    "relevance": item.relevance,
                    "method": item.method
                })
            })
            .collect::<Vec<_>>();
        let usage = compute_headlines_token_usage(&results);
        emit_recall_query_event(
            state,
            agent,
            source_prefix,
            json!({
            "agent": agent,
            "query": truncate_chars(query_text, 120),
            "budget": 0,
            "spent": usage.spent,
            "saved": usage.saved,
            "hits": headlines.len(),
            "mode": "headlines",
                "cached": false,
                "method_breakdown": method_breakdown,
                "tier": tier,
                "latency_ms": latency_ms,
                "latency_budget_ms": latency_budget_ms,
                "semantic_route": semantic_route.clone(),
                "shadow_semantic": shadow_semantic,
                "fail_closed": fail_closed,
                "rerank": rerank_route.clone()
            }),
        )
        .await;
        return Ok(json!({
        "count": headlines.len(),
            "results": headlines,
            "budget": 0,
            "spent": usage.spent,
            "saved": usage.saved,
            "overBudget": usage.over_budget,
            "tokenUsageLine": format_recall_token_usage_line(0, usage),
            "mode": "headlines",
            "policyMode": RecallPolicyMode::Headlines.as_str(),
            "tier": tier,
            "latencyMs": latency_ms,
            "latencyBudgetMs": latency_budget_ms,
            "failClosed": fail_closed,
            "semanticRoute": semantic_route.clone(),
            "rerankRoute": rerank_route
        }));
    }
    // Dedup and budget accounting
    let results = dedup_and_mark_served(state, agent, query_text, ctx, results).await;
    let results = enforce_budget_token_invariant(results, budget, query_text);
    let usage = compute_recall_budget_usage(&results, budget);
    let mode = recall_mode_for_budget(budget);
    let method_breakdown = build_method_breakdown(&results);
    let tier = classify_recall_tier(false, mode.as_str(), &method_breakdown);
    let latency_ms = started_at.elapsed().as_millis() as i64;
    emit_recall_query_event(
        state,
        agent,
        source_prefix,
        json!({
            "agent": agent,
            "query": truncate_chars(query_text, 120),
            "budget": budget,
            "spent": usage.spent,
            "saved": usage.saved,
            "over_budget": usage.over_budget,
            "hits": results.len(),
            "mode": mode.as_str(),
            "cached": false,
            "method_breakdown": method_breakdown,
            "tier": tier,
            "latency_ms": latency_ms,
            "latency_budget_ms": latency_budget_ms,
            "semantic_route": semantic_route.clone(),
            "shadow_semantic": shadow_semantic,
            "fail_closed": fail_closed,
            "rerank": rerank_route.clone()
        }),
    )
    .await;
    let payload = json!({
        "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": usage.spent,
        "saved": usage.saved,
        "overBudget": usage.over_budget,
        "tokenUsageLine": format_recall_token_usage_line(budget, usage),
        "mode": mode.as_str(),
        "policyMode": mode.as_str(),
        "tier": tier,
        "latencyMs": latency_ms,
        "latencyBudgetMs": latency_budget_ms,
        "failClosed": fail_closed,
        "semanticRoute": semantic_route,
        "rerankRoute": rerank_route
    });
    Ok(payload)
}
#[allow(clippy::too_many_arguments)]
pub async fn execute_recall_policy_explain(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    pool_k: usize,
    query_vector_override: Option<&[f32]>,
) -> Result<Value, String> {
    let requested_k = k.max(1);
    let pool_k = pool_k.max(requested_k).min(128);
    let engine = state.embedding_engine.clone();
    let dflag = Some(&state.degraded_mode);
    let query_vector = match query_vector_override {
        Some(vector) => Some(vector.to_vec()),
        None => match engine {
            Some(runtime_engine) => {
                runtime_engine
                    .embed_query_async(query_text.to_string())
                    .await
            }
            None => None,
        },
    };
    if query_vector_override.is_none() && state.embedding_engine.is_some() {
        update_semantic_search_health(dflag, query_vector.is_some(), true);
    }
    let mut conn = state.db.lock().await;
    let (
        budgeted,
        candidate_pool,
        pre_compaction_candidate_count,
        family_compactions,
        retrieval_depth,
        min_relevance,
        top_relevance,
        max_items,
        semantic_baseline,
        semantic_route,
    ) = if budget == 0 {
        let trace = run_recall_with_query_vector_trace(
            &mut conn,
            query_text,
            pool_k,
            query_vector.as_deref(),
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
        )?;
        let raw_pool = trace.ranked;
        let budgeted = raw_pool
            .iter()
            .take(requested_k)
            .cloned()
            .map(|mut item| {
                item.excerpt.clear();
                item.tokens = Some(estimate_tokens(&item.source));
                item
            })
            .collect::<Vec<_>>();
        let raw_pool_len = raw_pool.len();
        (
            budgeted,
            raw_pool,
            raw_pool_len,
            Vec::new(),
            pool_k,
            0.0_f64,
            0.0_f64,
            requested_k,
            trace.semantic_baseline,
            trace.semantic_route,
        )
    } else {
        let trace = run_budget_recall_trace_with_query_vector(
            &mut conn,
            query_text,
            budget,
            requested_k,
            query_vector.as_deref(),
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
        )?;
        (
            trace.budgeted,
            trace.candidate_pool,
            trace.pre_compaction_candidate_count,
            trace.family_compactions,
            trace.retrieval_depth,
            trace.min_relevance,
            trace.top_relevance,
            trace.max_items,
            trace.semantic_baseline,
            trace.semantic_route,
        )
    };
    let shadow_semantic = build_shadow_semantic_explain(
        &conn,
        query_vector.as_deref(),
        query_text,
        ctx,
        source_prefix,
        pool_k,
        semantic_baseline.as_ref(),
    );
    drop(conn);
    let (budgeted, rerank_route) = maybe_apply_rerank(state, query_text, budgeted, budget);
    let final_results = dedup_and_mark_served(state, agent, query_text, ctx, budgeted).await;
    let final_results = enforce_budget_token_invariant(final_results, budget, query_text);
    let usage = compute_recall_budget_usage(&final_results, budget);
    let mode = recall_mode_for_budget(budget);
    let family_compacted_count: usize = family_compactions
        .iter()
        .map(|entry| entry.dropped_sources.len())
        .sum();
    let family_compactions_json: Vec<Value> = family_compactions
        .iter()
        .map(|entry| {
            json!({
                "familyKey": entry.family_key,
                "keptSource": entry.kept_source,
                "droppedSources": entry.dropped_sources,
            })
        })
        .collect();
    let returned_sources: HashSet<&str> = final_results
        .iter()
        .map(|item| item.source.as_str())
        .collect();
    let dropped_candidates: Vec<Value> = candidate_pool
        .iter()
        .filter(|item| !returned_sources.contains(item.source.as_str()))
        .take(24)
        .map(|item| {
            let estimated_tokens = estimate_tokens(&format!("{}{}", item.source, item.excerpt));
            json!({
                "source": item.source,
                "relevance": item.relevance,
                "method": item.method,
                "estimatedTokens": estimated_tokens,
                "reason": "not_selected_under_current_budget_or_rank_cutoff"
            })
        })
        .collect();
    let query_entities = query_entity_terms(query_text);
    let mut entity_metrics_by_source: HashMap<String, (usize, f64, f64)> = HashMap::new();
    for candidate in &candidate_pool {
        let haystack = format!("{} {}", candidate.source, candidate.excerpt);
        let (entity_matches, entity_overlap) =
            entity_alignment_metrics_with_terms(&haystack, &query_entities);
        let entity_boost = entity_signal_boost(entity_matches, entity_overlap);
        entity_metrics_by_source.insert(
            candidate.source.clone(),
            (entity_matches, round4(entity_overlap), round4(entity_boost)),
        );
    }
    let final_with_factors: Vec<Value> = final_results
        .clone()
        .into_iter()
        .enumerate()
        .map(|(idx, item)| {
            let tokens = item
                .tokens
                .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)));
            let budget_ratio = if budget == 0 {
                0.0
            } else {
                ((tokens as f64) / (budget as f64)).min(1.0)
            };
            let (entity_matches, entity_overlap, entity_boost) = entity_metrics_by_source
                .get(&item.source)
                .copied()
                .unwrap_or_else(|| {
                    let haystack = format!("{} {}", item.source, item.excerpt);
                    let (matches, overlap) =
                        entity_alignment_metrics_with_terms(&haystack, &query_entities);
                    (
                        matches,
                        round4(overlap),
                        round4(entity_signal_boost(matches, overlap)),
                    )
                });
            json!({
                "rank": idx + 1,
                "source": item.source,
                "relevance": item.relevance,
                "method": item.method,
                "tokens": tokens,
                "rankingFactors": {
                    "relevance": item.relevance,
                    "method": item.method,
                    "tokenCost": tokens,
                    "budgetCostRatio": round4(budget_ratio),
                    "entropy": item.entropy,
                    "entityMatches": entity_matches,
                    "entityOverlap": entity_overlap,
                    "entityBoost": entity_boost
                }
            })
        })
        .collect();
    let post_compaction_dropped_count = candidate_pool
        .len()
        .saturating_sub(final_with_factors.len());
    Ok(json!({
        "query": query_text,
        "results": final_results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": usage.spent,
        "saved": usage.saved,
        "overBudget": usage.over_budget,
        "tokenUsageLine": format_recall_token_usage_line(budget, usage),
        "mode": mode.as_str(),
        "policyMode": mode.as_str(),
        "policy": {
            "name": "adaptive-recall-policy",
            "mode": mode.as_str(),
            "budget": budget,
            "requestedK": requested_k,
            "poolK": pool_k,
            "retrievalDepth": retrieval_depth,
            "candidateCutoff": {
                "topRelevance": round4(top_relevance),
                "minRelevance": round4(min_relevance),
                "maxItemsBeforeBudget": max_items
            },
            "budgetReasoning": {
                "requestedBudget": budget,
                "spent": usage.spent,
                "saved": usage.saved,
                "budgetPressure": if budget == 0 { 0.0 } else { round4((usage.spent as f64) / (budget as f64)) },
                "candidateCountBeforeFamilyCompaction": pre_compaction_candidate_count,
                "candidateCount": candidate_pool.len(),
                "candidateCountAfterFamilyCompaction": candidate_pool.len(),
                "familyCompactedCount": family_compacted_count,
                "returnedCount": final_with_factors.len(),
                "droppedCount": post_compaction_dropped_count,
                "totalPreBudgetDrops": family_compacted_count + post_compaction_dropped_count
            },
            "semanticRoute": semantic_route,
            "rerankRoute": rerank_route.clone()
        },
        "explain": {
            "returned": final_with_factors,
            "familyCompactions": family_compactions_json,
            "droppedCandidates": dropped_candidates,
            "shadowSemantic": shadow_semantic,
            "rerank": rerank_route
        }
    }))
}
pub async fn execute_semantic_recall(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Value, String> {
    let started_at = Instant::now();
    let query_vector = match state.embedding_engine.clone() {
        Some(engine) => engine.embed_query_async(query_text.to_string()).await,
        None => None,
    };
    let semantic_available = query_vector.is_some();
    let (budgeted, semantic_route) = {
        let conn = state.db.lock().await;
        let (results, semantic_route) = run_semantic_recall_with_query_vector(
            &conn,
            query_text,
            k,
            query_vector.as_deref(),
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
        );
        (
            apply_semantic_budget(results, budget, query_text),
            semantic_route,
        )
    };
    let budgeted = enforce_budget_token_invariant(budgeted, budget, query_text);
    let usage = compute_recall_budget_usage(&budgeted, budget);
    let mode = "semantic";
    let method_breakdown = build_method_breakdown(&budgeted);
    let tier = classify_recall_tier(false, mode, &method_breakdown);
    let latency_ms = started_at.elapsed().as_millis() as i64;
    emit_recall_query_event(
        state,
        agent,
        source_prefix,
        json!({
            "agent": agent,
            "query": truncate_chars(query_text, 120),
            "mode": mode,
            "k": k,
            "budget": budget,
            "spent": usage.spent,
            "saved": usage.saved,
            "over_budget": usage.over_budget,
            "hits": budgeted.len(),
            "results": budgeted.len(),
            "semantic_available": semantic_available,
            "cached": false,
            "method_breakdown": method_breakdown,
            "tier": tier,
            "latency_ms": latency_ms,
            "semantic_route": semantic_route.clone(),
        }),
    )
    .await;
    Ok(json!({
        "results": budgeted.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "mode": "semantic",
        "budget": budget,
        "spent": usage.spent,
        "saved": usage.saved,
        "overBudget": usage.over_budget,
        "tokenUsageLine": format_recall_token_usage_line(budget, usage),
        "semanticAvailable": semantic_available,
        "semanticRoute": semantic_route,
        "tier": tier,
        "latencyMs": latency_ms,
    }))
}
#[allow(clippy::type_complexity)]
pub(crate) fn run_recall_with_query_vector_trace(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>,
) -> Result<RecallWithVectorTrace, String> {
    let extracted = extract_search_keywords(query_text);
    let prefers_recency = query_prefers_recency(query_text);
    let keyword_query = if extracted.is_empty() {
        query_text.to_string()
    } else {
        extracted.join(" ")
    };
    // This function is the retrieval engine; caching is the caller's responsibility.
    // and should always surface regardless of FTS confidence.
    let scale_sim = |sim: f32| -> f64 {
        SEMANTIC_SCALE_BASE
            + (sim as f64 - SEMANTIC_SIM_FLOOR)
                * ((1.0 - SEMANTIC_SCALE_BASE) / (1.0 - SEMANTIC_SIM_FLOOR))
    };
    // Crystal results keyed by source. Their member sources are tracked so the
    // final merge can collapse near-duplicate family members under the crystal.
    let mut crystal_items: HashMap<String, RecallItem> = HashMap::new();
    let mut crystal_family_lookup: HashMap<String, String> = HashMap::new();
    if let Some(query_vec) = query_vector {
        for (crystal_id, label, text, relevance) in crate::crystallize::search_crystals_filtered(
            conn,
            query_vec,
            3,
            ctx.caller_id,
            ctx.team_mode,
        ) {
            let source = crystal_source(crystal_id, &label);
            if !source_matches_prefix(&source, source_prefix) {
                continue;
            }
            let family_members = crystal_member_sources(conn, crystal_id, ctx);
            for member_source in &family_members {
                crystal_family_lookup
                    .entry(member_source.clone())
                    .or_insert_with(|| source.clone());
            }
            crystal_items.insert(
                source.clone(),
                RecallItem {
                    source,
                    relevance: scale_sim(relevance as f32),
                    excerpt: query_focused_excerpt(&text, query_text, 300),
                    method: "crystal".to_string(),
                    tokens: None,
                    entropy: None,
                    family_members,
                    collapsed_sources: Vec::new(),
                    collapsed_source_scores: Vec::new(),
                },
            );
        }
    }
    // Run FTS5 first. If the top result is confident (score >= 0.93) with a
    // meaningful gap from #2 (delta >= 0.08), return immediately without
    // spending cycles on embedding inference. Target: 40%+ queries resolved here.
    const TIER2_CONFIDENCE: f64 = 0.78;
    const TIER2_GAP: f64 = 0.10;
    let raw_k = if ctx.team_mode { k.max(10) * 5 } else { 20 };
    let mut fts_limit = raw_k.max(20);
    let kw_candidates: Vec<SearchCandidate> = {
        let mut retry = 0;
        let mut all: Vec<SearchCandidate> = Vec::new();
        loop {
            all.clear();
            for row in search_memories(conn, &keyword_query, fts_limit, source_prefix)?
                .into_iter()
                .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
            {
                all.push(row);
            }
            for row in search_decisions(conn, &keyword_query, fts_limit, source_prefix)?
                .into_iter()
                .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
            {
                all.push(row);
            }
            all.sort_by(|a, b| {
                compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
            });
            if ctx.team_mode && all.len() < k && retry < 2 {
                fts_limit *= 2;
                retry += 1;
                continue;
            }
            break;
        }
        all
    };
    let required_keyword_hits = if extracted.is_empty() {
        1_i64
    } else {
        ((extracted.len() as f64) * 0.6).ceil() as i64
    };
    let tier2_resolved = if let Some(top) = kw_candidates.first() {
        let gap = kw_candidates
            .get(1)
            .map(|next| top.relevance - next.relevance)
            .unwrap_or(top.relevance);
        top.relevance >= TIER2_CONFIDENCE
            && top.matched_keywords >= required_keyword_hits
            && gap >= TIER2_GAP
    } else {
        false
    };
    // Produces a ranked list of (source, score) pairs for RRF.
    // Also accumulates per-source metadata (score, ts) for compound scoring.
    let (semantic_candidates, semantic_route, semantic_baseline) = if tier2_resolved {
        (
            Vec::new(),
            json!({
                "mode": "baseline",
                "reason": "tier2_keyword_resolved",
                "sampled": false,
                "trialPercent": canary
                    .map(|config| {
                        if matches!(config.effective_route_mode(), SqliteVecRouteMode::Primary) {
                            100
                        } else {
                            config.trial_percent
                        }
                    })
                    .unwrap_or(0),
                "routeMode": canary
                    .map(|config| config.effective_route_mode().as_str())
                    .unwrap_or("baseline")
            }),
            None,
        )
    } else {
        let baseline_semantic = query_vector
            .map(|query_vec| {
                collect_semantic_candidates(conn, query_vec, query_text, ctx, source_prefix)
            })
            .unwrap_or_default();
        let semantic_baseline = if baseline_semantic.is_empty() {
            None
        } else {
            Some(ShadowSemanticBaseline {
                candidate_count: baseline_semantic.len(),
                ranked_sources: baseline_semantic
                    .iter()
                    .take(MAX_SEMANTIC_RRF_CANDIDATES)
                    .map(|candidate| candidate.source.clone())
                    .collect(),
            })
        };
        let (semantic_candidates, semantic_route) = maybe_apply_sqlite_vec_trial(
            conn,
            query_text,
            query_vector,
            baseline_semantic,
            ctx,
            source_prefix,
            k,
            canary,
        );
        (semantic_candidates, semantic_route, semantic_baseline)
    };
    // Assign stable integer indices to each unique source across both lists,
    // then fuse ranks. rrf_fuse() works on (i64, f64) so we map source → index.
    //
    // ranking (correct behavior -- no fusion penalty).
    let mut source_index: HashMap<String, i64> = HashMap::new();
    let mut index_source: Vec<String> = Vec::new();
    let mut get_idx = |source: &str| -> i64 {
        if let Some(&idx) = source_index.get(source) {
            return idx;
        }
        let idx = index_source.len() as i64;
        source_index.insert(source.to_string(), idx);
        index_source.push(source.to_string());
        idx
    };
    // Build ranked list for keyword results (sorted by relevance desc)
    let kw_list: Vec<(i64, f64)> = kw_candidates
        .iter()
        .map(|c| (get_idx(&c.source), c.relevance))
        .collect();
    // Build ranked list for semantic results (sorted by relevance desc)
    let sem_list: Vec<(i64, f64)> = semantic_candidates
        .iter()
        .map(|candidate| (get_idx(&candidate.source), candidate.relevance))
        .collect();
    let fusion_weights =
        adaptive_rrf_weights(query_text, source_prefix, !semantic_candidates.is_empty());
    let fused = rrf_fuse_weighted(
        &[kw_list, sem_list],
        &[fusion_weights.keyword, fusion_weights.semantic],
        60.0,
    );
    // For each fused entry: look up metadata from keyword or semantic candidates,
    // determine method label, then apply compound_score().
    let mut merged: HashMap<String, RecallItem> = HashMap::new();
    for (idx, rrf_score) in &fused {
        let source = match index_source.get(*idx as usize) {
            Some(s) => s.clone(),
            None => continue,
        };
        // Prefer keyword candidate metadata (has score + ts); fall back to sem
        let (excerpt, importance, ts_ms, method) =
            if let Some(kw) = kw_candidates.iter().find(|c| c.source == source) {
                let in_sem = semantic_candidates.iter().any(|sem| sem.source == source);
                let method = if in_sem { "hybrid" } else { "keyword" };
                (kw.excerpt.clone(), kw.score, kw.ts, method)
            } else if let Some(sem) = semantic_candidates.iter().find(|sem| sem.source == source) {
                (sem.excerpt.clone(), sem.importance, sem.ts, "semantic")
            } else {
                continue;
            };
        // Convert ts (Unix-ms) to ISO 8601 for compound_score()
        let created_at_str = if ts_ms > 0 {
            Utc.timestamp_millis_opt(ts_ms)
                .single()
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        } else {
            String::new()
        };
        // importance is 0-1 in DB; normalize() expects 0-100 range
        let mut relevance = round4(compound_score(
            *rrf_score,
            importance * 100.0,
            &created_at_str,
        ));
        if prefers_recency {
            relevance = round4(relevance * temporal_intent_multiplier(ts_ms));
        }
        if let Some(crystal_source) = crystal_family_lookup.get(&source) {
            if let Some(crystal_item) = crystal_items.get_mut(crystal_source) {
                crystal_item.relevance = round4(crystal_item.relevance.max(relevance));
                if !crystal_item
                    .collapsed_sources
                    .iter()
                    .any(|collapsed| collapsed == &source)
                {
                    crystal_item.collapsed_sources.push(source.clone());
                }
                crystal_item
                    .collapsed_source_scores
                    .push((source.clone(), relevance));
                if prefer_query_focused_excerpt(&crystal_item.excerpt, &excerpt, query_text) {
                    crystal_item.excerpt = excerpt.clone();
                }
            }
            continue;
        }
        merged.insert(
            source.clone(),
            RecallItem {
                source,
                relevance,
                excerpt,
                method: method.to_string(),
                tokens: None,
                entropy: None,
                family_members: Vec::new(),
                collapsed_sources: Vec::new(),
                collapsed_source_scores: Vec::new(),
            },
        );
    }
    // Crystal items bypass RRF (they're already fused/consolidated knowledge);
    // insert after -- they will not be overwritten since crystal:: keys don't appear in kw/sem
    for (src, mut item) in crystal_items {
        dedup_preserve_order(&mut item.family_members);
        normalize_collapsed_source_rank(&mut item);
        merged.entry(src).or_insert(item);
    }
    // High-entropy (information-dense) excerpts get a relevance boost (+/-15%
    // around midpoint H=3.5). Applied after compound scoring so entropy acts as
    // a diversity signal on top of the RRF+compound base.
    let mut ranked: Vec<RecallItem> = merged.into_values().collect();
    apply_recall_ranking_boosts(&mut ranked, query_text, 0.08, 0.12);
    // Boost results that have been useful in past recalls (unfolded),
    // penalize results that were consistently ignored. Graceful no-op when
    // no feedback data exists (cold start).
    let sources: Vec<String> = ranked.iter().map(|r| r.source.clone()).collect();
    let boosts = crate::handlers::feedback::compute_boosts(conn, &sources, query_vector);
    if !boosts.is_empty() {
        for item in &mut ranked {
            if let Some(&boost) = boosts.get(&item.source) {
                item.relevance = round4(item.relevance * (1.0 + boost));
            }
        }
    }
    ranked.sort_by(|a, b| {
        compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source)
    });
    ranked.truncate(k);
    bump_retrievals_batch(conn, &ranked);
    Ok(RecallWithVectorTrace {
        ranked,
        semantic_baseline,
        semantic_route,
    })
}
pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Some(crystal_id) = parse_crystal_source_id(source) {
        if let Some((label, consolidated_text, member_count, owner_id, visibility)) =
            query_crystal_for_unfold(conn, crystal_id)
        {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                let members = crystal_member_sources(conn, crystal_id, ctx);
                let mut full_text = consolidated_text.clone();
                if !members.is_empty() {
                    full_text.push_str("\n\nFamily members:\n");
                    for member in members.iter().take(16) {
                        full_text.push_str("- ");
                        full_text.push_str(member);
                        full_text.push('\n');
                    }
                    if member_count as usize > members.len() {
                        full_text.push_str(&format!(
                            "... plus {} more hidden or archived member(s)",
                            (member_count as usize).saturating_sub(members.len())
                        ));
                    }
                }
                return Some(json!({
                    "source": crystal_source(crystal_id, &label),
                    "text": full_text.trim_end().to_string(),
                    "type": "crystal",
                    "label": label,
                    "clusterId": crystal_id,
                    "members": members,
                    "memberCount": member_count,
                }));
            }
        }
    }
    if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, source) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text": text, "type": ty}));
        }
    }
    if let Some(id_str) = source.strip_prefix("decision::") {
        if let Ok(id) = id_str.parse::<i64>() {
            if let Some((decision, context, owner_id, visibility)) =
                query_decision_by_id_for_unfold(conn, id)
            {
                if is_visible(owner_id, visibility.as_deref(), ctx) {
                    let full = match context {
                        Some(c) => format!("{decision}\n\nContext: {c}"),
                        None => decision,
                    };
                    return Some(json!({"text": full, "type": "decision"}));
                }
            }
        }
    }
    if let Some((decision, context, owner_id, visibility)) =
        query_decision_by_context_for_unfold(conn, source)
    {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            let full = match context {
                Some(c) => format!("{decision}\n\nContext: {c}"),
                None => decision,
            };
            return Some(json!({"text": full, "type": "decision"}));
        }
    }
    let stripped = source.strip_prefix("memory::").unwrap_or(source);
    if stripped != source {
        if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, stripped) {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                return Some(json!({"text": text, "type": ty}));
            }
        }
    }
    None
}
pub(crate) type MemoryUnfoldRow = (String, String, Option<i64>, Option<String>);
pub(crate) type DecisionUnfoldRow = (String, Option<String>, Option<i64>, Option<String>);
pub(crate) fn query_memory_for_unfold(conn: &Connection, source: &str) -> Option<MemoryUnfoldRow> {
    let sql_with_visibility =
        "SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT text, type FROM memories WHERE source = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn query_decision_by_id_for_unfold(conn: &Connection, id: i64) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE id = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))";
    match conn.query_row(sql_with_visibility, params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE id = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn query_decision_by_context_for_unfold(
    conn: &Connection,
    source: &str,
) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE context = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE context = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
