use super::{
    MIN_BUDGET_HEADROOM_TOKENS, MIN_EXCERPT_CHARS, QueryAlignmentProfile, QueryShapeProfile,
    RecallItem, compare_relevance_desc_source_asc, dedup_preserve_order, query_focused_excerpt,
    query_shape_profile,
};
use crate::handlers::estimate_tokens;
use std::collections::HashMap;

fn budget_band(token_budget: usize) -> usize {
    match token_budget {
        0..=220 => 0,
        221..=400 => 1,
        401..=800 => 2,
        _ => 3,
    }
}

const RANK_CHAR_CAPS: [[usize; 4]; 4] = [
    [180, 120, 90, 70],
    [260, 170, 130, 95],
    [320, 210, 160, 120],
    [420, 260, 200, 150],
];
const SEMANTIC_MAX_ITEMS: [usize; 4] = [4, 6, 8, 10];

fn shape_pick<T>(profile: QueryShapeProfile, exact: T, natural: T, mixed: T) -> T {
    match (profile.exactish, profile.naturalish) {
        (true, false) => exact,
        (false, true) => natural,
        _ => mixed,
    }
}

pub fn budget_rank_char_cap(token_budget: usize, rank_idx: usize, query_text: &str) -> usize {
    let base = RANK_CHAR_CAPS[budget_band(token_budget)][rank_idx.min(3)];
    let profile = query_shape_profile(query_text, None);
    shape_pick(
        profile,
        ((base as f64) * 1.12).round() as usize,
        ((base as f64) * 0.86).round() as usize,
        base,
    )
    .max(MIN_EXCERPT_CHARS)
}

pub fn semantic_budget_min_relevance(top_relevance: f64, query_text: &str) -> f64 {
    if top_relevance < 0.25 {
        return 0.0;
    }
    let profile = query_shape_profile(query_text, None);
    let (scale, floor) = shape_pick(profile, (0.78, 0.20), (0.64, 0.14), (0.72, 0.18));
    (top_relevance * scale).max(floor)
}

pub fn semantic_budget_max_items(token_budget: usize, query_text: &str, hard_cap: usize) -> usize {
    let base = SEMANTIC_MAX_ITEMS[budget_band(token_budget)];
    let profile = query_shape_profile(query_text, None);
    shape_pick(
        profile,
        base.saturating_sub(1).max(3),
        base.saturating_add(1),
        base,
    )
    .clamp(3, 12)
    .min(hard_cap.max(1))
}

pub fn fit_excerpt_to_remaining_budget(
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

#[derive(Clone)]
pub struct RecallFamilyCompaction {
    pub family_key: String,
    pub kept_source: String,
    pub dropped_sources: Vec<String>,
}

pub fn prefer_family_candidate(
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

pub fn compact_budget_family_candidates_with_trace(
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

pub fn compact_budget_family_candidates(
    candidates: Vec<RecallItem>,
    query_text: &str,
    token_budget: usize,
) -> Vec<RecallItem> {
    compact_budget_family_candidates_with_trace(candidates, query_text, token_budget).0
}
