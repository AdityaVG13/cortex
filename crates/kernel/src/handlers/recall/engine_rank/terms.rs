use super::{
    BUDGET_PRESSURE_EARLY_STOP_THRESHOLD, BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD,
    query_shape_profile, quote_fts_match_term,
};
use crate::handlers::{sorted_has, truncate_chars};
use std::collections::HashSet;

pub fn normalize_text(input: &str) -> String {
    let input = crate::clockwork::bound_query_text(input);
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        out.push(
            if ch.is_ascii_alphanumeric() || ch == '-' || ch.is_ascii_whitespace() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            },
        );
    }
    out
}

fn is_keyword_stop_word(word: &str) -> bool {
    const WORDS: &[&str] = &[
        "a", "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "being", "both",
        "but", "by", "can", "could", "did", "do", "does", "each", "every", "few", "for", "from",
        "had", "has", "have", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its",
        "may", "me", "might", "more", "most", "my", "no", "not", "of", "on", "or", "our", "shall",
        "should", "so", "some", "that", "the", "their", "then", "this", "to", "was", "were",
        "what", "when", "where", "which", "who", "why", "will", "with", "would", "your",
    ];
    sorted_has(WORDS, word)
}

fn extract_keywords_min_len(text: &str, min_len: usize) -> Vec<String> {
    normalize_text(text)
        .split_whitespace()
        .filter(|word| word.len() >= min_len && !is_keyword_stop_word(word))
        .take(crate::clockwork::MAX_QUERY_TOKENS)
        .map(str::to_string)
        .collect()
}

pub fn extract_keywords(text: &str) -> Vec<String> {
    extract_keywords_min_len(text, 3)
}

pub fn extract_search_keywords(text: &str) -> Vec<String> {
    extract_keywords_min_len(text, 2)
}

#[path = "terms/synonyms.rs"]
mod synonyms;
pub use synonyms::{coding_synonyms, query_intent_alias_terms};

pub fn is_low_signal_query_token(token: &str) -> bool {
    const WORDS: &[&str] = &[
        "a", "about", "an", "are", "as", "at", "be", "been", "being", "by", "did", "do", "does",
        "for", "from", "how", "i", "in", "into", "is", "it", "its", "me", "my", "of", "on", "our",
        "that", "the", "their", "this", "to", "was", "we", "were", "what", "when", "where",
        "which", "who", "why", "with", "you", "your",
    ];
    sorted_has(WORDS, token)
}

