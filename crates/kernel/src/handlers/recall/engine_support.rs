use super::{
    BENCHMARK_SOURCE_SCOPE_PREFIX, MIN_BUDGET_HEADROOM_TOKENS, MIN_EXCERPT_CHARS, RecallContext,
    RecallItem, budget_rank_char_cap, fit_excerpt_to_remaining_budget, query_focused_excerpt,
    round4, unfold_source,
};
use crate::handlers::{estimate_tokens, now_iso, starts_with_ascii_ignore_case, store};
use crate::protocol::nonempty_opt;
use crate::state::RuntimeState;
use chrono::Utc;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub(crate) fn numbered_placeholders(start: usize, len: usize) -> String {
    (0..len)
        .map(|idx| format!("?{}", start + idx))
        .collect::<Vec<_>>()
        .join(",")
}
fn bump_retrievals_keys<T: rusqlite::types::ToSql>(
    conn: &Connection,
    table: &str,
    key_col: &str,
    now: &str,
    keys: &[T],
) {
    if keys.is_empty() {
        return;
    }
    let placeholders = numbered_placeholders(2, keys.len());
    let sql = format!(
        "UPDATE {table} SET retrievals = retrievals + 1, last_accessed = ?1, score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) WHERE {key_col} IN ({placeholders})"
    );
    let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(keys.len() + 1);
    params.push(&now);
    for key in keys {
        params.push(key);
    }
    let _ = conn
        .prepare_cached(&sql)
        .and_then(|mut stmt| stmt.execute(params.as_slice()));
}
pub fn bump_retrievals_batch(conn: &Connection, items: &[RecallItem]) {
    let sources: Vec<String> = items.iter().map(|item| item.source.clone()).collect();
    bump_retrievals_sources(conn, &sources);
}
pub fn bump_retrievals_sources(conn: &Connection, sources: &[String]) {
    if sources.is_empty() {
        return;
    }
    let now = now_iso();
    // Blank keys would `UPDATE ... WHERE source IN ('')` and bump every
    // sourceless row. Identity keys (`memory::{id}`, `decision::{id}`) are
    // not stored in those columns; they must bump the row by id.
    let sources: Vec<&str> = sources
        .iter()
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .collect();
    if sources.is_empty() {
        return;
    }
    let ids_for = |prefix: &str| {
        sources
            .iter()
            .filter_map(|s| s.strip_prefix(prefix).and_then(|id| id.parse::<i64>().ok()))
            .collect::<Vec<i64>>()
    };
    let memory_ids = ids_for("memory::");
    let decision_ids = ids_for("decision::");
    let named_sources: Vec<&str> = sources
        .iter()
        .copied()
        .filter(|s| !s.starts_with("memory::") && !s.starts_with("decision::"))
        .collect();
    bump_retrievals_keys(conn, "memories", "source", &now, &named_sources);
    bump_retrievals_keys(conn, "memories", "id", &now, &memory_ids);
    bump_retrievals_keys(conn, "decisions", "id", &now, &decision_ids);
    bump_retrievals_keys(conn, "decisions", "context", &now, &named_sources);
}
pub fn recall_to_json(item: RecallItem) -> Value {
    let mut payload = json!({"source":item.source,"relevance":item.relevance,"excerpt":item.excerpt,"method":item.method,"why":item.why_json()});
    if let Value::Object(ref mut map) = payload {
        // Governance facts travel with the hit: status and validity are
        // what applicability is decided on, never inferred from relevance.
        insert_opt_str(map, "status", item.status.as_deref());
        insert_opt_str(map, "validFrom", item.valid_from.as_deref());
        insert_opt_str(map, "validUntil", item.valid_until.as_deref());
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
                        .map(|(source, relevance)| json!({"source":source,"relevance":relevance,}))
                        .collect(),
                ),
            );
        }
    }
    payload
}
fn insert_opt_str(map: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::String(value.to_string()));
    }
}
#[derive(Clone, Copy, Debug)]
pub struct RecallBudgetUsage {
    pub spent: usize,
    pub saved: i64,
    pub over_budget: bool,
}
pub fn recall_item_token_cost(item: &RecallItem) -> usize {
    item.tokens
        .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
}
pub fn compute_recall_budget_usage(items: &[RecallItem], budget: usize) -> RecallBudgetUsage {
    let spent: usize = items.iter().map(recall_item_token_cost).sum();
    let saved = budget as i64 - spent as i64;
    RecallBudgetUsage {
        spent,
        saved,
        over_budget: budget > 0 && spent > budget,
    }
}
pub fn compute_headlines_token_usage(items: &[RecallItem]) -> RecallBudgetUsage {
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
pub fn format_recall_token_usage_line(budget: usize, usage: RecallBudgetUsage) -> String {
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
pub fn pack_budget(
    items: Vec<RecallItem>,
    token_budget: usize,
    query_text: &str,
) -> Vec<RecallItem> {
    let mut spent = 0usize;
    let mut kept = Vec::new();
    for (idx, mut item) in items.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= MIN_BUDGET_HEADROOM_TOKENS {
            break;
        }
        let tokens = recall_item_token_cost(&item);
        if tokens <= remaining {
            item.tokens = Some(tokens);
            spent += tokens;
            kept.push(item);
            continue;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text)
            .min((remaining as f64 * 3.6) as usize)
            .max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, used)) =
            fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining)
        {
            item.excerpt = excerpt;
            item.tokens = Some(used);
            spent += used;
            kept.push(item);
        }
    }
    kept
}
pub fn enforce_budget_token_invariant(
    results: Vec<RecallItem>,
    token_budget: usize,
    query_text: &str,
) -> Vec<RecallItem> {
    if token_budget == 0
        || results.is_empty()
        || !compute_recall_budget_usage(&results, token_budget).over_budget
    {
        return results;
    }
    pack_budget(results, token_budget, query_text)
}
pub fn hash_content(content: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    // Length first so two excerpts that share a long prefix still differ.
    // Hash the whole string: recall excerpts are already bounded, and a
    // 100-char window treated distinct memories as already-served.
    hash ^= content.chars().count() as u32;
    hash = hash.wrapping_mul(16_777_619);
    for ch in content.chars() {
        hash ^= ch as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}
pub fn source_dedup_hash(source: &str) -> u32 {
    hash_content(&format!("source::{source}"))
}
pub fn collapse_score_is_better(
    candidate_score: f64,
    candidate_order: usize,
    best_score: f64,
    best_order: usize,
) -> bool {
    candidate_score
        .total_cmp(&best_score)
        .then(best_order.cmp(&candidate_order))
        .is_gt()
}
pub async fn load_collapsed_source_fallback(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    source: &str,
    query: &str,
    ctx: &RecallContext,
    relevance: f64,
) -> Result<Option<RecallItem>, String> {
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    let Some(payload) = unfold_source(&conn, source, ctx) else {
        return Ok(None);
    };
    let canonical_source = payload
        .get("source")
        .and_then(|value| value.as_str())
        .unwrap_or(source)
        .to_string();
    let Some(text) = payload.get("text").and_then(|value| value.as_str()) else {
        return Ok(None);
    };
    Ok(Some({
        let mut item = RecallItem::new_with_why(
            canonical_source,
            relevance,
            query_focused_excerpt(text, query, 260),
            "crystal".to_string(),
        );
        item.base_relevance = 0.0;
        item
    }))
}
pub const SERVED_TTL_MS: i64 = 60_000;
fn served_hashes(item: &RecallItem) -> [u32; 2] {
    [hash_content(&item.excerpt), source_dedup_hash(&item.source)]
}

async fn best_unserved_member(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    query: &str,
    ctx: &RecallContext,
    result: &RecallItem,
    seen: &HashSet<u32>,
) -> Result<Option<(RecallItem, [u32; 2])>, String> {
    if result.method != "crystal" || result.collapsed_sources.is_empty() {
        return Ok(None);
    }
    // Borrow the selected source list. Scores take precedence when present.
    let unscored = if result.collapsed_source_scores.is_empty() {
        result.collapsed_sources.as_slice()
    } else {
        &[]
    };
    let candidates = result
        .collapsed_source_scores
        .iter()
        .map(|(source, score)| (source, *score))
        .chain(unscored.iter().map(|source| (source, 0.0)));
    let mut best: Option<(usize, f64, RecallItem, [u32; 2])> = None;
    for (order, (source, score)) in candidates.enumerate() {
        if seen.contains(&source_dedup_hash(source)) {
            continue;
        }
        let relevance = round4(score.max(0.0));
        let Some(candidate) =
            load_collapsed_source_fallback(cx, state, source, query, ctx, relevance).await?
        else {
            continue;
        };
        let hashes = served_hashes(&candidate);
        if hashes.iter().any(|hash| seen.contains(hash)) {
            continue;
        }
        if best.as_ref().is_none_or(|(best_order, best_score, _, _)| {
            collapse_score_is_better(relevance, order, *best_score, *best_order)
        }) {
            best = Some((order, relevance, candidate, hashes));
        }
    }
    Ok(best.map(|(_, _, item, hashes)| (item, hashes)))
}

pub async fn dedup_and_mark_served(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    agent: &str,
    query: &str,
    ctx: &RecallContext,
    results: Vec<RecallItem>,
) -> Result<Vec<RecallItem>, String> {
    if results.is_empty() {
        return Ok(results);
    }
    let now = Utc::now().timestamp_millis();
    let scope_key = served_content_scope(agent, query, ctx);
    let mut seen_hashes: HashSet<u32> = {
        let mut served = state
            .served_content
            .lock(cx)
            .await
            .map_err(|e| e.to_string())?;
        let map = served
            .entry(scope_key.clone())
            .or_insert_with(HashMap::<u32, i64>::new);
        map.retain(|_, ts| now - *ts < SERVED_TTL_MS);
        map.keys().copied().collect()
    };
    let mut staged_hashes = Vec::with_capacity(results.len() * 2);
    let mut filtered = Vec::new();
    for result in results {
        let hashes = served_hashes(&result);
        let selected = if hashes.iter().any(|hash| seen_hashes.contains(hash)) {
            best_unserved_member(cx, state, query, ctx, &result, &seen_hashes).await?
        } else {
            Some((result, hashes))
        };
        let Some((item, hashes)) = selected else {
            continue;
        };
        seen_hashes.extend(hashes);
        staged_hashes.extend(hashes);
        filtered.push(item);
    }
    if !staged_hashes.is_empty() {
        let mut served = state
            .served_content
            .lock(cx)
            .await
            .map_err(|e| e.to_string())?;
        let map = served
            .entry(scope_key)
            .or_insert_with(HashMap::<u32, i64>::new);
        map.retain(|_, ts| now - *ts < SERVED_TTL_MS);
        map.extend(staged_hashes.into_iter().map(|hash| (hash, now)));
    }
    Ok(filtered)
}
pub fn recall_owner_scope(ctx: &RecallContext) -> String {
    if !ctx.team_mode {
        return "solo".to_string();
    }
    ctx.caller_id
        .map(|owner_id| format!("team:{owner_id}"))
        .unwrap_or_else(|| "team:none".to_string())
}
pub fn recall_scope_key(agent: &str, ctx: &RecallContext) -> String {
    format!("{}::{agent}", recall_owner_scope(ctx))
}
pub fn served_content_scope(agent: &str, query: &str, ctx: &RecallContext) -> String {
    let normalized_query = query
        .split_whitespace()
        .map(|segment| segment.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    format!("{}::{agent}::{normalized_query}", recall_owner_scope(ctx))
}
pub fn is_benchmark_recall_scope(agent: &str, source_prefix: Option<&str>) -> bool {
    store::is_benchmark_source_agent(agent)
        || starts_with_ascii_ignore_case(
            nonempty_opt(source_prefix).unwrap_or(""),
            BENCHMARK_SOURCE_SCOPE_PREFIX,
        )
}
