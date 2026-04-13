// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::ensure_auth_with_caller;
use super::{estimate_tokens, json_response, now_iso, resolve_source_identity, truncate_chars};
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::state::{PreCacheEntry, RecallHistoryEntry, RuntimeState};

// ─── Constants ───────────────────────────────────────────────────────────────

const MAX_RECALL_HISTORY: usize = 50;
const PRECACHE_TTL_MS: i64 = 5 * 60 * 1000;
const SEMANTIC_SIM_FLOOR: f64 = 0.3;
const SEMANTIC_SCALE_BASE: f64 = 0.55;

// ─── Internal types ──────────────────────────────────────────────────────────

#[derive(Clone)]
struct RecallItem {
    source: String,
    relevance: f64,
    excerpt: String,
    method: String,
    tokens: Option<usize>,
    entropy: Option<f64>,
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
struct SearchCandidate {
    source: String,
    excerpt: String,
    relevance: f64,
    matched_keywords: i64,
    score: f64,
    ts: i64,
    owner_id: Option<i64>,
    visibility: Option<String>,
}

#[derive(Clone)]
struct SemanticCandidate {
    source: String,
    excerpt: String,
    relevance: f64,
}

type MemorySemanticRow = (Vec<u8>, String, String, Option<i64>, Option<String>);
type DecisionSemanticRow = (Vec<u8>, String, Option<String>, Option<i64>, Option<String>);

// ─── Visibility context ─────────────────────────────────────────────────────

/// Caller identity + team mode flag, threaded through the recall pipeline
/// so visibility filtering can gate results without changing SQL queries.
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
    pub fn solo() -> Self {
        Self {
            caller_id: None,
            team_mode: false,
        }
    }
}

/// Check whether a record is visible to the current caller.
/// Solo mode: everything visible (no filtering).
/// Team mode (fail closed):
///   - caller_id=None → deny (unidentified caller sees nothing)
///   - owner_id=None → deny (unowned data hidden until backfilled)
///   - owner == caller → allow
///   - visibility shared/team → allow
///   - otherwise → deny
fn is_visible(owner_id: Option<i64>, visibility: Option<&str>, ctx: &RecallContext) -> bool {
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

// ─── Query types ─────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct RecallQuery {
    pub q: Option<String>,
    pub k: Option<usize>,
    pub budget: Option<usize>,
    pub agent: Option<String>,
}

// ─── GET /recall ─────────────────────────────────────────────────────────────

pub async fn handle_recall(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    let k = query.k.unwrap_or(10);
    let budget = query.budget.unwrap_or(200);
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;

    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing query parameter: q" }),
        );
    }

    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_unified_recall(&state, q.trim(), budget, k, &agent, &ctx).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Recall failed: {err}") }),
        ),
    }
}

pub async fn handle_semantic_recall(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    let k = query.k.unwrap_or(10);
    let budget = query.budget.unwrap_or(200);
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;

    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing query parameter: q" }),
        );
    }

    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_semantic_recall(&state, q.trim(), budget, k, &agent, &ctx).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Semantic recall failed: {err}") }),
        ),
    }
}

// ─── GET /recall/budget ──────────────────────────────────────────────────────

pub async fn handle_budget_recall(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<RecallQuery>,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = match query.q.as_deref() {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing query parameter: q" }),
            );
        }
    };

    let budget = query.budget.unwrap_or(300);
    let k = query.k.unwrap_or(10);

    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    let engine = state.embedding_engine.as_deref();
    match run_budget_recall_with_engine(
        &mut conn,
        &q,
        budget,
        k,
        engine,
        &ctx,
        Some(&state.degraded_mode),
    ) {
        Ok(results) => {
            let spent: usize = results
                .iter()
                .map(|item| {
                    item.tokens.unwrap_or_else(|| {
                        estimate_tokens(&format!("{}{}", item.source, item.excerpt))
                    })
                })
                .sum();
            let saved = budget as i64 - spent as i64;
            json_response(
                StatusCode::OK,
                json!({
                    "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
                    "budget": budget,
                    "spent": spent,
                    "saved": saved,
                }),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Budget recall failed: {e}") }),
        ),
    }
}

// ─── GET /peek ───────────────────────────────────────────────────────────────

pub async fn handle_peek(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<RecallQuery>,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = match &query.q {
        Some(q) if !q.is_empty() => q.clone(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "Missing query parameter: q"}),
            );
        }
    };
    let k = query.k.unwrap_or(10);
    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    match run_recall(&mut conn, &q, k, &ctx) {
        Ok(results) => {
            let matches: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "source": r.source,
                        "relevance": r.relevance,
                        "method": r.method,
                    })
                })
                .collect();
            json_response(
                StatusCode::OK,
                json!({"count": matches.len(), "matches": matches}),
            )
        }
        Err(e) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    }
}

// ─── Unified recall pipeline ─────────────────────────────────────────────────

async fn emit_recall_query_event(state: &RuntimeState, agent: &str, payload: Value) {
    let conn = state.db.lock().await;
    if super::log_event(&conn, "recall_query", payload, agent).is_ok() {
        checkpoint_wal_best_effort(&conn);
    }
}

pub async fn execute_unified_recall(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
) -> Result<Value, String> {
    // Check pre-cache
    if budget > 0 {
        if let Some(cached) = get_pre_cached(state, agent, query_text).await {
            let deduped_cached = dedup_and_mark_served(state, agent, query_text, cached).await;
            emit_recall_query_event(
                state,
                agent,
                json!({
                    "agent": agent,
                    "query": truncate_chars(query_text, 120),
                    "budget": budget,
                    "spent": 0,
                    "saved": budget as i64,
                    "hits": deduped_cached.len(),
                    "mode": if budget >= 500 { "full" } else { "balanced" },
                    "cached": true
                }),
            )
            .await;
            return Ok(json!({
                "results": deduped_cached.into_iter().map(recall_to_json).collect::<Vec<_>>(),
                "budget": budget,
                "spent": 0,
                "saved": budget as i64,
                "mode": if budget >= 500 { "full" } else { "balanced" },
                "cached": true
            }));
        }
    }

    let mut conn = state.db.lock().await;
    let engine = state.embedding_engine.as_deref();
    let dflag = Some(&state.degraded_mode);
    let results = if budget == 0 {
        run_recall_with_engine(&mut conn, query_text, k, engine, ctx, dflag)?
    } else {
        run_budget_recall_with_engine(&mut conn, query_text, budget, k, engine, ctx, dflag)?
    };

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
    record_recall_pattern(state, agent, query_text).await;

    // Fire-and-forget pre-cache warming
    let state_clone = state.clone();
    let agent_owned = agent.to_string();
    let query_owned = query_text.to_string();
    tokio::spawn(async move {
        let _ = predict_and_cache(state_clone, &agent_owned, &query_owned).await;
    });

    // Headlines mode (budget == 0)
    if budget == 0 {
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
        emit_recall_query_event(
            state,
            agent,
            json!({
                "agent": agent,
                "query": truncate_chars(query_text, 120),
                "budget": 0,
                "spent": 0,
                "saved": 0,
                "hits": headlines.len(),
                "mode": "headlines",
                "cached": false
            }),
        )
        .await;
        return Ok(json!({
            "count": headlines.len(),
            "results": headlines,
            "budget": 0,
            "spent": 0,
            "mode": "headlines"
        }));
    }

    // Dedup and budget accounting
    let results = dedup_and_mark_served(state, agent, query_text, results).await;
    let spent: usize = results
        .iter()
        .map(|item| {
            item.tokens
                .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
        })
        .sum();
    let saved = budget as i64 - spent as i64;
    let mode = if budget >= 500 { "full" } else { "balanced" };
    emit_recall_query_event(
        state,
        agent,
        json!({
            "agent": agent,
            "query": truncate_chars(query_text, 120),
            "budget": budget,
            "spent": spent,
            "saved": saved,
            "hits": results.len(),
            "mode": mode,
            "cached": false
        }),
    )
    .await;

    let payload = json!({
        "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": spent,
        "saved": saved,
        "mode": mode
    });

    Ok(payload)
}

