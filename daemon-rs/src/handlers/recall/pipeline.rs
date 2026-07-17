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
pub(crate) async fn execute_recall_policy_explain_inner(
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
) -> Result<Value, String> {
    execute_recall_policy_explain_inner(
        state,
        query_text,
        budget,
        k,
        agent,
        ctx,
        source_prefix,
        pool_k,
        None,
    )
    .await
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

    // ── Tier 0/1: Cache check (handled upstream in execute_unified_recall) ────
    // This function is the retrieval engine; caching is the caller's responsibility.

    // ── Crystal search (highest priority, always runs when engine available) ──
    // Crystals bypass Tier 2 early-exit: they represent consolidated knowledge
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

    // ── Tier 2: Keyword-only fast path (ByteRover-inspired) ──────────────────
    // Run FTS5 first. If the top result is confident (score >= 0.93) with a
    // meaningful gap from #2 (delta >= 0.08), return immediately without
    // spending cycles on embedding inference. Target: 40%+ queries resolved here.
    const TIER2_CONFIDENCE: f64 = 0.78;
    const TIER2_GAP: f64 = 0.10;

    let raw_k = if ctx.team_mode { k.max(10) * 5 } else { 20 };
    let mut fts_limit = raw_k.max(20);

    // Collect keyword candidates for Tier 2 check and later RRF
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

    // Tier 2 early exit: high-confidence keyword result with no close competitor
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

    // ── Semantic search (skipped on Tier 2 early exit or no engine) ──────────
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

    // ── RRF fusion ────────────────────────────────────────────────────────────
    // Assign stable integer indices to each unique source across both lists,
    // then fuse ranks. rrf_fuse() works on (i64, f64) so we map source → index.
    //
    // On Tier 2 early exit: semantic list is empty, RRF degrades to keyword-only
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

    // ── Compound scoring + merge into RecallItem map ──────────────────────────
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

    // ── Entropy-weighted re-ranking ───────────────────────────────────────────
    // High-entropy (information-dense) excerpts get a relevance boost (+/-15%
    // around midpoint H=3.5). Applied after compound scoring so entropy acts as
    // a diversity signal on top of the RRF+compound base.
    let query_entities = query_entity_terms(query_text);
    let alignment_profile = QueryAlignmentProfile::from_query(query_text);
    let query_focus_term_count = alignment_profile.term_count;
    let mut ranked: Vec<RecallItem> = merged
        .into_values()
        .map(|mut item| {
            let h = shannon_entropy(&item.excerpt);
            item.entropy = Some(round4(h));
            let boost = ((h - 3.5).max(0.0) * 0.08).min(0.12);
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
            item
        })
        .collect();

    // ── Relevance feedback reranking ──────────────────────────────────────────
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

#[allow(clippy::type_complexity)]
pub(crate) fn run_recall_with_query_vector(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    Ok(run_recall_with_query_vector_trace(
        conn,
        query_text,
        k,
        query_vector,
        ctx,
        source_prefix,
        None,
    )?
    .ranked)
}