pub fn build_search_term_groups(text: &str) -> Vec<Vec<String>> {
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
        if let Some(expanded) = coding_synonyms(&word).map(str::to_string) {
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

pub fn count_matching_term_groups(haystacks: &[String], term_groups: &[Vec<String>]) -> i64 {
    term_groups
        .iter()
        .filter(|group| {
            group.iter().any(|term| {
                haystacks
                    .iter()
                    .any(|haystack| crate::clockwork::hay_has_lexical(haystack, term))
            })
        })
        .count() as i64
}

pub fn query_focus_terms(query_text: &str) -> Vec<String> {
    let mut terms = extract_keywords(query_text);
    let mut seen: HashSet<String> = terms.iter().cloned().collect();
    for term in build_search_term_groups(query_text).into_iter().flatten() {
        if seen.insert(term.clone()) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        terms = extract_search_keywords(query_text);
    }
    terms
}

pub fn build_fts_query(groups: &[Vec<String>]) -> String {
    groups
        .iter()
        .filter_map(|group| {
            let alternates: Vec<String> = group
                .iter()
                .filter_map(|t| quote_fts_match_term(t))
                .collect();
            if alternates.is_empty() {
                return None;
            }
            let joined = alternates.join(" OR ");
            Some(if alternates.len() > 1 {
                format!("({joined})")
            } else {
                joined
            })
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn query_focus_terms_for_excerpt(query_text: &str) -> Vec<String> {
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

pub fn excerpt_signature_terms(source: &str, excerpt: &str) -> HashSet<String> {
    let mut terms = HashSet::new();
    terms.extend(
        extract_search_keywords(source)
            .into_iter()
            .chain(extract_search_keywords(excerpt))
            .filter(|token| token.len() > 2),
    );
    terms
}

pub fn term_set_jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let union = a.union(b).count();
    a.intersection(b).count() as f64 / union as f64
}

pub fn query_term_coverage_gain(
    signature_terms: &HashSet<String>,
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> usize {
    query_terms
        .iter()
        .filter(|term| signature_terms.contains(*term) && !covered_terms.contains(*term))
        .count()
}

pub fn should_skip_redundant_budget_candidate(
    signature_terms: &HashSet<String>,
    selected_signatures: &[HashSet<String>],
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> bool {
    if selected_signatures.is_empty()
        || signature_terms.is_empty()
        || query_term_coverage_gain(signature_terms, query_terms, covered_terms) > 0
    {
        return false;
    }
    selected_signatures
        .iter()
        .map(|existing| term_set_jaccard(existing, signature_terms))
        .fold(0.0_f64, f64::max)
        >= BUDGET_REDUNDANCY_SIMILARITY_THRESHOLD
}

pub fn update_query_term_coverage(
    signature_terms: &HashSet<String>,
    query_terms: &HashSet<String>,
    covered_terms: &mut HashSet<String>,
) {
    covered_terms.extend(
        query_terms
            .iter()
            .filter(|term| signature_terms.contains(*term))
            .cloned(),
    );
}

pub fn should_early_stop_budget_selection(
    token_budget: usize,
    spent_tokens: usize,
    selected_count: usize,
    query_terms: &HashSet<String>,
    covered_terms: &HashSet<String>,
) -> bool {
    if token_budget == 0
        || selected_count < 2
        || query_terms.is_empty()
        || covered_terms.len() < query_terms.len()
    {
        return false;
    }
    let pressure = spent_tokens as f64 / token_budget as f64;
    pressure >= BUDGET_PRESSURE_EARLY_STOP_THRESHOLD
}

fn slice_chars(text: &str, start_char: usize, end_char: usize, total_chars: usize) -> String {
    let mut excerpt = text
        .chars()
        .skip(start_char)
        .take(end_char.saturating_sub(start_char))
        .collect::<String>();
    if start_char > 0 {
        excerpt = format!("...{excerpt}");
    }
    if end_char < total_chars {
        excerpt.push_str("...");
    }
    excerpt
}

fn excerpt_user_answer(
    text: &str,
    lower_text: &str,
    max_chars: usize,
    total_chars: usize,
) -> Option<String> {
    if !lower_text.contains("[assistant-question]") {
        return None;
    }
    let answer_byte_idx = lower_text.find("[user-answer]")?;
    let answer_char_idx = text[..answer_byte_idx].chars().count();
    let excerpt = slice_chars(
        text,
        answer_char_idx,
        (answer_char_idx + max_chars).min(total_chars),
        total_chars,
    );
    (!excerpt.trim().is_empty()).then_some(excerpt)
}

pub fn query_focused_excerpt_with_terms(
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
    if let Some(excerpt) = excerpt_user_answer(text, &lower_text, max_chars, total_chars) {
        return excerpt;
    }
    if sorted_focus_terms.is_empty() {
        return truncate_chars(text, max_chars);
    }
    let Some(byte_idx) = sorted_focus_terms
        .iter()
        .find_map(|term| lower_text.find(term.as_str()))
    else {
        return truncate_chars(text, max_chars);
    };
    let hit_char_idx = text[..byte_idx].chars().count();
    let left_window = max_chars / 3;
    let mut start_char = hit_char_idx.saturating_sub(left_window);
    let end_char = (start_char + max_chars).min(total_chars);
    if end_char - start_char < max_chars {
        start_char = end_char.saturating_sub(max_chars);
    }
    slice_chars(text, start_char, end_char, total_chars)
}

pub fn query_focused_excerpt(text: &str, query_text: &str, max_chars: usize) -> String {
    let terms = query_focus_terms_for_excerpt(query_text);
    query_focused_excerpt_with_terms(text, &terms, max_chars)
}