pub async fn execute_semantic_recall(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
) -> Result<Value, String> {
    let query_vector = state
        .embedding_engine
        .as_ref()
        .and_then(|engine| engine.embed(query_text));
    let semantic_available = query_vector.is_some();
    let budgeted = {
        let conn = state.db.lock().await;
        let results = run_semantic_recall_with_query_vector(&conn, k, query_vector.as_deref(), ctx);
        apply_semantic_budget(results, budget)
    };
    let spent: usize = budgeted
        .iter()
        .map(|item| {
            item.tokens
                .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
        })
        .sum();
    let saved = budget as i64 - spent as i64;

    emit_recall_query_event(
        state,
        agent,
        json!({
            "query": query_text,
            "mode": "semantic",
            "k": k,
            "budget": budget,
            "results": budgeted.len(),
            "semantic_available": semantic_available,
        }),
    )
    .await;

    Ok(json!({
        "results": budgeted.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "mode": "semantic",
        "budget": budget,
        "spent": spent,
        "saved": saved,
        "semanticAvailable": semantic_available,
    }))
}

// ─── Core recall ─────────────────────────────────────────────────────────────

fn run_recall(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    ctx: &RecallContext,
) -> Result<Vec<RecallItem>, String> {
    run_recall_with_engine(conn, query_text, k, None, ctx, None)
}

#[allow(clippy::type_complexity)]
fn run_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let query_vector = engine.and_then(|engine| engine.embed(query_text));
    if engine.is_some() {
        update_semantic_search_health(degraded_flag, query_vector.is_some(), true);
    }

    run_recall_with_query_vector(conn, query_text, k, query_vector.as_deref(), ctx)
}

#[allow(clippy::type_complexity)]
fn run_recall_with_query_vector(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
) -> Result<Vec<RecallItem>, String> {
    let extracted = extract_keywords(query_text);
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

    // crystal results keyed by source -- inserted into final merged map after fusion
    let mut crystal_items: HashMap<String, RecallItem> = HashMap::new();

    if let Some(query_vec) = query_vector {
        for (crystal_id, label, text, relevance) in crate::crystallize::search_crystals_filtered(
            conn,
            query_vec,
            3,
            ctx.caller_id,
            ctx.team_mode,
        ) {
            let source = format!("crystal::{crystal_id}::{label}");
            crystal_items.insert(
                source.clone(),
                RecallItem {
                    source,
                    relevance: scale_sim(relevance as f32),
                    excerpt: text.chars().take(300).collect(),
                    method: "crystal".to_string(),
                    tokens: None,
                    entropy: None,
                },
            );
        }
    }

    // ── Tier 2: Keyword-only fast path (ByteRover-inspired) ──────────────────
    // Run FTS5 first. If the top result is confident (score >= 0.93) with a
    // meaningful gap from #2 (delta >= 0.08), return immediately without
    // spending cycles on embedding inference. Target: 40%+ queries resolved here.
    const TIER2_CONFIDENCE: f64 = 0.93;
    const TIER2_GAP: f64 = 0.08;

    let raw_k = if ctx.team_mode { k.max(10) * 5 } else { 20 };
    let mut fts_limit = raw_k.max(20);

    // Collect keyword candidates for Tier 2 check and later RRF
    let kw_candidates: Vec<SearchCandidate> = {
        let mut retry = 0;
        let mut all: Vec<SearchCandidate> = Vec::new();
        loop {
            all.clear();
            for row in search_memories(conn, &keyword_query, fts_limit)?
                .into_iter()
                .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
            {
                all.push(row);
            }
            for row in search_decisions(conn, &keyword_query, fts_limit)?
                .into_iter()
                .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
            {
                all.push(row);
            }
            all.sort_by(|a, b| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
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
    let tier2_resolved = kw_candidates.len() >= 2
        && kw_candidates[0].relevance >= TIER2_CONFIDENCE
        && (kw_candidates[0].relevance - kw_candidates[1].relevance) >= TIER2_GAP;

    // ── Semantic search (skipped on Tier 2 early exit or no engine) ──────────
    // Produces a ranked list of (source, score) pairs for RRF.
    // Also accumulates per-source metadata (score, ts) for compound scoring.
    let semantic_candidates = if tier2_resolved {
        Vec::new()
    } else {
        query_vector
            .map(|query_vec| collect_semantic_candidates(conn, query_vec, ctx))
            .unwrap_or_default()
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

    let fused = rrf_fuse(&[kw_list, sem_list], 60.0);

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
                (sem.excerpt.clone(), 1.0_f64, 0_i64, "semantic")
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
        let relevance = round4(compound_score(
            *rrf_score,
            importance * 100.0,
            &created_at_str,
        ));

        merged.insert(
            source.clone(),
            RecallItem {
                source,
                relevance,
                excerpt,
                method: method.to_string(),
                tokens: None,
                entropy: None,
            },
        );
    }

    // Crystal items bypass RRF (they're already fused/consolidated knowledge);
    // insert after -- they will not be overwritten since crystal:: keys don't appear in kw/sem
    for (src, item) in crystal_items {
        merged.entry(src).or_insert(item);
    }

    // ── Entropy-weighted re-ranking ───────────────────────────────────────────
    // High-entropy (information-dense) excerpts get a relevance boost (+/-15%
    // around midpoint H=3.5). Applied after compound scoring so entropy acts as
    // a diversity signal on top of the RRF+compound base.
    let mut ranked: Vec<RecallItem> = merged
        .into_values()
        .map(|mut item| {
            let h = shannon_entropy(&item.excerpt);
            item.entropy = Some(round4(h));
            item.relevance = round4(item.relevance * (1.0 + (h - 3.5) * 0.15));
            item
        })
        .collect();

    // ── Relevance feedback reranking ──────────────────────────────────────────
    // Boost results that have been useful in past recalls (unfolded),
    // penalize results that were consistently ignored. Graceful no-op when
    // no feedback data exists (cold start).
    let sources: Vec<String> = ranked.iter().map(|r| r.source.clone()).collect();
    let boosts = super::feedback::compute_boosts(conn, &sources);
    if !boosts.is_empty() {
        for item in &mut ranked {
            if let Some(&boost) = boosts.get(&item.source) {
                item.relevance = round4(item.relevance * (1.0 + boost));
            }
        }
    }

    ranked.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(k);

    bump_retrievals_batch(conn, &ranked);

    Ok(ranked)
}

fn run_budget_recall(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    ctx: &RecallContext,
) -> Result<Vec<RecallItem>, String> {
    run_budget_recall_with_engine(conn, query_text, token_budget, k, None, ctx, None)
}

fn run_semantic_recall_with_query_vector(
    conn: &Connection,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
) -> Vec<RecallItem> {
    let mut ranked: Vec<RecallItem> = query_vector
        .map(|query_vec| collect_semantic_candidates(conn, query_vec, ctx))
        .unwrap_or_default()
        .into_iter()
        .map(|candidate| RecallItem {
            source: candidate.source,
            relevance: round4(candidate.relevance),
            excerpt: candidate.excerpt,
            method: "semantic".to_string(),
            tokens: None,
            entropy: None,
        })
        .collect();

    for item in &mut ranked {
        let h = shannon_entropy(&item.excerpt);
        item.entropy = Some(round4(h));
        item.relevance = round4(item.relevance * (1.0 + (h - 3.5) * 0.10));
    }

    ranked.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(k);
    bump_retrievals_batch(conn, &ranked);
    ranked
}

fn apply_semantic_budget(raw: Vec<RecallItem>, token_budget: usize) -> Vec<RecallItem> {
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

    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, mut item) in raw.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }

        let max_chars = if idx == 0 {
            ((remaining as f64 * 3.8) as usize).min(280)
        } else if idx <= 2 {
            ((remaining as f64 * 3.8) as usize).min(140)
        } else {
            ((remaining as f64 * 3.8) as usize).min(80)
        };
        item.excerpt = truncate_chars(&item.excerpt, max_chars);
        let tokens = estimate_tokens(&format!("{}{}", item.source, item.excerpt));
        item.tokens = Some(tokens);
        spent += tokens;
        budgeted.push(item);
    }
    budgeted
}

