use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use super::{ensure_auth, ensure_auth_with_caller};
use super::{estimate_tokens, json_response, now_iso, truncate_chars};
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::state::{PreCacheEntry, RecallHistoryEntry, RuntimeState};

// ─── Constants ───────────────────────────────────────────────────────────────

const MAX_RECALL_HISTORY: usize = 50;
const PRECACHE_TTL_MS: i64 = 5 * 60 * 1000;

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
/// Team mode: owner sees own; shared/team visible to all; private hidden from non-owners.
/// Unowned data (owner_id=None, e.g. pre-migration rows or fallback paths) is visible to all.
fn is_visible(owner_id: Option<i64>, visibility: Option<&str>, ctx: &RecallContext) -> bool {
    if !ctx.team_mode || ctx.caller_id.is_none() {
        return true;
    }
    if owner_id.is_none() {
        return true;
    }
    let caller = ctx.caller_id.unwrap();
    if owner_id == Some(caller) {
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
    let agent = query
        .agent
        .or_else(|| {
            headers
                .get("x-source-agent")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "http".to_string());

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
            )
        }
    };

    let budget = query.budget.unwrap_or(300);
    let k = query.k.unwrap_or(10);

    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    let engine = state.embedding_engine.as_deref();
    match run_budget_recall_with_engine(&mut conn, &q, budget, k, engine, &ctx, Some(&state.degraded_mode)) {
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
            )
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
            let deduped_cached = dedup_and_mark_served(state, agent, cached).await;
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
        return Ok(json!({
            "count": headlines.len(),
            "results": headlines,
            "budget": 0,
            "spent": 0,
            "mode": "headlines"
        }));
    }

    // Dedup and budget accounting
    let results = dedup_and_mark_served(state, agent, results).await;
    let spent: usize = results
        .iter()
        .map(|item| {
            item.tokens
                .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
        })
        .sum();
    let saved = budget as i64 - spent as i64;

    let payload = json!({
        "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": spent,
        "saved": saved,
        "mode": if budget >= 500 { "full" } else { "balanced" }
    });

    Ok(payload)
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

