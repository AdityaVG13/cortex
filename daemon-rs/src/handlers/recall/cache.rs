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

pub(crate) fn round4(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * 10000.0).round() / 10000.0
}

/// Ebbinghaus-aware retrieval bump.
///
/// Each recall:
///   1. Increments retrieval count
///   2. Updates last_accessed timestamp
///   3. Boosts score using spaced-repetition formula:
///      new_score = min(1.0, current_score + boost)
///      boost = 0.15 * (1.0 / (1.0 + 0.1 * retrievals))
///
///   Early retrievals give big boosts (0.15 → 0.14 → 0.12...),
///   diminishing as the memory is already well-reinforced.
///   This counteracts the time-based decay in decay_pass().
/// Batch-update retrieval stats for all returned results in 2 statements
/// instead of 2*N individual UPDATEs.
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

// ─── Content dedup / served tracking ─────────────────────────────────────────

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

/// Content served within this window is suppressed to avoid echo in rapid
/// successive recalls. After this TTL, the same content can be re-served.
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

// ─── Recall pattern tracking / pre-cache ─────────────────────────────────────

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

/// Tier 0: Exact query match for the agent.
/// Tier 1: Jaccard fuzzy match on keywords (threshold >= 0.6) across all agents' caches.
///
/// Both tiers enforce the 5-minute TTL.  The pre_cache is a per-agent HashMap;
/// for Tier 1 we scan all entries and pick the best Jaccard match above the threshold.
/// LRU ordering is maintained by `predict_and_cache` (max 100 entries, oldest evicted).
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

    // Tier 0: exact match for this agent
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

    // Tier 1: fuzzy Jaccard match across scoped entries (same owner in team mode).
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

