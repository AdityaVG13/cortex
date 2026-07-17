use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::{estimate_tokens, now_iso, parse_timestamp_ms, truncate_chars};
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode};
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
        exactish: has_exact_markers || token_count <= 3 || char_count <= 24 || source_prefix.is_some(),
        naturalish: token_count >= 8 || char_count >= 56 || trimmed.ends_with('?'),
    }
}
pub(crate) const MAX_RECALL_HISTORY: usize = 50;
pub(crate) const PRECACHE_TTL_MS: i64 = 5 * 60 * 1000;
pub(crate) const RECALL_PREDICTIVE_PRECACHE_ENV: &str = "CORTEX_RECALL_PREDICTIVE_PRECACHE";
pub(crate) const SEMANTIC_SIM_FLOOR: f64 = 0.3;
pub(crate) const SEMANTIC_SCALE_BASE: f64 = 0.55;
pub(crate) const MAX_SEMANTIC_RRF_CANDIDATES: usize = 120;
pub(crate) const MAX_SEMANTIC_SQL_ROWS_PER_KIND: usize = MAX_SEMANTIC_RRF_CANDIDATES * 24;
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
        memories_text: parse_bm25_weight(resolve_env("CORTEX_BM25_MEM_TEXT_WEIGHT"), MEMORIES_BM25_TEXT_WEIGHT),
        memories_source: parse_bm25_weight(resolve_env("CORTEX_BM25_MEM_SOURCE_WEIGHT"), MEMORIES_BM25_SOURCE_WEIGHT),
        memories_tags: parse_bm25_weight(resolve_env("CORTEX_BM25_MEM_TAGS_WEIGHT"), MEMORIES_BM25_TAGS_WEIGHT),
        decisions_text: parse_bm25_weight(resolve_env("CORTEX_BM25_DECISION_WEIGHT"), DECISIONS_BM25_DECISION_WEIGHT),
        decisions_context: parse_bm25_weight(resolve_env("CORTEX_BM25_CONTEXT_WEIGHT"), DECISIONS_BM25_CONTEXT_WEIGHT),
    }
}
pub(crate) fn bm25_weights() -> &'static Bm25Weights {
    BM25_WEIGHTS.get_or_init(|| bm25_weights_from_resolver(|name| std::env::var(name).ok()))
}
pub(crate) fn recall_predictive_precache_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(RECALL_PREDICTIVE_PRECACHE_ENV)
            .ok()
            .is_some_and(|raw| matches!(raw.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
    })
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
        self.ranked_sources.iter().take(top_k.clamp(1, MAX_SEMANTIC_RRF_CANDIDATES)).cloned().collect()
    }
}
pub(crate) struct RecallWithVectorTrace {
    pub(crate) ranked: Vec<RecallItem>,
    pub(crate) semantic_baseline: Option<ShadowSemanticBaseline>,
    pub(crate) semantic_route: Value,
}
pub(crate) type CrystalMemberSourceRow = (Option<String>, Option<i64>, Option<String>);
#[derive(Clone, Copy)]
pub struct RecallContext {
    pub caller_id: Option<i64>,
    pub team_mode: bool,
}
impl RecallContext {
    pub fn from_caller(caller_id: Option<i64>, state: &RuntimeState) -> Self {
        Self { caller_id, team_mode: state.team_mode }
    }
    #[allow(dead_code)]
    pub fn from_state(state: &RuntimeState) -> Self {
        Self { caller_id: state.default_owner_id, team_mode: state.team_mode }
    }
    #[allow(dead_code)]
    pub fn solo() -> Self {
        Self { caller_id: None, team_mode: false }
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
    let mut ranked: Vec<(String, f64, usize)> = best_scores.into_iter().map(|(source, (score, order))| (source, score, order)).collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.2.cmp(&b.2)));
    item.collapsed_source_scores = ranked.iter().map(|(source, score, _)| (source.clone(), *score)).collect();
    item.collapsed_sources = item.collapsed_source_scores.iter().map(|(source, _)| source.clone()).collect();
}
pub(crate) fn parse_crystal_source_id(source: &str) -> Option<i64> {
    let rest = source.strip_prefix("crystal::")?;
    let (id, _) = rest.split_once("::")?;
    id.parse::<i64>().ok()
}
pub(crate) fn crystal_member_sources(conn: &Connection, crystal_id: i64, ctx: &RecallContext) -> Vec<String> {
    let query_rows = |sql: &str, with_visibility: bool| -> Result<Vec<CrystalMemberSourceRow>, rusqlite::Error> {
        let mut stmt = conn.prepare(sql)?;
        let mapped = stmt.query_map(params![crystal_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                if with_visibility { row.get::<_, Option<i64>>(1)? } else { None },
                if with_visibility { row.get::<_, Option<String>>(2)? } else { None },
            ))
        })?;
        Ok(mapped.flatten().collect())
    };
    let sql_with_visibility="SELECT CASE
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
      AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) AND (m.valid_until IS NULL OR m.valid_until > datetime('now'))
     LEFT JOIN decisions d
       ON cm.target_type = 'decision'
      AND cm.target_id = d.id
      AND d.status = 'active'
      AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) AND (d.valid_until IS NULL OR d.valid_until > datetime('now'))
     WHERE cm.cluster_id = ?1
     ORDER BY cm.target_type, cm.target_id";
    let sql_legacy="SELECT CASE
                WHEN cm.target_type = 'memory' THEN COALESCE(m.source, 'memory::' || m.id)
                ELSE COALESCE(d.context, 'decision::' || d.id)
            END AS source
     FROM cluster_members cm
     LEFT JOIN memories m
       ON cm.target_type = 'memory'
      AND cm.target_id = m.id
      AND m.status = 'active'
      AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) AND (m.valid_from IS NULL OR m.valid_from <= datetime('now')) AND (m.valid_until IS NULL OR m.valid_until > datetime('now'))
     LEFT JOIN decisions d
       ON cm.target_type = 'decision'
      AND cm.target_id = d.id
      AND d.status = 'active'
      AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) AND (d.valid_from IS NULL OR d.valid_from <= datetime('now')) AND (d.valid_until IS NULL OR d.valid_until > datetime('now'))
     WHERE cm.cluster_id = ?1
     ORDER BY cm.target_type, cm.target_id";
    let rows = match query_rows(sql_with_visibility, true) {
        Ok(rows) => rows,
        Err(err) if is_missing_team_visibility_columns(&err) => match query_rows(sql_legacy, false) {
            Ok(rows) => rows,
            Err(_) => return Vec::new(),
        },
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
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, None, None)),
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn is_missing_team_visibility_columns(err: &rusqlite::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("no such column") && (normalized.contains("owner_id") || normalized.contains("visibility"))
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
        RecallPolicyMode::Fast => parse_env_usize("CORTEX_RECALL_FAST_BUDGET", DEFAULT_RECALL_BUDGET_FAST, 1, 2000),
        RecallPolicyMode::Balanced => parse_env_usize("CORTEX_RECALL_BALANCED_BUDGET", DEFAULT_RECALL_BUDGET_BALANCED, 1, 4000),
        RecallPolicyMode::Deep => parse_env_usize("CORTEX_RECALL_DEEP_BUDGET", DEFAULT_RECALL_BUDGET_DEEP, 1, 8000),
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
        RecallPolicyMode::Headlines => {
            parse_env_usize("CORTEX_RECALL_HEADLINES_MAX_LATENCY_MS", DEFAULT_RECALL_LATENCY_FAST_MS as usize, 0, 60_000) as u128
        }
        RecallPolicyMode::Fast => {
            parse_env_usize("CORTEX_RECALL_FAST_MAX_LATENCY_MS", DEFAULT_RECALL_LATENCY_FAST_MS as usize, 0, 60_000) as u128
        }
        RecallPolicyMode::Balanced => {
            parse_env_usize("CORTEX_RECALL_BALANCED_MAX_LATENCY_MS", DEFAULT_RECALL_LATENCY_BALANCED_MS as usize, 0, 60_000) as u128
        }
        RecallPolicyMode::Deep => {
            parse_env_usize("CORTEX_RECALL_DEEP_MAX_LATENCY_MS", DEFAULT_RECALL_LATENCY_DEEP_MS as usize, 0, 120_000) as u128
        }
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
            return Err("Invalid policy mode. Expected one of: headlines, fast, balanced, deep".to_string());
        }
    };
    Ok(Some(mode))
}
pub fn resolve_recall_budget_k(
    requested_mode: Option<RecallPolicyMode>, budget: Option<usize>, k: Option<usize>,
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
pub(crate) fn adaptive_default_budget_for_query(query_text: &str, resolved_k: usize, default_budget: usize) -> usize {
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
    query_text: &str, requested_mode: Option<RecallPolicyMode>, requested_budget: Option<usize>, resolved_budget: usize, resolved_k: usize,
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
            let (entity_matches, entity_overlap) = entity_alignment_metrics_with_terms(&haystack, &query_entities);
            let entity_boost = entity_signal_boost(entity_matches, entity_overlap);
            if entity_boost > 0.0 {
                item.relevance = round4(item.relevance * (1.0 + entity_boost));
            }
        }
        let alignment_boost = query_alignment_boost_with_profile(&item.source, &item.excerpt, &alignment_profile, query_focus_term_count);
        if alignment_boost > 0.0 {
            item.relevance = round4(item.relevance * (1.0 + alignment_boost));
        }
    }
}
pub(crate) fn normalize_text(input: &str) -> String {
    input
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch.is_ascii_whitespace() { ch.to_ascii_lowercase() } else { ' ' })
        .collect()
}
pub(crate) fn extract_keywords(text: &str) -> Vec<String> {
    let stop_words: HashSet<&'static str> = [
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would",
        "could", "should", "may", "might", "shall", "can", "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into",
        "about", "that", "this", "it", "its", "not", "but", "and", "or", "if", "then", "so", "what", "which", "who", "how", "when",
        "where", "why", "all", "each", "every", "both", "few", "more", "most", "some", "any", "no", "my", "your", "his", "her", "our",
        "their", "i", "me",
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
    normalize_text(text).split_whitespace().filter(|word| word.len() > 1).map(str::to_string).collect()
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
        aliases.extend(["attend", "attended", "exchange", "semester"].into_iter().map(str::to_string));
    }
    if lower.contains("coupon") && lower.contains("creamer") {
        aliases.extend(["redeem", "redeemed", "store", "grocery"].into_iter().map(str::to_string));
    }
    if lower.contains("birthday") && (lower.contains("gift") || lower.contains("present")) {
        aliases.extend(["buy", "bought", "item", "present"].into_iter().map(str::to_string));
    }
    aliases
}
pub(crate) fn build_search_term_groups(text: &str) -> Vec<Vec<String>> {
    let mut base = extract_search_keywords(text);
    let profile = query_shape_profile(text, None);
    if profile.naturalish && base.len() >= 6 {
        let filtered = base.iter().filter(|token| !is_low_signal_query_token(token.as_str())).cloned().collect::<Vec<_>>();
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
        .filter(|group| group.iter().any(|term| haystacks.iter().any(|haystack| haystack.contains(term))))
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
            let alternates = group.iter().map(|t| format!("\"{}\"", t.replace('"', "\"\""))).collect::<Vec<_>>().join(" OR ");
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
    for token in extract_search_keywords(source).into_iter().chain(extract_search_keywords(excerpt)) {
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
    signature_terms: &HashSet<String>, query_terms: &HashSet<String>, covered_terms: &HashSet<String>,
) -> usize {
    query_terms
        .iter()
        .filter(|term| signature_terms.contains(*term) && !covered_terms.contains(*term))
        .count()
}
pub(crate) fn should_skip_redundant_budget_candidate(
    signature_terms: &HashSet<String>, selected_signatures: &[HashSet<String>], query_terms: &HashSet<String>,
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
    signature_terms: &HashSet<String>, query_terms: &HashSet<String>, covered_terms: &mut HashSet<String>,
) {
    for term in query_terms {
        if signature_terms.contains(term) {
            covered_terms.insert(term.clone());
        }
    }
}
pub(crate) fn should_early_stop_budget_selection(
    token_budget: usize, spent_tokens: usize, selected_count: usize, query_terms: &HashSet<String>, covered_terms: &HashSet<String>,
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
pub(crate) fn query_focused_excerpt_with_terms(text: &str, sorted_focus_terms: &[String], max_chars: usize) -> String {
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
    let mut excerpt = text.chars().skip(start_char).take(end_char - start_char).collect::<String>();
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
pub(crate) fn compare_relevance_desc_source_asc(a_relevance: f64, a_source: &str, b_relevance: f64, b_source: &str) -> std::cmp::Ordering {
    let a = if a_relevance.is_finite() { a_relevance } else { f64::NEG_INFINITY };
    let b = if b_relevance.is_finite() { b_relevance } else { f64::NEG_INFINITY };
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
        Self { lower_query, terms, term_count }
    }
    pub(crate) fn alignment_score(&self, text: &str) -> (usize, usize) {
        if text.is_empty() || self.lower_query.is_empty() {
            return (0, 0);
        }
        let lower_text = text.to_ascii_lowercase();
        let exact_phrase = usize::from(lower_text.contains(&self.lower_query));
        let keyword_hits = self.terms.iter().filter(|term| lower_text.contains(term.as_str())).count();
        (exact_phrase, keyword_hits)
    }
}
pub(crate) fn prefer_query_focused_excerpt_with_profile(current: &str, candidate: &str, profile: &QueryAlignmentProfile) -> bool {
    let current_score = profile.alignment_score(current);
    let candidate_score = profile.alignment_score(candidate);
    candidate_score > current_score || (candidate_score == current_score && candidate.len() < current.len())
}
#[allow(dead_code)]
pub(crate) fn prefer_query_focused_excerpt(current: &str, candidate: &str, query_text: &str) -> bool {
    let profile = QueryAlignmentProfile::from_query(query_text);
    prefer_query_focused_excerpt_with_profile(current, candidate, &profile)
}
pub(crate) fn query_prefers_recency(query_text: &str) -> bool {
    let lower = query_text.to_ascii_lowercase();
    ["latest", "most recent", "recent", "newest", "current", "today", "now", "up to date", "up-to-date"]
        .iter()
        .any(|needle| lower.contains(needle))
}
pub(crate) fn temporal_intent_multiplier(ts_ms: i64) -> f64 {
    if ts_ms <= 0 {
        return 1.0 - (TEMPORAL_INTENT_MULTIPLIER_RANGE * 0.25);
    }
    let age_days = ((Utc::now().timestamp_millis() - ts_ms).max(0) as f64) / (1000.0 * 60.0 * 60.0 * 24.0);
    let freshness = (1.0 / (1.0 + age_days / 14.0)).clamp(0.0, 1.0);
    1.0 + ((freshness - 0.5) * TEMPORAL_INTENT_MULTIPLIER_RANGE)
}
pub(crate) fn query_alignment_boost_with_profile(
    source: &str, excerpt: &str, profile: &QueryAlignmentProfile, query_focus_term_count: usize,
) -> f64 {
    if profile.lower_query.is_empty() {
        return 0.0;
    }
    let lower_source = source.to_ascii_lowercase();
    let lower_excerpt = excerpt.to_ascii_lowercase();
    let exact_phrase = usize::from(lower_source.contains(&profile.lower_query) || lower_excerpt.contains(&profile.lower_query));
    let keyword_hits = profile
        .terms
        .iter()
        .filter(|term| lower_source.contains(term.as_str()) || lower_excerpt.contains(term.as_str()))
        .count();
    if exact_phrase == 0 && keyword_hits == 0 {
        return 0.0;
    }
    let term_count = query_focus_term_count.max(1) as f64;
    let coverage = (keyword_hits as f64 / term_count).clamp(0.0, 1.0);
    let exact_bonus = if exact_phrase > 0 { ALIGNMENT_EXACT_BONUS_MAX } else { 0.0 };
    let coverage_bonus = (coverage * ALIGNMENT_COVERAGE_BONUS_MAX).min(ALIGNMENT_COVERAGE_BONUS_MAX);
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
    for raw in text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':'))) {
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
        let has_symbol = token.chars().any(|c| matches!(c, '_' | '-' | '.' | '/' | ':'));
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
pub(crate) fn entity_alignment_metrics_with_terms(haystack: &str, query_entities: &HashSet<String>) -> (usize, f64) {
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
    let matches = query_entities.iter().filter(|term| haystack_terms.contains(*term)).count();
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
pub(crate) fn adaptive_rrf_weights(query_text: &str, source_prefix: Option<&str>, semantic_available: bool) -> FusionWeights {
    if !semantic_available {
        return FusionWeights { keyword: 1.0, semantic: 0.0 };
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
    FusionWeights { keyword: keyword.clamp(0.35, 1.75), semantic: semantic.clamp(0.35, 1.75) }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FallbackRankingWeights {
    pub(crate) keyword: f64,
    pub(crate) score: f64,
    pub(crate) recency: f64,
    pub(crate) retrieval: f64,
}
pub(crate) fn adaptive_fallback_ranking_weights(query_text: &str, term_group_count: usize) -> FallbackRankingWeights {
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
    query_text: &str, term_group_count: usize, matched: i64, effective_score: f64, recency_days: i64, retrievals: Option<i64>,
) -> f64 {
    let keyword_weight = if term_group_count == 0 { 0.0 } else { matched as f64 / term_group_count as f64 };
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
        Err(_) => f64::MAX,
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
    let recency = (-days / 30.0).exp();
    let importance_normalized = normalize(importance);
    rrf * 0.6 + importance_normalized * 0.2 + recency * 0.2
}
fn sort_search_candidates(ranked: &mut [SearchCandidate], by_keywords: bool) {
    ranked.sort_by(|a, b| {
        let ord = b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal);
        let ord = if by_keywords { ord.then(b.matched_keywords.cmp(&a.matched_keywords)) } else { ord };
        ord.then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.ts.cmp(&a.ts))
            .then(b.alignment.cmp(&a.alignment))
            .then_with(|| a.source.cmp(&b.source))
    });
}
#[derive(Clone, Copy)]
enum SearchTableKind {
    Memories,
    Decisions,
}
const ACTIVE_TEMPORAL:&str="status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) AND (valid_from IS NULL OR valid_from <= datetime('now')) AND (valid_until IS NULL OR valid_until > datetime('now'))";
fn fts_keyword_sort(ranked: &mut [SearchCandidate]) {
    ranked.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.matched_keywords.cmp(&a.matched_keywords))
            .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
            .then(b.ts.cmp(&a.ts))
            .then_with(|| a.source.cmp(&b.source))
    });
}
fn search_source_key(kind: SearchTableKind, id: i64, alt: Option<&str>) -> String {
    match (kind, alt) {
        (SearchTableKind::Memories, Some(s)) => s.to_string(),
        (SearchTableKind::Memories, None) => format!("memory::{id}"),
        (SearchTableKind::Decisions, Some(c)) => c.to_string(),
        (SearchTableKind::Decisions, None) => format!("decision::{id}"),
    }
}
fn search_table_recency(
    conn: &Connection, limit: usize, source_prefix: Option<&str>, source_like: Option<&str>, kind: SearchTableKind,
    excerpt_focus_terms: &[String],
) -> Result<Vec<SearchCandidate>, String> {
    let(sql,use_aging)=match kind{SearchTableKind::Memories=>(format!("SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at, compressed_text, age_tier FROM memories WHERE {ACTIVE_TEMPORAL} AND (?2 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?2) ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1"),true,),SearchTableKind::Decisions=>(format!("SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at FROM decisions WHERE {ACTIVE_TEMPORAL} AND (?2 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?2) ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1"),false,),};
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![limit as i64, source_like], |row| {
            let effective_score = blend_importance(
                row.get::<_, Option<f64>>(if use_aging { 4 } else { 3 })?,
                row.get::<_, Option<f64>>(if use_aging { 5 } else { 4 })?,
            );
            let (display, source) = if use_aging {
                let text: String = row.get(1)?;
                let compressed: Option<String> = row.get(9)?;
                let age_tier: String = row.get::<_, Option<String>>(10)?.unwrap_or_else(|| "fresh".to_string());
                let display = crate::aging::get_display_text(&text, &compressed, &age_tier);
                let source = row
                    .get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| format!("memory::{}", row.get::<_, i64>(0).unwrap_or(0)));
                (display, source)
            } else {
                let decision: String = row.get(1)?;
                let source = row
                    .get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| format!("decision::{}", row.get::<_, i64>(0).unwrap_or(0)));
                (decision, source)
            };
            Ok(SearchCandidate {
                source,
                excerpt: query_focused_excerpt_with_terms(&display, excerpt_focus_terms, 220),
                alignment: (0, 0),
                relevance: round4(0.5 * effective_score),
                matched_keywords: 0,
                score: effective_score,
                ts: parse_timestamp_ms(
                    &row.get::<_, Option<String>>(if use_aging { 7 } else { 6 })?
                        .or(row.get::<_, Option<String>>(if use_aging { 8 } else { 7 })?)
                        .unwrap_or_default(),
                ),
                owner_id: None,
                visibility: None,
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().filter(|row| source_matches_prefix(&row.source, source_prefix)).collect())
}
fn search_table_fts(
    conn: &Connection, fts_query: &str, limit: usize, source_like: Option<&str>, source_prefix: Option<&str>, kind: SearchTableKind,
    term_groups: &[Vec<String>], excerpt_focus_terms: &[String], query_text: &str, bm25: &Bm25Weights,
) -> Result<Vec<SearchCandidate>, String> {
    let sql=match kind{SearchTableKind::Memories=>format!("SELECT m.id, m.text, m.source, m.tags, m.score, m.trust_score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility FROM memories_fts fts JOIN memories m ON m.id = fts.rowid WHERE memories_fts MATCH ?1 AND m.{ACTIVE_TEMPORAL} AND (?6 IS NULL OR COALESCE(m.source, 'memory::' || m.id) LIKE ?6) ORDER BY bm25(memories_fts, ?3, ?4, ?5) LIMIT ?2"),SearchTableKind::Decisions=>format!("SELECT d.id, d.decision, d.context, d.score, d.trust_score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility FROM decisions_fts fts JOIN decisions d ON d.id = fts.rowid WHERE decisions_fts MATCH ?1 AND d.{ACTIVE_TEMPORAL} AND (?5 IS NULL OR COALESCE(d.context, 'decision::' || d.id) LIKE ?5) ORDER BY bm25(decisions_fts, ?3, ?4) LIMIT ?2"),};
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut ranked = Vec::new();
    let mut push_fts_row = |id: i64,
                            primary: String,
                            alt: Option<String>,
                            tags: Option<String>,
                            score: Option<f64>,
                            trust_score: Option<f64>,
                            retrievals: Option<i64>,
                            last_accessed: Option<String>,
                            created_at: Option<String>,
                            compressed_text: Option<String>,
                            age_tier: Option<String>,
                            row_owner_id: Option<i64>,
                            row_visibility: Option<String>| {
        let source_key = search_source_key(kind, id, alt.as_deref());
        if !source_matches_prefix(&source_key, source_prefix) {
            return;
        }
        let effective_score = blend_importance(score, trust_score);
        let ts = parse_timestamp_ms(last_accessed.as_deref().or(created_at.as_deref()).unwrap_or(""));
        let display = crate::aging::get_display_text(&primary, &compressed_text, age_tier.as_deref().unwrap_or("fresh"));
        let haystacks: Vec<String> = match kind {
            SearchTableKind::Memories => vec![
                primary.to_lowercase(),
                alt.as_deref().unwrap_or("").to_lowercase(),
                tags.as_deref().unwrap_or("").to_lowercase(),
            ],
            SearchTableKind::Decisions => vec![primary.to_lowercase(), alt.as_deref().unwrap_or("").to_lowercase()],
        };
        let matched = count_matching_term_groups(&haystacks, term_groups);
        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let ranking = fallback_ranking_score(query_text, term_groups.len(), matched, effective_score, recency_d, retrievals);
        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: query_focused_excerpt_with_terms(&display, excerpt_focus_terms, 280),
            alignment: (0, 0),
            relevance: round4(ranking),
            matched_keywords: matched,
            score: effective_score,
            ts,
            owner_id: row_owner_id,
            visibility: row_visibility,
        });
    };
    if matches!(kind, SearchTableKind::Memories) {
        let rows = stmt
            .query_map(params![fts_query, limit as i64, bm25.memories_text, bm25.memories_source, bm25.memories_tags, source_like], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            let (
                id,
                primary,
                alt,
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
            push_fts_row(
                id,
                primary,
                alt,
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
            );
        }
    } else {
        let rows = stmt
            .query_map(params![fts_query, limit as i64, bm25.decisions_text, bm25.decisions_context, source_like], |row| {
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
            })
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            let (
                id,
                primary,
                alt,
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
            push_fts_row(
                id,
                primary,
                alt,
                None,
                score,
                trust_score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            );
        }
    }
    fts_keyword_sort(&mut ranked);
    ranked.truncate(limit);
    Ok(ranked)
}
fn search_table_scan_fallback(
    conn: &Connection, query_text: &str, limit: usize, source_prefix: Option<&str>, kind: SearchTableKind, term_groups: &[Vec<String>],
    excerpt_focus_terms: &[String], alignment_profile: &QueryAlignmentProfile,
) -> Result<Vec<SearchCandidate>, String> {
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let mut ranked = Vec::new();
    let sql=match kind{SearchTableKind::Memories=>format!("SELECT id, text, source, tags, score, trust_score, retrievals, last_accessed, created_at FROM memories WHERE {ACTIVE_TEMPORAL} AND (?1 IS NULL OR COALESCE(source, 'memory::' || id) LIKE ?1)"),SearchTableKind::Decisions=>format!("SELECT id, decision, context, score, trust_score, retrievals, last_accessed, created_at FROM decisions WHERE {ACTIVE_TEMPORAL} AND (?1 IS NULL OR COALESCE(context, 'decision::' || id) LIKE ?1)"),};
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![source_like.as_deref()], |row| {
            Ok(match kind {
                SearchTableKind::Memories => (
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ),
                SearchTableKind::Decisions => (
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    None,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows.flatten() {
        let (id, primary, alt, tags, score, trust_score, retrievals, last_accessed, created_at) = row;
        let source_key = search_source_key(kind, id, alt.as_deref());
        if !source_matches_prefix(&source_key, source_prefix) {
            continue;
        }
        let effective_score = blend_importance(score, trust_score);
        let ts = parse_timestamp_ms(last_accessed.as_deref().or(created_at.as_deref()).unwrap_or(""));
        if term_groups.is_empty() {
            let excerpt = query_focused_excerpt_with_terms(&primary, excerpt_focus_terms, 220);
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
        let haystacks: Vec<String> = match kind {
            SearchTableKind::Memories => vec![
                primary.to_lowercase(),
                alt.as_deref().unwrap_or("").to_lowercase(),
                tags.as_deref().unwrap_or("").to_lowercase(),
            ],
            SearchTableKind::Decisions => vec![primary.to_lowercase(), alt.as_deref().unwrap_or("").to_lowercase()],
        };
        let matched = count_matching_term_groups(&haystacks, term_groups);
        if matched == 0 {
            continue;
        }
        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let ranking = fallback_ranking_score(query_text, term_groups.len(), matched, effective_score, recency_d, retrievals);
        let excerpt = query_focused_excerpt_with_terms(&primary, excerpt_focus_terms, 260);
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
    sort_search_candidates(&mut ranked, !term_groups.is_empty());
    ranked.truncate(limit);
    Ok(ranked)
}
fn search_table(
    conn: &Connection, query_text: &str, limit: usize, source_prefix: Option<&str>, kind: SearchTableKind,
) -> Result<Vec<SearchCandidate>, String> {
    let term_groups = build_search_term_groups(query_text);
    let excerpt_focus_terms = query_focus_terms_for_excerpt(query_text);
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    if term_groups.is_empty() {
        return search_table_recency(conn, limit, source_prefix, source_like.as_deref(), kind, &excerpt_focus_terms);
    }
    let fts_result = search_table_fts(
        conn,
        &build_fts_query(&term_groups),
        limit,
        source_like.as_deref(),
        source_prefix,
        kind,
        &term_groups,
        &excerpt_focus_terms,
        query_text,
        bm25_weights(),
    );
    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_table_scan_fallback(
            conn,
            query_text,
            limit,
            source_prefix,
            kind,
            &term_groups,
            &excerpt_focus_terms,
            &QueryAlignmentProfile::from_query(query_text),
        ),
    }
}
pub(crate) fn search_memories(
    conn: &Connection, query_text: &str, limit: usize, source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table(conn, query_text, limit, source_prefix, SearchTableKind::Memories)
}
pub(crate) fn search_decisions(
    conn: &Connection, query_text: &str, limit: usize, source_prefix: Option<&str>,
) -> Result<Vec<SearchCandidate>, String> {
    search_table(conn, query_text, limit, source_prefix, SearchTableKind::Decisions)
}
include!("engine_semantic.rs");
include!("engine_support.rs");
include!("engine_execution.rs");
