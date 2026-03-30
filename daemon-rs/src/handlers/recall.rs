use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{NaiveDateTime, TimeZone, Utc};
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::state::{PreCacheEntry, RecallHistoryEntry, RuntimeState};
use super::{estimate_tokens, json_response, now_iso, truncate_chars};

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
}

#[derive(Clone)]
struct SearchCandidate {
    source: String,
    excerpt: String,
    relevance: f64,
    matched_keywords: i64,
    score: f64,
    ts: i64,
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

    match execute_unified_recall(&state, q.trim(), budget, k, &agent).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Recall failed: {err}") }),
        ),
    }
}

// ─── GET /peek ───────────────────────────────────────────────────────────────

pub async fn handle_peek(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
) -> Response {
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
    let mut conn = state.db.lock().await;
    match run_recall(&mut conn, &q, k) {
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

async fn execute_unified_recall(
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
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
    let results = if budget == 0 {
        run_recall(&mut conn, query_text, k)?
    } else {
        run_budget_recall(&mut conn, query_text, budget, k)?
    };

    // Co-occurrence tracking + prediction
    let sources: Vec<String> = results.iter().map(|item| item.source.clone()).collect();
    let predictions = if sources.len() >= 2 {
        if co_occurrence::record(&conn, &sources).is_ok() {
            checkpoint_wal_best_effort(&conn);
        } else {
            let _ = co_occurrence::reset(&conn);
        }

        match co_occurrence::predict(&conn, &sources, 3) {
            Ok(preds) => preds,
            Err(_) => {
                let _ = co_occurrence::reset(&conn);
                vec![]
            }
        }
    } else {
        vec![]
    };
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

    let mut payload = json!({
        "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": spent,
        "saved": saved,
        "mode": if budget >= 500 { "full" } else { "balanced" }
    });

    if let Value::Object(ref mut map) = payload {
        if !predictions.is_empty() {
            map.insert("predictions".to_string(), Value::Array(predictions));
        }
    }

    Ok(payload)
}

// ─── Core recall ─────────────────────────────────────────────────────────────

fn run_recall(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
) -> Result<Vec<RecallItem>, String> {
    let extracted = extract_keywords(query_text);
    let keyword_query = if extracted.is_empty() {
        query_text.to_string()
    } else {
        extracted.join(" ")
    };

    let mut merged: HashMap<String, RecallItem> = HashMap::new();

    for row in search_memories(conn, &keyword_query, 20)? {
        let key = row.source.clone();
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
                },
            );
        }
    }

    for row in search_decisions(conn, &keyword_query, 20)? {
        let key = row.source.clone();
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
                },
            );
        }
    }

    let mut ranked = merged.into_values().collect::<Vec<_>>();
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
) -> Result<Vec<RecallItem>, String> {
    let raw = run_recall(conn, query_text, k)?;
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
            });
            continue;
        }

        let haystacks = vec![
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
        let retrieval_weight = (retrievals.unwrap_or(0).max(0).min(20) as f64) / 20.0;
        let score_weight = score.min(5.0) / 5.0;
        let ranking =
            (keyword_weight * 0.5) + (recency_weight * 0.2) + (retrieval_weight * 0.15) + (score_weight * 0.15);

        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: truncate_chars(&text, 200),
            relevance: round4(ranking),
            matched_keywords: matched,
            score,
            ts,
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
            });
            continue;
        }

        let haystacks = vec![
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
        let retrieval_weight = (retrievals.unwrap_or(0).max(0).min(20) as f64) / 20.0;
        let score_weight = score.min(5.0) / 5.0;
        let ranking =
            (keyword_weight * 0.5) + (recency_weight * 0.2) + (retrieval_weight * 0.15) + (score_weight * 0.15);

        ranked.push(SearchCandidate {
            source: source_key,
            excerpt: truncate_chars(&decision, 200),
            relevance: round4(ranking),
            matched_keywords: matched,
            score,
            ts,
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
    ((Utc::now().timestamp_millis() - ts).max(0) / (24 * 60 * 60 * 1000)) as i64
}

fn round4(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

fn bump_retrieval(conn: &Connection, source: &str) {
    let now = now_iso();
    let _ = conn.execute(
        "UPDATE memories SET retrievals = retrievals + 1, last_accessed = ?1 WHERE source = ?2",
        params![now.clone(), source],
    );
    if let Some(id_text) = source.strip_prefix("decision::") {
        if let Ok(id) = id_text.parse::<i64>() {
            let _ = conn.execute(
                "UPDATE decisions SET retrievals = retrievals + 1, last_accessed = ?1 WHERE id = ?2",
                params![now, id],
            );
        }
    } else {
        let _ = conn.execute(
            "UPDATE decisions SET retrievals = retrievals + 1, last_accessed = ?1 WHERE context = ?2",
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
    if let Some(tokens) = item.tokens {
        if let Value::Object(ref mut map) = payload {
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

async fn dedup_and_mark_served(
    state: &RuntimeState,
    agent: &str,
    results: Vec<RecallItem>,
) -> Vec<RecallItem> {
    if results.is_empty() {
        return results;
    }

    let mut served = state.served_content.lock().await;
    let set = served
        .entry(agent.to_string())
        .or_insert_with(HashSet::<u32>::new);

    let mut filtered = Vec::new();
    for result in results {
        let hash = hash_content(&result.excerpt);
        if set.contains(&hash) {
            continue;
        }
        set.insert(hash);
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

async fn get_pre_cached(
    state: &RuntimeState,
    agent: &str,
    query: &str,
) -> Option<Vec<RecallItem>> {
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

    let mut conn = state.db.lock().await;
    let results = run_budget_recall(&mut conn, &predicted_query, 200, 5)?;
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