fn run_budget_recall_with_query_vector(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
) -> Result<Vec<RecallItem>, String> {
    let raw = run_recall_with_query_vector(conn, query_text, k, query_vector, ctx)?;
    if raw.is_empty() {
        return Ok(vec![]);
    }

    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, item) in raw.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }

        let max_chars = if idx == 0 {
            ((remaining as f64 * 3.8) as usize).min(400)
        } else if idx <= 2 {
            ((remaining as f64 * 3.8) as usize).min(150)
        } else {
            ((remaining as f64 * 3.8) as usize).min(60)
        };

        let original = item.excerpt.clone();
        let mut excerpt = truncate_chars(&original, max_chars);
        if excerpt.chars().count() < original.chars().count() {
            excerpt.push_str("...");
        }
        let tokens = estimate_tokens(&format!("{}{}", item.source, excerpt));
        spent += tokens;

        budgeted.push(RecallItem {
            source: item.source,
            relevance: item.relevance,
            excerpt,
            method: item.method,
            tokens: Some(tokens),
            entropy: item.entropy,
        });
    }

    Ok(budgeted)
}

fn run_budget_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let query_vector = engine.and_then(|engine| engine.embed(query_text));
    if engine.is_some() {
        update_semantic_search_health(degraded_flag, query_vector.is_some(), true);
    }

    run_budget_recall_with_query_vector(
        conn,
        query_text,
        token_budget,
        k,
        query_vector.as_deref(),
        ctx,
    )
}

fn update_semantic_search_health(
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

// ─── Search helpers ──────────────────────────────────────────────────────────

fn search_memories(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
    let tokens = extract_search_keywords_with_synonyms(query_text);

    if tokens.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, text, source, tags, score, retrievals, last_accessed, created_at, compressed_text, age_tier \
                 FROM memories WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                let text: String = row.get(1)?;
                let compressed: Option<String> = row.get(8)?;
                let age_tier: String = row
                    .get::<_, Option<String>>(9)?
                    .unwrap_or_else(|| "fresh".to_string());
                let display = crate::aging::get_display_text(&text, &compressed, &age_tier);
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("memory::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: truncate_chars(&display, 200),
                    relevance: round4(0.5 * row.get::<_, Option<f64>>(4)?.unwrap_or(1.0).max(0.0)),
                    matched_keywords: 0,
                    score: row.get::<_, Option<f64>>(4)?.unwrap_or(1.0).max(0.0),
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(6)?
                            .or(row.get::<_, Option<String>>(7)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;

        return Ok(rows.flatten().collect());
    }

    let fts_query = tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");

    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // Field-boosted BM25: memories_fts columns are (text, source, tags).
        // Weights: text=1.0, source=5.0, tags=3.0 -- matches in source (e.g. file paths)
        // and tags carry higher signal than body text.
        // bm25() returns negative values (more negative = better match), so ORDER BY ASC.
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.text, m.source, m.tags, m.score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility \
                 FROM memories_fts fts \
                 JOIN memories m ON m.id = fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.status = 'active' \
                 AND (m.expires_at IS NULL OR m.expires_at > datetime('now')) \
                 ORDER BY bm25(memories_fts, 1.0, 5.0, 3.0) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![&fts_query, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
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

        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                text,
                source,
                tags,
                score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = source.clone().unwrap_or_else(|| format!("memory::{id}"));
            let score = score.unwrap_or(1.0).max(0.0);
            let ts_source = last_accessed
                .clone()
                .or(created_at.clone())
                .unwrap_or_default();
            let ts = parse_timestamp_ms(&ts_source);
            let display = crate::aging::get_display_text(
                &text,
                &compressed_text,
                &age_tier.unwrap_or_else(|| "fresh".to_string()),
            );

            let haystacks = [
                text.to_lowercase(),
                source.unwrap_or_default().to_lowercase(),
                tags.unwrap_or_default().to_lowercase(),
            ];
            let mut matched = 0_i64;
            for token in &tokens {
                if haystacks.iter().any(|h| h.contains(token)) {
                    matched += 1;
                }
            }
            if matched == 0 {
                matched = 1;
            }

            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let recency_weight = 1.0 / (1.0 + recency_d as f64 / 7.0);
            let keyword_weight = matched as f64 / tokens.len() as f64;
            let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
            let score_weight = score.clamp(0.0, 1.0);
            let ranking = (keyword_weight * 0.40)
                + (score_weight * 0.25)
                + (recency_weight * 0.20)
                + (retrieval_weight * 0.15);

            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: truncate_chars(&display, 200),
                relevance: round4(ranking),
                matched_keywords: matched,
                score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }

        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
        });

        ranked.truncate(limit);
        Ok(ranked)
    })();

    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_memories_fallback(conn, query_text, limit),
    }
}

fn search_memories_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, text, source, tags, score, retrievals, last_accessed, created_at \
             FROM memories WHERE status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now'))",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<f64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let tokens = extract_search_keywords(query_text);
    let mut ranked = Vec::new();

    for row in rows.flatten() {
        let (id, text, source, tags, score, retrievals, last_accessed, created_at) = row;
        let source_key = source.clone().unwrap_or_else(|| format!("memory::{id}"));
        let score = score.unwrap_or(1.0).max(0.0);
        let ts_source = last_accessed
            .clone()
            .or(created_at.clone())
            .unwrap_or_default();
        let ts = parse_timestamp_ms(&ts_source);

        if tokens.is_empty() {
            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: truncate_chars(&text, 200),
                relevance: round4(0.5 * score),
                matched_keywords: 0,
                score,
                ts,
                owner_id: None,
                visibility: None,
            });
            continue;
        }

        let haystacks = [
            text.to_lowercase(),
            source.unwrap_or_default().to_lowercase(),
            tags.unwrap_or_default().to_lowercase(),
        ];

        let mut matched = 0_i64;
        for token in &tokens {
            if haystacks.iter().any(|h| h.contains(token)) {
                matched += 1;
            }
        }
        if matched == 0 {
            continue;
        }

        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let recency_weight = 1.0 / (1.0 + recency_d as f64 / 7.0);
        let keyword_weight = matched as f64 / tokens.len() as f64;
        let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
        let score_weight = score.clamp(0.0, 1.0);
        let ranking = (keyword_weight * 0.40)
            + (score_weight * 0.25)
            + (recency_weight * 0.20)
            + (retrieval_weight * 0.15);

        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: truncate_chars(&text, 200),
            relevance: round4(ranking),
            matched_keywords: matched,
            score,
            ts,
            owner_id: None,
            visibility: None,
        });
    }

    if tokens.is_empty() {
        ranked.sort_by(|a, b| b.ts.cmp(&a.ts));
    } else {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
        });
    }

    ranked.truncate(limit);
    Ok(ranked)
}

