use super::{
    ALIGNMENT_BOOST_MAX, ALIGNMENT_COVERAGE_BONUS_MAX, ALIGNMENT_EXACT_BONUS_MAX,
    ENTITY_SIGNAL_MATCH_WEIGHT, ENTITY_SIGNAL_MAX_BOOST, ENTITY_SIGNAL_OVERLAP_WEIGHT,
    TEMPORAL_INTENT_MULTIPLIER_RANGE,
};
pub(super) use super::{
    BUDGET_PRESSURE_EARLY_STOP_THRESHOLD, BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD,
    MIN_BUDGET_HEADROOM_TOKENS, MIN_EXCERPT_CHARS, QueryShapeProfile, RecallItem,
    dedup_preserve_order, query_shape_profile, quote_fts_match_term,
};
use crate::handlers::{parse_timestamp_ms, sorted_has};
use chrono::Utc;
use std::collections::HashSet;

pub fn round4(value: f64) -> f64 {
    if value.is_finite() {
        (value * 10000.0).round() / 10000.0
    } else {
        0.0
    }
}

#[path = "engine_rank/terms.rs"]
mod terms;
pub use terms::*;

#[path = "engine_rank/budget.rs"]
mod budget;
pub use budget::*;

pub fn recency_days(value: Option<&str>) -> i64 {
    let ts = value.map(parse_timestamp_ms).unwrap_or(0);
    if ts == 0 {
        return 3650;
    }
    (Utc::now().timestamp_millis() - ts).max(0) / (24 * 60 * 60 * 1000)
}
pub fn blend_importance(score: Option<f64>, trust_score: Option<f64>) -> f64 {
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
pub fn compare_relevance_desc_source_asc(
    a_relevance: f64,
    a_source: &str,
    b_relevance: f64,
    b_source: &str,
) -> std::cmp::Ordering {
    let fin = |v: f64| if v.is_finite() { v } else { f64::NEG_INFINITY };
    fin(b_relevance)
        .total_cmp(&fin(a_relevance))
        .then_with(|| a_source.cmp(b_source))
}
#[derive(Clone)]
pub struct QueryAlignmentProfile {
    pub lower_query: String,
    pub terms: Vec<String>,
    pub term_count: usize,
}
impl QueryAlignmentProfile {
    pub fn from_query(query_text: &str) -> Self {
        let terms = query_focus_terms_for_excerpt(query_text);
        Self {
            lower_query: query_text.trim().to_ascii_lowercase(),
            term_count: terms.len().max(1),
            terms,
        }
    }
    pub fn alignment_score(&self, text: &str) -> (usize, usize) {
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
pub fn prefer_query_focused_excerpt_with_profile(
    current: &str,
    candidate: &str,
    profile: &QueryAlignmentProfile,
) -> bool {
    let current_score = profile.alignment_score(current);
    let candidate_score = profile.alignment_score(candidate);
    candidate_score > current_score
        || (candidate_score == current_score && candidate.len() < current.len())
}
#[allow(dead_code)]
pub fn prefer_query_focused_excerpt(current: &str, candidate: &str, query_text: &str) -> bool {
    let profile = QueryAlignmentProfile::from_query(query_text);
    prefer_query_focused_excerpt_with_profile(current, candidate, &profile)
}
pub fn query_prefers_recency(query_text: &str) -> bool {
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
pub fn temporal_intent_multiplier(ts_ms: i64) -> f64 {
    if ts_ms <= 0 {
        return 1.0 - (TEMPORAL_INTENT_MULTIPLIER_RANGE * 0.25);
    }
    let age_days =
        ((Utc::now().timestamp_millis() - ts_ms).max(0) as f64) / (1000.0 * 60.0 * 60.0 * 24.0);
    let freshness = (1.0 / (1.0 + age_days / 14.0)).clamp(0.0, 1.0);
    1.0 + ((freshness - 0.5) * TEMPORAL_INTENT_MULTIPLIER_RANGE)
}
pub fn query_alignment_boost_with_profile(
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
pub fn is_entity_stopword(token: &str) -> bool {
    const WORDS: &[&str] = &[
        "a", "about", "an", "and", "around", "could", "for", "from", "had", "has", "have", "how",
        "into", "or", "our", "should", "that", "the", "their", "there", "these", "this", "those",
        "what", "when", "where", "which", "why", "will", "with", "would", "your",
    ];
    sorted_has(WORDS, token)
}
pub fn is_short_technical_term(token: &str) -> bool {
    const WORDS: &[&str] = &[
        "ai", "api", "cpu", "db", "dns", "gpu", "http", "https", "id", "ios", "ip", "jwt", "ml",
        "ram", "sdk", "sql", "ssh", "tls", "ui", "uid", "url", "uuid", "ux",
    ];
    sorted_has(WORDS, token)
}
pub fn extract_entity_like_terms(text: &str) -> HashSet<String> {
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
        if token.chars().any(|c| {
            c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '/' | ':')
        }) || lowered.len() >= 9
        {
            terms.insert(lowered);
        }
    }
    terms
}
fn absorb_entity_terms(dst: &mut HashSet<String>, src: impl IntoIterator<Item = String>) {
    dst.extend(src.into_iter().filter(|term| {
        !is_entity_stopword(term) && (term.len() >= 3 || is_short_technical_term(term))
    }));
}
pub fn query_entity_terms(query_text: &str) -> HashSet<String> {
    let mut terms = extract_entity_like_terms(query_text);
    if terms.is_empty() {
        absorb_entity_terms(&mut terms, query_focus_terms(query_text));
    }
    terms
}
pub fn entity_alignment_metrics_with_terms(
    haystack: &str,
    query_entities: &HashSet<String>,
) -> (usize, f64) {
    if query_entities.is_empty() {
        return (0, 0.0);
    }
    let mut haystack_terms = extract_entity_like_terms(haystack);
    if haystack_terms.is_empty() {
        absorb_entity_terms(&mut haystack_terms, extract_search_keywords(haystack));
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
pub fn entity_signal_boost(matches: usize, overlap: f64) -> f64 {
    if matches == 0 {
        return 0.0;
    }
    let overlap_component = overlap.clamp(0.0, 1.0) * ENTITY_SIGNAL_OVERLAP_WEIGHT;
    let match_component = matches.min(3) as f64 * ENTITY_SIGNAL_MATCH_WEIGHT;
    (overlap_component + match_component).min(ENTITY_SIGNAL_MAX_BOOST)
}

#[path = "engine_rank/fusion.rs"]
mod fusion;
pub use fusion::*;
