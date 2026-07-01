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

    let query_entities = query_entity_terms(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let query_focus_term_count = alignment_profile.term_count;
    for item in &mut ranked {
        let h = shannon_entropy(&item.excerpt);
        item.entropy = Some(round4(h));
        let boost = ((h - 3.5).max(0.0) * 0.05).min(0.08);
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
            &item.source,
            &item.excerpt,
            &alignment_profile,
            query_focus_term_count,
        );
        if alignment_boost > 0.0 {
            item.relevance = round4(item.relevance * (1.0 + alignment_boost));
        }
    }

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