fn search_decisions(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
    let tokens = extract_search_keywords_with_synonyms(query_text);

    if tokens.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, decision, context, score, retrievals, last_accessed, created_at \
                 FROM decisions WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(SearchCandidate {
                    source: row.get::<_, Option<String>>(2)?.unwrap_or_else(|| {
                        format!("decision::{}", row.get::<_, i64>(0).unwrap_or(0))
                    }),
                    excerpt: truncate_chars(&row.get::<_, String>(1)?, 200),
                    relevance: round4(0.5 * row.get::<_, Option<f64>>(3)?.unwrap_or(1.0).max(0.0)),
                    matched_keywords: 0,
                    score: row.get::<_, Option<f64>>(3)?.unwrap_or(1.0).max(0.0),
                    ts: parse_timestamp_ms(
                        &row.get::<_, Option<String>>(5)?
                            .or(row.get::<_, Option<String>>(6)?)
                            .unwrap_or_default(),
                    ),
                    owner_id: None,
                    visibility: None,
                })
            })
            .map_err(|e| e.to_string())?;

        return Ok(rows.flatten().collect());
    }

    let fts_query = tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");

    let fts_result: Result<Vec<SearchCandidate>, String> = (|| {
        // Field-boosted BM25: decisions_fts columns are (decision, context).
        // Weights: decision=5.0, context=1.0 -- the decision text is primary signal;
        // context is the source/label string and lower priority.
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.decision, d.context, d.score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility \
                 FROM decisions_fts fts \
                 JOIN decisions d ON d.id = fts.rowid \
                 WHERE decisions_fts MATCH ?1 AND d.status = 'active' \
                 AND (d.expires_at IS NULL OR d.expires_at > datetime('now')) \
                 ORDER BY bm25(decisions_fts, 5.0, 1.0) \
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![&fts_query, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<i64>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        let mut ranked = Vec::new();
        for row in rows.flatten() {
            let (
                id,
                decision,
                context,
                score,
                retrievals,
                last_accessed,
                created_at,
                compressed_text,
                age_tier,
                row_owner_id,
                row_visibility,
            ) = row;
            let source_key = context.clone().unwrap_or_else(|| format!("decision::{id}"));
            let score = score.unwrap_or(1.0).max(0.0);
            let ts_source = last_accessed
                .clone()
                .or(created_at.clone())
                .unwrap_or_default();
            let ts = parse_timestamp_ms(&ts_source);
            let display = crate::aging::get_display_text(
                &decision,
                &compressed_text,
                &age_tier.unwrap_or_else(|| "fresh".to_string()),
            );

            let haystacks = [
                decision.to_lowercase(),
                context.unwrap_or_default().to_lowercase(),
            ];
            let mut matched = 0_i64;
            for token in &tokens {
                if haystacks.iter().any(|h| h.contains(token)) {
                    matched += 1;
                }
            }
            if matched == 0 {
                matched = 1;
            }

            let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
            let recency_weight = 1.0 / (1.0 + recency_d as f64 / 7.0);
            let keyword_weight = matched as f64 / tokens.len() as f64;
            let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
            let score_weight = score.clamp(0.0, 1.0);
            let ranking = (keyword_weight * 0.40)
                + (score_weight * 0.25)
                + (recency_weight * 0.20)
                + (retrieval_weight * 0.15);

            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: truncate_chars(&display, 200),
                relevance: round4(ranking),
                matched_keywords: matched,
                score,
                ts,
                owner_id: row_owner_id,
                visibility: row_visibility,
            });
        }

        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
        });

        ranked.truncate(limit);
        Ok(ranked)
    })();

    match fts_result {
        Ok(results) if !results.is_empty() => Ok(results),
        _ => search_decisions_fallback(conn, query_text, limit),
    }
}

fn search_decisions_fallback(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, decision, context, score, retrievals, last_accessed, created_at \
             FROM decisions WHERE status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now'))",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let tokens = extract_search_keywords(query_text);
    let mut ranked = Vec::new();

    for row in rows.flatten() {
        let (id, decision, context, score, retrievals, last_accessed, created_at) = row;
        let source_key = context.clone().unwrap_or_else(|| format!("decision::{id}"));
        let score = score.unwrap_or(1.0).max(0.0);
        let ts_source = last_accessed
            .clone()
            .or(created_at.clone())
            .unwrap_or_default();
        let ts = parse_timestamp_ms(&ts_source);

        if tokens.is_empty() {
            ranked.push(SearchCandidate {
                source: source_key,
                excerpt: truncate_chars(&decision, 200),
                relevance: round4(0.5 * score),
                matched_keywords: 0,
                score,
                ts,
                owner_id: None,
                visibility: None,
            });
            continue;
        }

        let haystacks = [
            decision.to_lowercase(),
            context.unwrap_or_default().to_lowercase(),
        ];
        let mut matched = 0_i64;
        for token in &tokens {
            if haystacks.iter().any(|h| h.contains(token)) {
                matched += 1;
            }
        }
        if matched == 0 {
            continue;
        }

        let recency_d = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
        let recency_weight = 1.0 / (1.0 + recency_d as f64 / 7.0);
        let keyword_weight = matched as f64 / tokens.len() as f64;
        let retrieval_weight = (retrievals.unwrap_or(0).clamp(0, 20) as f64) / 20.0;
        let score_weight = score.clamp(0.0, 1.0);
        let ranking = (keyword_weight * 0.40)
            + (score_weight * 0.25)
            + (recency_weight * 0.20)
            + (retrieval_weight * 0.15);

        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: truncate_chars(&decision, 200),
            relevance: round4(ranking),
            matched_keywords: matched,
            score,
            ts,
            owner_id: None,
            visibility: None,
        });
    }

    if tokens.is_empty() {
        ranked.sort_by(|a, b| b.ts.cmp(&a.ts));
    } else {
        ranked.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.matched_keywords.cmp(&a.matched_keywords))
                .then(
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.ts.cmp(&a.ts))
        });
    }

    ranked.truncate(limit);
    Ok(ranked)
}

