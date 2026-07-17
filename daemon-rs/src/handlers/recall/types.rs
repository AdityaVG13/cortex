// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
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

use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget};
use crate::handlers::{
    estimate_tokens, json_response, now_iso, parse_timestamp_ms, resolve_source_identity,
    truncate_chars,
};
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

use super::query_shape_profile;

// ─── Constants ───────────────────────────────────────────────────────────────

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

// ─── Internal types ──────────────────────────────────────────────────────────

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

/// Shannon entropy of text (bits per character).
/// English prose: ~4.0-4.5, boilerplate: ~2.0-3.0, code/decisions: ~4.5-5.0.
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

// ─── Visibility context ─────────────────────────────────────────────────────

/// Caller identity + team mode flag, threaded through the recall pipeline
/// so visibility filtering can gate results without changing SQL queries.
#[derive(Clone, Copy)]
pub struct RecallContext {
    pub caller_id: Option<i64>,
    pub team_mode: bool,
}

impl RecallContext {
    /// Build from already-resolved caller_id (avoids double argon2).
    pub fn from_caller(caller_id: Option<i64>, state: &RuntimeState) -> Self {
        Self {
            caller_id,
            team_mode: state.team_mode,
        }
    }

    /// Build from runtime state (for MCP/non-HTTP callers). Uses default_owner_id.
    #[allow(dead_code)]
    pub fn from_state(state: &RuntimeState) -> Self {
        Self {
            caller_id: state.default_owner_id,
            team_mode: state.team_mode,
        }
    }

    /// Solo-mode context where everything is visible (no filtering).
    #[allow(dead_code)]
    pub fn solo() -> Self {
        Self {
            caller_id: None,
            team_mode: false,
        }
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn require_team_caller(
    state: &RuntimeState,
    caller_id: Option<i64>,
) -> Result<Option<i64>, Response> {
    if state.team_mode && caller_id.is_none() {
        return Err(json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
        ));
    }
    Ok(caller_id)
}

/// Check whether a record is visible to the current caller.
/// Solo mode: everything visible (no filtering).
/// Team mode (fail closed):
///   - caller_id=None → deny (unidentified caller sees nothing)
///   - owner_id=None → deny (unowned data hidden until backfilled)
///   - owner == caller → allow
///   - visibility shared/team → allow
///   - otherwise → deny
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

// ─── Query types ─────────────────────────────────────────────────────────────

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

#[derive(Deserialize, Default)]
pub struct RecallBody {
    pub q: Option<String>,
    pub k: Option<usize>,
    pub budget: Option<usize>,
    pub agent: Option<String>,
    pub source_prefix: Option<String>,
    #[serde(alias = "policyMode")]
    pub policy_mode: Option<String>,
}

