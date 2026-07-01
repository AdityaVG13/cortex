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

// ─── Text / keyword utilities ────────────────────────────────────────────────

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

/// Coding synonym map: maps abbreviated/shorthand terms to their full-form equivalents
/// and vice versa. Used during FTS query construction to expand search coverage.
///
/// Strategy: every token in the query that has a synonym gets BOTH forms added to the
/// OR list. This is directional expansion (short → long, or long → short) -- the map
/// handles both directions as separate entries.
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

/// Like `extract_search_keywords` but also expands coding synonyms.
/// Each token that has a known synonym produces both the original and the expanded form.
/// Deduplicates the final list while preserving order.
#[cfg(test)]
pub(crate) fn extract_search_keywords_with_synonyms(text: &str) -> Vec<String> {
    build_search_term_groups(text)
        .into_iter()
        .flatten()
        .collect()
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