fn collect_semantic_candidates(
    conn: &Connection,
    query_vector: &[f32],
    ctx: &RecallContext,
) -> Vec<SemanticCandidate> {
    let scale_sim = |sim: f32| -> f64 {
        SEMANTIC_SCALE_BASE
            + (sim as f64 - SEMANTIC_SIM_FLOOR)
                * ((1.0 - SEMANTIC_SCALE_BASE) / (1.0 - SEMANTIC_SIM_FLOOR))
    };

    let mut candidates: HashMap<String, SemanticCandidate> = HashMap::new();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT e.vector, m.text, m.source, m.owner_id, m.visibility \
         FROM embeddings e \
         JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active' \
         AND (m.expires_at IS NULL OR m.expires_at > datetime('now'))",
    ) {
        let rows: Vec<MemorySemanticRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        for (blob, text, source, owner_id, visibility) in rows {
            if !is_visible(owner_id, visibility.as_deref(), ctx) {
                continue;
            }
            let existing_vec = crate::embeddings::blob_to_vector(&blob);
            let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
            if sim <= SEMANTIC_SIM_FLOOR as f32 {
                continue;
            }

            let scaled = scale_sim(sim);
            let entry = candidates.entry(source.clone()).or_insert(SemanticCandidate {
                source,
                excerpt: text.chars().take(200).collect(),
                relevance: scaled,
            });
            if scaled > entry.relevance {
                *entry = SemanticCandidate {
                    source: entry.source.clone(),
                    excerpt: text.chars().take(200).collect(),
                    relevance: scaled,
                };
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT e.vector, d.decision, d.context, d.owner_id, d.visibility \
         FROM embeddings e \
         JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active' \
         AND (d.expires_at IS NULL OR d.expires_at > datetime('now'))",
    ) {
        let rows: Vec<DecisionSemanticRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|r| r.ok())
            .collect();

        for (blob, decision, context, owner_id, visibility) in rows {
            if !is_visible(owner_id, visibility.as_deref(), ctx) {
                continue;
            }
            let existing_vec = crate::embeddings::blob_to_vector(&blob);
            let sim = crate::embeddings::cosine_similarity(query_vector, &existing_vec);
            if sim <= SEMANTIC_SIM_FLOOR as f32 {
                continue;
            }

            let source = context.unwrap_or_else(|| {
                format!("decision::{}", decision.chars().take(40).collect::<String>())
            });
            let scaled = scale_sim(sim);
            let excerpt = decision.chars().take(200).collect::<String>();
            let entry = candidates.entry(source.clone()).or_insert(SemanticCandidate {
                source,
                excerpt: excerpt.clone(),
                relevance: scaled,
            });
            if scaled > entry.relevance {
                *entry = SemanticCandidate {
                    source: entry.source.clone(),
                    excerpt,
                    relevance: scaled,
                };
            }
        }
    }

    let mut sorted: Vec<SemanticCandidate> = candidates.into_values().collect();
    sorted.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

// ─── Text / keyword utilities ────────────────────────────────────────────────

fn normalize_text(input: &str) -> String {
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

fn extract_keywords(text: &str) -> Vec<String> {
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

fn extract_search_keywords(text: &str) -> Vec<String> {
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
fn coding_synonyms(word: &str) -> Option<&'static str> {
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
        _ => None,
    }
}

/// Like `extract_search_keywords` but also expands coding synonyms.
/// Each token that has a known synonym produces both the original and the expanded form.
/// Deduplicates the final list while preserving order.
fn extract_search_keywords_with_synonyms(text: &str) -> Vec<String> {
    let base = extract_search_keywords(text);
    let mut seen = HashSet::new();
    let mut result = Vec::with_capacity(base.len() * 2);
    for word in base {
        if seen.insert(word.clone()) {
            if let Some(expanded) = coding_synonyms(&word) {
                // Insert expanded form first (higher signal), then original
                let exp_str = expanded.to_string();
                if seen.insert(exp_str.clone()) {
                    result.push(exp_str);
                }
            }
            result.push(word);
        }
    }
    result
}

fn parse_timestamp_ms(value: &str) -> i64 {
    if value.trim().is_empty() {
        return 0;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    0
}

fn recency_days(value: Option<&str>) -> i64 {
    let ts = value.map(parse_timestamp_ms).unwrap_or(0);
    if ts == 0 {
        return 3650;
    }
    (Utc::now().timestamp_millis() - ts).max(0) / (24 * 60 * 60 * 1000)
}

fn round4(value: f64) -> f64 {
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
fn bump_retrievals_batch(conn: &Connection, items: &[RecallItem]) {
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

fn recall_to_json(item: RecallItem) -> Value {
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
    }
    payload
}

// ─── Content dedup / served tracking ─────────────────────────────────────────

fn hash_content(content: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for ch in content.chars().take(100) {
        hash ^= ch as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

/// Content served within this window is suppressed to avoid echo in rapid
/// successive recalls. After this TTL, the same content can be re-served.
const SERVED_TTL_MS: i64 = 60_000; // 60 seconds

async fn dedup_and_mark_served(
    state: &RuntimeState,
    agent: &str,
    query: &str,
    results: Vec<RecallItem>,
) -> Vec<RecallItem> {
    if results.is_empty() {
        return results;
    }

    let now = Utc::now().timestamp_millis();
    let mut served = state.served_content.lock().await;
    let map = served
        .entry(served_content_scope(agent, query))
        .or_insert_with(HashMap::<u32, i64>::new);

    // Evict expired entries
    map.retain(|_, ts| now - *ts < SERVED_TTL_MS);

    let mut filtered = Vec::new();
    for result in results {
        let hash = hash_content(&result.excerpt);
        if map.contains_key(&hash) {
            continue;
        }
        map.insert(hash, now);
        filtered.push(result);
    }

    filtered
}

fn served_content_scope(agent: &str, query: &str) -> String {
    let normalized_query = query
        .split_whitespace()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{agent}::{normalized_query}")
}

// ─── Recall pattern tracking / pre-cache ─────────────────────────────────────

async fn record_recall_pattern(state: &RuntimeState, agent: &str, query: &str) {
    let mut history = state.recall_history.lock().await;
    let entries = history
        .entry(agent.to_string())
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
const JACCARD_FUZZY_THRESHOLD: f64 = 0.6;

async fn get_pre_cached(state: &RuntimeState, agent: &str, query: &str) -> Option<Vec<RecallItem>> {
    let mut cache = state.pre_cache.lock().await;
    let now = Utc::now().timestamp_millis();

    // Tier 0: exact match for this agent
    if let Some(entry) = cache.get(agent) {
        if entry.query == query && entry.expires_at > now {
            return deserialize_cache_entry(&entry.results);
        }
    }

    // Evict expired entry for this agent
    if cache
        .get(agent)
        .map(|e| e.expires_at <= now)
        .unwrap_or(false)
    {
        cache.remove(agent);
    }

    // Tier 1: fuzzy Jaccard match across all cached entries (any agent)
    let mut best_score = 0.0_f64;
    let mut best_key: Option<String> = None;
    for (key, entry) in cache.iter() {
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

fn deserialize_cache_entry(results: &serde_json::Value) -> Option<Vec<RecallItem>> {
    let arr = results.as_array()?;
    let items: Vec<RecallItem> = arr
        .iter()
        .filter_map(|v| {
            Some(RecallItem {
                source: v.get("source")?.as_str()?.to_string(),
                relevance: v.get("relevance")?.as_f64()?,
                excerpt: v.get("excerpt")?.as_str()?.to_string(),
                method: v.get("method")?.as_str()?.to_string(),
                tokens: v.get("tokens").and_then(|t| t.as_u64()).map(|t| t as usize),
                entropy: v.get("entropy").and_then(|e| e.as_f64()),
            })
        })
        .collect();
    Some(items)
}

async fn predict_and_cache(
    state: RuntimeState,
    agent: &str,
    current_query: &str,
) -> Result<(), String> {
    let predicted_query = {
        let history = state.recall_history.lock().await;
        let entries = match history.get(agent) {
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

    let predict_ctx = RecallContext::solo();
    let mut conn = state.db.lock().await;
    let results = run_budget_recall(&mut conn, &predicted_query, 200, 5, &predict_ctx)?;
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
        agent.to_string(),
        PreCacheEntry {
            query: predicted_query,
            results: results_json,
            expires_at: now_ms + PRECACHE_TTL_MS,
        },
    );
    Ok(())
}

// ─── GET /unfold ────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct UnfoldQuery {
    pub sources: Option<String>,
}

/// Unfold specific items by source string. Returns full text for each requested
/// source without re-running search. Designed for progressive disclosure:
/// peek (headlines) → unfold (full text of selected items).
pub async fn handle_unfold(
    State(state): State<RuntimeState>,
    Query(query): Query<UnfoldQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id = match ensure_auth_with_caller(&headers, &state) {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let ctx = RecallContext::from_caller(caller_id, &state);
    let sources_str = match &query.sources {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "Missing query parameter: sources (comma-separated)"}),
            );
        }
    };

    let requested: Vec<&str> = sources_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if requested.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "No valid sources provided"}),
        );
    }

    let conn = state.db.lock().await;
    let mut results: Vec<Value> = Vec::new();
    let mut total_tokens = 0usize;

    for source in &requested {
        if let Some(item) = unfold_source(&conn, source, &ctx) {
            let tokens = estimate_tokens(item["text"].as_str().unwrap_or(""));
            total_tokens += tokens;
            results.push(json!({
                "source": source,
                "text": item["text"],
                "type": item["type"],
                "tokens": tokens,
            }));
        } else {
            results.push(json!({
                "source": source,
                "text": null,
                "type": "not_found",
                "tokens": 0,
            }));
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "results": results,
            "totalTokens": total_tokens,
            "count": results.iter().filter(|r| r["type"] != "not_found").count(),
        }),
    )
}

/// Look up the full text of a single source string (team visibility applied when `ctx.team_mode`).
pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Ok((text, ty, owner_id, visibility)) = conn.query_row(
        "SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) ORDER BY score DESC LIMIT 1",
        params![source],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    ) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text": text, "type": ty}));
        }
    }

    if let Some(id_str) = source.strip_prefix("decision::") {
        if let Ok(id) = id_str.parse::<i64>() {
            if let Ok((decision, context, owner_id, visibility)) = conn.query_row(
                "SELECT decision, context, owner_id, visibility FROM decisions WHERE id = ?1 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            ) {
                if is_visible(owner_id, visibility.as_deref(), ctx) {
                    let full = match context {
                        Some(c) => format!("{decision}\n\nContext: {c}"),
                        None => decision,
                    };
                    return Some(json!({"text": full, "type": "decision"}));
                }
            }
        }
    }

    if let Ok((decision, context, owner_id, visibility)) = conn.query_row(
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE context = ?1 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) ORDER BY score DESC LIMIT 1",
        params![source],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    ) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            let full = match context {
                Some(c) => format!("{decision}\n\nContext: {c}"),
                None => decision,
            };
            return Some(json!({"text": full, "type": "decision"}));
        }
    }

    let stripped = source.strip_prefix("memory::").unwrap_or(source);
    if let Ok((text, ty, owner_id, visibility)) = conn.query_row(
        "SELECT text, type, owner_id, visibility FROM memories WHERE source LIKE ?1 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) ORDER BY score DESC LIMIT 1",
        params![format!("%{stripped}%")],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    ) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text": text, "type": ty}));
        }
    }

    None
}

