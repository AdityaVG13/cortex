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

use super::*;
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

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
    // NaN/infinite values are treated as the lowest possible relevance so
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

// ─── Jaccard keyword similarity ──────────────────────────────────────────────

/// Jaccard similarity on whitespace-tokenized keyword sets.
///
/// Returns |A ∩ B| / |A ∪ B|.  Returns 0.0 for empty inputs.
/// Used for Tier-1 fuzzy cache matching: queries with >= 0.6 Jaccard similarity
/// are considered close enough to reuse cached results.
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

// ─── RRF fusion ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FusionWeights {
    pub(crate) keyword: f64,
    pub(crate) semantic: f64,
}

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

/// Weighted Reciprocal Rank Fusion (Cormack et al., 2009).
///
/// Fuses multiple ranked lists into a single list using the formula:
///   score(item) = Σ  weight / (k + rank + 1)   for each list containing item
///
/// `k = 60.0` is the standard value from the original paper.
/// Items only in one list still accumulate their 1/(k+1) score.
/// Returns results sorted by fused score descending.
///
/// # Arguments
/// * `lists` -- slice of ranked lists, each a `Vec<(id, score)>` in descending score order
/// * `weights` -- per-list weights in the same order as `lists`
/// * `k`     -- smoothing constant (use `60.0` per Cormack et al.)
///
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

#[cfg(test)]
pub(crate) fn rrf_fuse(lists: &[Vec<(i64, f64)>], k: f64) -> Vec<(i64, f64)> {
    let default_weights = vec![1.0; lists.len()];
    rrf_fuse_weighted(lists, &default_weights, k)
}

// ─── Compound scoring (Task 1.4) ─────────────────────────────────────────────

/// Calculate elapsed days since an ISO 8601 timestamp.
/// Returns days as f64, handling invalid timestamps gracefully (returns very large value).
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

/// Normalize importance score to 0.0-1.0 range.
/// Legacy records may use 0-100, while current records use 0-1.
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

/// Calculate compound score combining RRF rank, importance, and recency.
/// Formula: compound = rrf * 0.6 + importance_norm * 0.2 + recency * 0.2
/// Recency follows 21-day half-life: exp(-days/30)
///
/// # Arguments
/// * `rrf` -- fused RRF score from rrf_fuse()
/// * `importance` -- DB score field (typically 0-100)
/// * `created_at` -- ISO 8601 timestamp string
///
/// Returns compound score in 0.0-1.0 range (approximately)
pub(crate) fn compound_score(rrf: f64, importance: f64, created_at: &str) -> f64 {
    let days = days_since(created_at);
    let recency = (-days / 30.0).exp(); // 21-day half-life
    let importance_normalized = normalize(importance);
    rrf * 0.6 + importance_normalized * 0.2 + recency * 0.2
}