fn run_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let extracted = extract_keywords(query_text);
    let keyword_query = if extracted.is_empty() {
        query_text.to_string()
    } else {
        extracted.join(" ")
    };

    let mut merged: HashMap<String, RecallItem> = HashMap::new();

    // ── Pass 1: Semantic search via embeddings (if available) ────────────────
    // Cosine sims land in ~[0.3, 0.8] while keyword scores are ~[0.5, 1.0].
    // Rescale so semantic results compete fairly: 0.3 → 0.55, 1.0 → 1.0.
    const SIM_FLOOR: f64 = 0.3;
    const SCALE_BASE: f64 = 0.55;
    let scale_sim = |sim: f32| -> f64 {
        SCALE_BASE + (sim as f64 - SIM_FLOOR) * ((1.0 - SCALE_BASE) / (1.0 - SIM_FLOOR))
    };

    if let Some(engine) = engine {
        if let Some(query_vec) = engine.embed(query_text) {
            // ── Pass 0: Crystal search (highest priority, visibility-aware) ─
            for (crystal_id, label, text, relevance) in
                crate::crystallize::search_crystals_filtered(conn, &query_vec, 3, ctx.caller_id, ctx.team_mode)
            {
                let source = format!("crystal::{crystal_id}::{label}");
                merged.insert(
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

            // Search memory embeddings (includes owner_id + visibility for team filtering)
            if let Ok(mut stmt) = conn.prepare(
                "SELECT e.target_id, e.vector, m.text, m.source, m.owner_id, m.visibility \
                 FROM embeddings e \
                 JOIN memories m ON e.target_type = 'memory' AND e.target_id = m.id AND m.status = 'active'"
            ) {
                let rows: Vec<(Vec<u8>, String, String, Option<i64>, Option<String>)> = stmt
                    .query_map([], |row| Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
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
                    let sim = crate::embeddings::cosine_similarity(&query_vec, &existing_vec);
                    if sim > SIM_FLOOR as f32 {
                        merged.insert(source.clone(), RecallItem {
                            source,
                            relevance: scale_sim(sim),
                            excerpt: text.chars().take(200).collect(),
                            method: "semantic".to_string(),
                            tokens: None,
                            entropy: None,
                        });
                    }
                }
            }

            // Search decision embeddings (includes owner_id + visibility for team filtering)
            if let Ok(mut stmt) = conn.prepare(
                "SELECT e.target_id, e.vector, d.decision, d.context, d.owner_id, d.visibility \
                 FROM embeddings e \
                 JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id AND d.status = 'active'"
            ) {
                let rows: Vec<(Vec<u8>, String, Option<String>, Option<i64>, Option<String>)> = stmt
                    .query_map([], |row| Ok((row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)))
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
                    let sim = crate::embeddings::cosine_similarity(&query_vec, &existing_vec);
                    if sim > SIM_FLOOR as f32 {
                        let source = context.unwrap_or_else(|| format!("decision::{}", decision.chars().take(40).collect::<String>()));
                        let scaled = scale_sim(sim);
                        let existing = merged.get(&source);
                        if existing.is_none() || scaled > existing.unwrap().relevance {
                            merged.insert(source.clone(), RecallItem {
                                source,
                                relevance: scaled,
                                excerpt: decision.chars().take(200).collect(),
                                method: "semantic".to_string(),
                                tokens: None,
                                entropy: None,
                            });
                        }
                    }
                }
            }
        } else {
            // engine.embed() returned None -- ONNX inference failed at runtime
            if let Some(flag) = degraded_flag {
                // compare_exchange: only log on first transition false→true
                if flag
                    .compare_exchange(
                        false,
                        true,
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    eprintln!(
                        "[recall] Semantic search unavailable, using keyword fallback"
                    );
                }
            }
        }
    }

    // ── Pass 2: Keyword search (over-fetch for visibility filtering) ──────
    // When an entry was already found by semantic search, fuse scores:
    //   hybrid_score = max(semantic, keyword) + 0.15 * min(semantic, keyword)
    // This boosts entries confirmed by both methods without over-inflating.
    //
    // Over-fetch: in team mode, fetch raw_k candidates so that after visibility
    // filtering we still have enough results. Retry with doubled limit up to
    // 2 times if too many get filtered out.
    let raw_k = if ctx.team_mode { k.max(10) * 5 } else { 20 };
    let mut fts_limit = raw_k.max(20);
    let mut retry = 0;
    loop {
    for row in search_memories(conn, &keyword_query, fts_limit)?
        .into_iter()
        .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
    {
        let key = row.source.clone();
        if let Some(existing) = merged.get(&key) {
            if existing.method == "semantic" {
                let fused = existing.relevance.max(row.relevance)
                    + 0.15 * existing.relevance.min(row.relevance);
                merged.insert(
                    key,
                    RecallItem {
                        source: row.source,
                        relevance: fused,
                        excerpt: row.excerpt,
                        method: "hybrid".to_string(),
                        tokens: None,
                        entropy: None,
                    },
                );
                continue;
            }
        }
        let should_replace = merged
            .get(&key)
            .map(|existing| row.relevance > existing.relevance)
            .unwrap_or(true);
        if should_replace {
            merged.insert(
                key,
                RecallItem {
                    source: row.source,
                    relevance: row.relevance,
                    excerpt: row.excerpt,
                    method: "keyword".to_string(),
                    tokens: None,
                    entropy: None,
                },
            );
        }
    }

    for row in search_decisions(conn, &keyword_query, fts_limit)?
        .into_iter()
        .filter(|r| is_visible(r.owner_id, r.visibility.as_deref(), ctx))
    {
        let key = row.source.clone();
        if let Some(existing) = merged.get(&key) {
            if existing.method == "semantic" {
                let fused = existing.relevance.max(row.relevance)
                    + 0.15 * existing.relevance.min(row.relevance);
                merged.insert(
                    key,
                    RecallItem {
                        source: row.source,
                        relevance: fused,
                        excerpt: row.excerpt,
                        method: "hybrid".to_string(),
                        tokens: None,
                        entropy: None,
                    },
                );
                continue;
            }
        }
        let should_replace = merged
            .get(&key)
            .map(|existing| row.relevance > existing.relevance)
            .unwrap_or(true);
        if should_replace {
            merged.insert(
                key,
                RecallItem {
                    source: row.source,
                    relevance: row.relevance,
                    excerpt: row.excerpt,
                    method: "keyword".to_string(),
                    tokens: None,
                    entropy: None,
                },
            );
        }
    }

    // Over-fetch retry: if team mode filtered out too many, widen the FTS net
    if ctx.team_mode && merged.len() < k && retry < 2 {
        fts_limit *= 2;
        retry += 1;
        continue;
    }
    break;
    } // end over-fetch retry loop

    // Compute entropy and apply entropy-weighted re-ranking.
    // High-entropy (information-dense) results get boosted; low-entropy
    // (boilerplate) gets penalized. Weight: 15% adjustment around midpoint 3.5.
    let mut ranked: Vec<RecallItem> = merged
        .into_values()
        .map(|mut item| {
            let h = shannon_entropy(&item.excerpt);
            item.entropy = Some(round4(h));
            item.relevance = round4(item.relevance * (1.0 + (h - 3.5) * 0.15));
            item
        })
        .collect();

    // ── Pass 3: Relevance feedback reranking ──────────────────────────────
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

    for row in &ranked {
        bump_retrieval(conn, &row.source);
    }

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

fn run_budget_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let raw = run_recall_with_engine(conn, query_text, k, engine, ctx, degraded_flag)?;
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

// ─── Search helpers ──────────────────────────────────────────────────────────

fn search_memories(
    conn: &Connection,
    query_text: &str,
    limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
    let tokens = extract_search_keywords(query_text);

    if tokens.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, text, source, tags, score, retrievals, last_accessed, created_at, compressed_text, age_tier \
                 FROM memories WHERE status = 'active' \
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
        let mut stmt = conn
            .prepare(
                "SELECT m.id, m.text, m.source, m.tags, m.score, m.retrievals, m.last_accessed, m.created_at, m.compressed_text, m.age_tier, m.owner_id, m.visibility \
                 FROM memories_fts fts \
                 JOIN memories m ON m.id = fts.rowid \
                 WHERE memories_fts MATCH ?1 AND m.status = 'active' \
                 LIMIT 100",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([&fts_query], |row| {
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
             FROM memories WHERE status = 'active'",
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
    let tokens = extract_search_keywords(query_text);

    if tokens.is_empty() {
        let mut stmt = conn
            .prepare(
                "SELECT id, decision, context, score, retrievals, last_accessed, created_at \
                 FROM decisions WHERE status = 'active' \
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
        let mut stmt = conn
            .prepare(
                "SELECT d.id, d.decision, d.context, d.score, d.retrievals, d.last_accessed, d.created_at, d.compressed_text, d.age_tier, d.owner_id, d.visibility \
                 FROM decisions_fts fts \
                 JOIN decisions d ON d.id = fts.rowid \
                 WHERE decisions_fts MATCH ?1 AND d.status = 'active' \
                 LIMIT 100",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map([&fts_query], |row| {
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
             FROM decisions WHERE status = 'active'",
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
fn bump_retrieval(conn: &Connection, source: &str) {
    let now = now_iso();

    // Boost memories
    let _ = conn.execute(
        "UPDATE memories SET \
           retrievals = retrievals + 1, \
           last_accessed = ?1, \
           score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
         WHERE source = ?2",
        params![now.clone(), source],
    );

    // Boost decisions
    if let Some(id_text) = source.strip_prefix("decision::") {
        if let Ok(id) = id_text.parse::<i64>() {
            let _ = conn.execute(
                "UPDATE decisions SET \
                   retrievals = retrievals + 1, \
                   last_accessed = ?1, \
                   score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
                 WHERE id = ?2",
                params![now, id],
            );
        }
    } else {
        let _ = conn.execute(
            "UPDATE decisions SET \
               retrievals = retrievals + 1, \
               last_accessed = ?1, \
               score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) \
             WHERE context = ?2",
            params![now, source],
        );
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
    results: Vec<RecallItem>,
) -> Vec<RecallItem> {
    if results.is_empty() {
        return results;
    }

    let now = Utc::now().timestamp_millis();
    let mut served = state.served_content.lock().await;
    let map = served
        .entry(agent.to_string())
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

async fn get_pre_cached(state: &RuntimeState, agent: &str, query: &str) -> Option<Vec<RecallItem>> {
    let mut cache = state.pre_cache.lock().await;
    let now = Utc::now().timestamp_millis();

    if let Some(entry) = cache.get(agent) {
        if entry.query == query && entry.expires_at > now {
            // Deserialize the cached Value results back into RecallItems
            if let Some(arr) = entry.results.as_array() {
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
                return Some(items);
            }
        }
    }

    let should_remove = cache
        .get(agent)
        .map(|entry| entry.expires_at <= now)
        .unwrap_or(false);
    if should_remove {
        cache.remove(agent);
    }
    None
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

    let mut cache = state.pre_cache.lock().await;
    cache.insert(
        agent.to_string(),
        PreCacheEntry {
            query: predicted_query,
            results: results_json,
            expires_at: Utc::now().timestamp_millis() + PRECACHE_TTL_MS,
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
            )
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
        "SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 AND status = 'active' ORDER BY score DESC LIMIT 1",
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
                "SELECT decision, context, owner_id, visibility FROM decisions WHERE id = ?1 AND status = 'active'",
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
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE context = ?1 AND status = 'active' ORDER BY score DESC LIMIT 1",
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
        "SELECT text, type, owner_id, visibility FROM memories WHERE source LIKE ?1 AND status = 'active' ORDER BY score DESC LIMIT 1",
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

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