// ─── Jaccard keyword similarity ──────────────────────────────────────────────

/// Jaccard similarity on whitespace-tokenized keyword sets.
///
/// Returns |A ∩ B| / |A ∪ B|.  Returns 0.0 for empty inputs.
/// Used for Tier-1 fuzzy cache matching: queries with >= 0.6 Jaccard similarity
/// are considered close enough to reuse cached results.
fn jaccard_similarity(a: &str, b: &str) -> f64 {
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

/// Reciprocal Rank Fusion (Cormack et al., 2009).
///
/// Fuses multiple ranked lists into a single list using the formula:
///   score(item) = Σ  1 / (k + rank + 1)   for each list containing item
///
/// `k = 60.0` is the standard value from the original paper.
/// Items only in one list still accumulate their 1/(k+1) score.
/// Returns results sorted by fused score descending.
///
/// # Arguments
/// * `lists` -- slice of ranked lists, each a `Vec<(id, score)>` in descending score order
/// * `k`     -- smoothing constant (use `60.0` per Cormack et al.)
///
fn rrf_fuse(lists: &[Vec<(i64, f64)>], k: f64) -> Vec<(i64, f64)> {
    let mut fused: HashMap<i64, f64> = HashMap::new();
    for list in lists {
        for (rank, &(id, _score)) in list.iter().enumerate() {
            *fused.entry(id).or_insert(0.0) += 1.0 / (k + rank as f64 + 1.0);
        }
    }
    let mut result: Vec<(i64, f64)> = fused.into_iter().collect();
    result.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    result
}

// ─── Compound scoring (Task 1.4) ─────────────────────────────────────────────

/// Calculate elapsed days since an ISO 8601 timestamp.
/// Returns days as f64, handling invalid timestamps gracefully (returns very large value).
fn days_since(created_at: &str) -> f64 {
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
/// Scores are typically 0-100 from spaced repetition; we normalize by dividing by 100.
/// Also clamps to stay within bounds.
fn normalize(importance: f64) -> f64 {
    let normalized = importance / 100.0;
    normalized.clamp(0.0, 1.0)
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
fn compound_score(rrf: f64, importance: f64, created_at: &str) -> f64 {
    let days = days_since(created_at);
    let recency = (-days / 30.0).exp(); // 21-day half-life
    let importance_normalized = normalize(importance);
    rrf * 0.6 + importance_normalized * 0.2 + recency * 0.2
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding};
    use rusqlite::params;

    // ── is_visible tests ───────────────────────────────────────────

    fn solo_ctx() -> RecallContext {
        RecallContext {
            caller_id: None,
            team_mode: false,
        }
    }
    fn team_ctx(caller: i64) -> RecallContext {
        RecallContext {
            caller_id: Some(caller),
            team_mode: true,
        }
    }
    fn team_ctx_no_caller() -> RecallContext {
        RecallContext {
            caller_id: None,
            team_mode: true,
        }
    }

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn insert_memory_with_embedding(
        conn: &rusqlite::Connection,
        text: &str,
        source: &str,
        vector: &[f32],
    ) -> i64 {
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params![text, source],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model)
             VALUES ('memory', ?1, ?2, 'test-model')",
            params![id, crate::embeddings::vector_to_blob(vector)],
        )
        .unwrap();
        id
    }

    fn store_decision_with_embedding(
        conn: &mut rusqlite::Connection,
        decision: &str,
        context: &str,
        vector: &[f32],
    ) {
        let (_, new_id) = store_decision_with_input_embedding(
            conn,
            decision,
            Some(context.to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            Some(vector),
            None,
        )
        .unwrap();

        if let Some(id) = new_id {
            persist_decision_embedding(conn, id, vector).unwrap();
        }
    }

    #[test]
    fn search_memories_excludes_expired_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('expired memory', 'note', 'expired-memory', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('active memory', 'note', 'active-memory', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_memories(&conn, "", 10).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();

        assert!(sources.contains(&"active-memory"));
        assert!(!sources.contains(&"expired-memory"));
    }

    #[test]
    fn search_decisions_excludes_expired_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired decision', 'expired-decision', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('active decision', 'active-decision', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_decisions(&conn, "", 10).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();

        assert!(sources.contains(&"active-decision"));
        assert!(!sources.contains(&"expired-decision"));
    }

    #[test]
    fn store_then_keyword_recall_ranks_expected_entry_first() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "Run a WAL checkpoint before daily backup rotation during daemon startup.",
            "memory::wal-checkpoint",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use rtk cargo clippy -- -D warnings so CI fails on every warning.",
            "decision::clippy-gate",
            &[0.0, 1.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use the expect skill for screenshot QA and breakpoint comparisons on the dashboard.",
            "decision::expect-skill",
            &[0.0, 0.0, 1.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Keep three recent backups and delete older cortex database snapshots on startup.",
            "decision::backup-retention",
            &[0.0, 0.0, 0.0, 1.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Truncate write_buffer.jsonl after buffered entries flush into SQLite.",
            "decision::write-buffer",
            &[0.0, 0.0, 0.0, 0.0, 1.0],
        );

        let results = run_budget_recall(&mut conn, "write buffer", 400, 5, &solo_ctx()).unwrap();

        assert!(!results.is_empty());
        assert_eq!(
            results[0].source,
            "decision::write-buffer",
            "unexpected keyword ranking: {:?}",
            results
                .iter()
                .map(|item| item.source.clone())
                .collect::<Vec<_>>()
        );
        assert!(matches!(results[0].method.as_str(), "keyword" | "hybrid"));
    }

    #[test]
    fn store_then_semantic_recall_keeps_expected_entry_in_top_three() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "Run a WAL checkpoint before daily backup rotation during daemon startup.",
            "memory::wal-checkpoint",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use rtk cargo clippy -- -D warnings so CI fails on every warning.",
            "decision::clippy-gate",
            &[0.0, 1.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use the expect skill for screenshot QA and breakpoint comparisons on the dashboard.",
            "decision::expect-skill",
            &[0.0, 0.0, 1.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Keep three recent backups and delete older cortex database snapshots on startup.",
            "decision::backup-retention",
            &[0.0, 0.0, 0.0, 1.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Truncate write_buffer.jsonl after buffered entries flush into SQLite.",
            "decision::write-buffer",
            &[0.0, 0.0, 0.0, 0.0, 1.0],
        );

        let results = run_budget_recall_with_engine(
            &mut conn,
            "aurora lattice signal",
            400,
            5,
            None,
            &solo_ctx(),
            None,
        )
        .unwrap();
        assert!(results.is_empty(), "keyword-only path should not match");

        let embedding_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(embedding_count, 5);

        let expect_context: String = conn
            .query_row(
                "SELECT context FROM decisions WHERE context = 'decision::expect-skill'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expect_context, "decision::expect-skill");

        let expect_blob: Vec<u8> = conn
            .query_row(
                "SELECT e.vector
                 FROM embeddings e
                 JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id
                 WHERE d.context = 'decision::expect-skill'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let expect_similarity = crate::embeddings::cosine_similarity(
            &[0.0, 0.0, 1.0, 0.0, 0.0],
            &crate::embeddings::blob_to_vector(&expect_blob),
        );
        assert!(expect_similarity > 0.99);

        let decision_embedding_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM embeddings e
                 JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decision_embedding_rows, 4);

        let mut manual_semantic_ranking: Vec<(String, f32)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT e.vector, d.context
                     FROM embeddings e
                     JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id
                     WHERE d.status = 'active'",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap()
            .filter_map(|row| row.ok())
            .filter_map(|(blob, context)| {
                let similarity = crate::embeddings::cosine_similarity(
                    &[0.0, 0.0, 1.0, 0.0, 0.0],
                    &crate::embeddings::blob_to_vector(&blob),
                );
                (similarity > 0.3).then_some((context.unwrap_or_default(), similarity))
            })
            .collect()
        };
        manual_semantic_ranking
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        assert!(
            manual_semantic_ranking
                .iter()
                .any(|(source, _)| source == "decision::expect-skill"),
            "expected semantic candidates to include target, got {:?}",
            manual_semantic_ranking
        );
        let position = manual_semantic_ranking
            .iter()
            .position(|(source, _)| source == "decision::expect-skill")
            .unwrap_or_else(|| {
                panic!(
                    "expected semantic target to be recalled, got {:?}",
                    manual_semantic_ranking
                )
            });
        assert!(
            position < 3,
            "expected top-3 semantic rank, got {}",
            position + 1
        );
        assert_eq!(
            manual_semantic_ranking[position].0,
            "decision::expect-skill"
        );
    }

    #[test]
    fn is_visible_solo_mode_always_true() {
        let ctx = solo_ctx();
        assert!(is_visible(None, None, &ctx));
        assert!(is_visible(Some(1), Some("private"), &ctx));
        assert!(is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_owner_sees_own() {
        let ctx = team_ctx(1);
        assert!(is_visible(Some(1), Some("private"), &ctx));
        assert!(is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_shared_visible_to_other() {
        let ctx = team_ctx(2);
        assert!(is_visible(Some(1), Some("shared"), &ctx));
        assert!(is_visible(Some(1), Some("team"), &ctx));
    }

    #[test]
    fn is_visible_team_private_hidden_from_other() {
        let ctx = team_ctx(2);
        assert!(!is_visible(Some(1), Some("private"), &ctx));
        assert!(!is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_none_caller_denied() {
        let ctx = team_ctx_no_caller();
        assert!(!is_visible(Some(1), Some("private"), &ctx));
        assert!(!is_visible(Some(1), Some("shared"), &ctx));
        assert!(!is_visible(None, None, &ctx));
    }

    #[test]
    fn is_visible_team_none_owner_denied() {
        let ctx = team_ctx(1);
        assert!(!is_visible(None, Some("shared"), &ctx));
        assert!(!is_visible(None, None, &ctx));
    }

    // ── existing tests ─────────────────────────────────────────────

    #[test]
    fn test_shannon_entropy_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn test_shannon_entropy_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn test_shannon_entropy_two_equal_chars() {
        let h = shannon_entropy("ab");
        assert!((h - 1.0).abs() < 0.001, "expected ~1.0, got {h}");
    }

    #[test]
    fn test_shannon_entropy_english_prose_range() {
        let prose = "The quick brown fox jumps over the lazy dog near the riverbank";
        let h = shannon_entropy(prose);
        assert!(
            h > 3.5 && h < 5.0,
            "english prose entropy {h} outside expected 3.5-5.0"
        );
    }

    #[test]
    fn test_shannon_entropy_boilerplate_lower() {
        let boilerplate = "aaabbbccc aaabbbccc aaabbbccc";
        let prose = "The zephyr-cache module uses LRU eviction with a 512-entry cap";
        assert!(shannon_entropy(boilerplate) < shannon_entropy(prose));
    }

    #[test]
    fn test_hash_content_deterministic() {
        assert_eq!(hash_content("test content"), hash_content("test content"));
    }

    #[test]
    fn test_hash_content_different() {
        assert_ne!(hash_content("content a"), hash_content("content b"));
    }

    #[test]
    fn test_extract_keywords_filters_stopwords() {
        let kw = extract_keywords("the quick brown fox jumps over a lazy dog");
        assert!(kw.contains(&"quick".to_string()));
        assert!(kw.contains(&"brown".to_string()));
        assert!(!kw.contains(&"the".to_string()));
        assert!(!kw.contains(&"an".to_string()));
    }

    #[test]
    fn test_extract_keywords_filters_short() {
        let kw = extract_keywords("go to db");
        assert!(kw.is_empty());
    }

    #[test]
    fn test_extract_search_keywords_keeps_short() {
        let kw = extract_search_keywords("go to db");
        assert!(kw.contains(&"go".to_string()));
        assert!(kw.contains(&"db".to_string()));
    }

    #[test]
    fn test_round4() {
        assert_eq!(round4(0.12345), 0.1235);
        assert_eq!(round4(1.0), 1.0);
    }

    // ── RRF fusion tests ───────────────────────────────────────────

    #[test]
    fn test_rrf_fuse_single_list() {
        // Single list: ranks 0,1,2 with k=60
        let list = vec![(10, 0.9), (20, 0.7), (30, 0.5)];
        let result = rrf_fuse(&[list], 60.0);
        assert_eq!(result.len(), 3);
        // Item at rank 0 should be first (highest fused score)
        assert_eq!(result[0].0, 10);
        assert_eq!(result[1].0, 20);
        assert_eq!(result[2].0, 30);
        // Score for rank-0 item: 1/(60+0+1) = 1/61
        let expected = 1.0 / 61.0;
        assert!(
            (result[0].1 - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            result[0].1
        );
    }

    #[test]
    fn test_rrf_fuse_two_lists_agreement() {
        // Item 10 is rank-0 in both lists -- should score highest
        let list_a = vec![(10, 0.9), (20, 0.5)];
        let list_b = vec![(10, 0.8), (30, 0.4)];
        let result = rrf_fuse(&[list_a, list_b], 60.0);
        assert_eq!(result[0].0, 10);
        // Score = 1/(60+0+1) + 1/(60+0+1) = 2/61
        let expected = 2.0 / 61.0;
        assert!((result[0].1 - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_fuse_promotes_consistent_middle() {
        // Verify RRF correctly weights cross-list agreement vs single-list high rank.
        //
        // list_a = [(10,_), (20,_), (30,_)]: rank0=10, rank1=20, rank2=30
        // list_b = [(30,_), (20,_)]:          rank0=30, rank1=20
        //
        // RRF scores (k=60):
        //   item10: 1/(60+0+1)           = 1/61  ≈ 0.016393
        //   item20: 1/(60+1+1)+1/(60+1+1) = 2/62  ≈ 0.032258
        //   item30: 1/(60+2+1)+1/(60+0+1) = 1/63+1/61 ≈ 0.032266
        //
        // item30 beats item20 by 0.000008 (rank-0 bonus in list_b outweighs
        // rank-2 penalty in list_a vs rank-1 in both for item20).
        // Both item20 and item30 score ~2x item10 (cross-list agreement crushes lone rank-0).
        let list_a = vec![(10, 0.9), (20, 0.6), (30, 0.2)];
        let list_b = vec![(30, 0.8), (20, 0.5)];
        let result = rrf_fuse(&[list_a, list_b], 60.0);
        assert_eq!(result.len(), 3);

        // item 10 (only in list_a at rank 0) should be last -- single-list penalty
        let pos_10 = result.iter().position(|(id, _)| *id == 10).unwrap();
        let pos_20 = result.iter().position(|(id, _)| *id == 20).unwrap();
        let pos_30 = result.iter().position(|(id, _)| *id == 30).unwrap();
        assert!(
            pos_10 > pos_20,
            "item10 (rank-0 in one list) should lose to item20 (rank-1 in both)"
        );
        assert!(
            pos_10 > pos_30,
            "item10 (rank-0 in one list) should lose to item30 (rank-0 + rank-2)"
        );

        // Both multi-list items score well above single-list item10
        let score_10 = result[pos_10].1;
        let score_20 = result[pos_20].1;
        assert!(
            score_20 > score_10 * 1.9,
            "item20 cross-list score should be ~2x item10"
        );
    }

    #[test]
    fn test_rrf_fuse_empty_lists() {
        let result = rrf_fuse(&[], 60.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rrf_fuse_single_empty_list() {
        let result = rrf_fuse(&[vec![]], 60.0);
        assert!(result.is_empty());
    }

    // ── compound scoring tests (Task 1.4) ──────────────────────────

    #[test]
    fn test_days_since() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let days_today = days_since(&today);

        // Today should be very close to 0 (within 1 minute tolerance)
        assert!(
            days_today < 0.001,
            "days_since(today) should be ~0, got {}",
            days_today
        );

        //Yesterday (approximately)
        let yesterday = (now - chrono::Duration::days(1)).to_rfc3339();
        let days_yesterday = days_since(&yesterday);
        assert!(
            (days_yesterday - 1.0).abs() < 0.02,
            "days_since(yesterday) should be ~1.0, got {}",
            days_yesterday
        );

        // Invalid timestamp should return MAX
        let days_invalid = days_since("invalid-date");
        assert_eq!(
            days_invalid,
            f64::MAX,
            "days_since(invalid) should return MAX"
        );
    }

    #[test]
    fn test_normalize() {
        // Typical range: 0-100
        assert!((normalize(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((normalize(50.0) - 0.5).abs() < f64::EPSILON);
        assert!((normalize(100.0) - 1.0).abs() < f64::EPSILON);

        // Clamp above 100
        assert_eq!(normalize(150.0), 1.0);

        // Clamp below 0
        assert_eq!(normalize(-10.0), 0.0);
    }

    #[test]
    fn test_compound_score() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let week_ago = (now - chrono::Duration::weeks(1)).to_rfc3339();
        let month_ago = (now - chrono::Duration::days(30)).to_rfc3339();

        // High RRF, high importance, recent: should score well
        let score_high = compound_score(0.1, 100.0, &today);
        assert!(
            score_high > 0.06,
            "high RRF + high importance + recent should score well, got {}",
            score_high
        );

        // Low RRF, low importance, old: should score poorly (recency factor dominates but is low for old items)
        let score_low = compound_score(0.001, 0.0, &month_ago);
        assert!(
            score_low < 0.08,
            "low RRF + low importance + old should score poorly, got {}",
            score_low
        );

        // Recency decay: same RRF/imp, older date = lower score
        let score_today = compound_score(0.05, 50.0, &today);
        let score_week = compound_score(0.05, 50.0, &week_ago);
        assert!(
            score_today > score_week,
            "same RRF/imp, today should score > week ago"
        );
    }

    // ── synonym expansion tests ────────────────────────────────────

    #[test]
    fn test_synonym_expansion_func() {
        let kw = extract_search_keywords_with_synonyms("func error db");
        assert!(kw.contains(&"function".to_string()), "func -> function");
        assert!(kw.contains(&"error".to_string()));
        assert!(kw.contains(&"database".to_string()), "db -> database");
    }

    #[test]
    fn test_synonym_expansion_no_duplicates() {
        // "function" is already full form -- should not duplicate
        let kw = extract_search_keywords_with_synonyms("function");
        let count = kw.iter().filter(|w| *w == "function").count();
        assert_eq!(count, 1, "no duplicate expansions");
    }

    // ── query cache tests ──────────────────────────────────────────

    #[test]
    fn test_jaccard_similarity_identical() {
        let score = jaccard_similarity("rust error handling", "rust error handling");
        assert!((score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let score = jaccard_similarity("apple orange", "banana grape");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // "rust error" vs "rust warning" -- 1 shared ("rust"), 3 total -> 1/3
        let score = jaccard_similarity("rust error", "rust warning");
        assert!((score - 1.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_above_threshold() {
        // "recall pipeline rrf fusion" vs "recall rrf pipeline" -- 3 shared, 4 total -> 0.75 >= 0.6
        let score = jaccard_similarity("recall pipeline rrf fusion", "recall rrf pipeline");
        assert!(score >= 0.6, "expected >= 0.6, got {score}");
    }
}
