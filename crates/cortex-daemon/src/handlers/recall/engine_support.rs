pub(crate) fn round4(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * 10000.0).round() / 10000.0
}
fn numbered_placeholders(start: usize, len: usize) -> String {
    let mut placeholders = String::new();
    for idx in 0..len {
        if idx > 0 {
            placeholders.push(',');
        }
        let _ = write!(placeholders, "?{}", start + idx);
    }
    placeholders
}
fn bump_retrievals_str_keys(conn: &Connection, table: &str, key_col: &str, now: &str, keys: &[&str]) {
    if keys.is_empty() {
        return;
    }
    let placeholders = numbered_placeholders(2, keys.len());
    let sql=format!("UPDATE {table} SET retrievals = retrievals + 1, last_accessed = ?1, score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) WHERE {key_col} IN ({placeholders})");
    let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(keys.len() + 1);
    params.push(&now);
    for key in keys {
        params.push(key);
    }
    let _ = conn.execute(&sql, params.as_slice());
}
fn bump_retrievals_i64_keys(conn: &Connection, table: &str, key_col: &str, now: &str, keys: &[i64]) {
    if keys.is_empty() {
        return;
    }
    let placeholders = numbered_placeholders(2, keys.len());
    let sql=format!("UPDATE {table} SET retrievals = retrievals + 1, last_accessed = ?1, score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) WHERE {key_col} IN ({placeholders})");
    let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(keys.len() + 1);
    params.push(&now);
    for key in keys {
        params.push(key);
    }
    let _ = conn.execute(&sql, params.as_slice());
}
pub(crate) fn bump_retrievals_batch(conn: &Connection, items: &[RecallItem]) {
    if items.is_empty() {
        return;
    }
    let now = now_iso();
    let sources: Vec<&str> = items.iter().map(|item| item.source.as_str()).collect();
    bump_retrievals_str_keys(conn, "memories", "source", &now, &sources);
    let decision_ids: Vec<i64> = sources.iter().filter_map(|s| s.strip_prefix("decision::").and_then(|id| id.parse::<i64>().ok())).collect();
    bump_retrievals_i64_keys(conn, "decisions", "id", &now, &decision_ids);
    let context_sources: Vec<&str> = sources.iter().copied().filter(|source| !source.starts_with("decision::")).collect();
    bump_retrievals_str_keys(conn, "decisions", "context", &now, &context_sources);
}
pub(crate) fn recall_to_json(item: RecallItem) -> Value {
    let mut payload = json!({"source":item.source,"relevance":item.relevance,"excerpt":item.excerpt,"method":item.method});
    if let Value::Object(ref mut map) = payload {
        if let Some(tokens) = item.tokens {
            map.insert("tokens".to_string(), Value::Number((tokens as u64).into()));
        }
        if !item.family_members.is_empty() {
            let family_size = item.family_members.len() as u64;
            map.insert("familyMembers".to_string(), Value::Array(item.family_members.into_iter().map(Value::String).collect()));
            map.insert("familySize".to_string(), Value::Number(family_size.into()));
        }
        if !item.collapsed_sources.is_empty() {
            map.insert("collapsedSources".to_string(), Value::Array(item.collapsed_sources.into_iter().map(Value::String).collect()));
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecallBudgetUsage {
    pub(crate) spent: usize,
    pub(crate) saved: i64,
    pub(crate) over_budget: bool,
}
pub(crate) fn recall_item_token_cost(item: &RecallItem) -> usize {
    item.tokens.unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
}
pub(crate) fn compute_recall_budget_usage(items: &[RecallItem], budget: usize) -> RecallBudgetUsage {
    let spent: usize = items.iter().map(recall_item_token_cost).sum();
    let saved = budget as i64 - spent as i64;
    RecallBudgetUsage { spent, saved, over_budget: budget > 0 && spent > budget }
}
pub(crate) fn compute_headlines_token_usage(items: &[RecallItem]) -> RecallBudgetUsage {
    let spent = items.iter().map(|item| estimate_tokens(&item.source)).sum::<usize>();
    let full_recall_tokens = items.iter().map(recall_item_token_cost).sum::<usize>();
    RecallBudgetUsage { spent, saved: full_recall_tokens as i64 - spent as i64, over_budget: false }
}
pub(crate) fn format_recall_token_usage_line(budget: usize, usage: RecallBudgetUsage) -> String {
    if budget == 0 {
        if usage.saved > 0 {
            format!("Cortex recall used {} tokens in headlines mode and saved {} vs full excerpts.", usage.spent, usage.saved)
        } else {
            format!("Cortex recall used {} tokens (headlines mode).", usage.spent)
        }
    } else if usage.saved >= 0 {
        format!("Cortex recall used {} tokens and saved {} of {} budget.", usage.spent, usage.saved, budget)
    } else {
        format!("Cortex recall used {} tokens ({} over budget {}).", usage.spent, usage.saved.abs(), budget)
    }
}
pub(crate) fn enforce_budget_token_invariant(results: Vec<RecallItem>, token_budget: usize, query_text: &str) -> Vec<RecallItem> {
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
        let cap = budget_rank_char_cap(token_budget, idx, query_text).min((remaining as f64 * 3.6) as usize).max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) = fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining) {
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
pub(crate) fn collapse_score_is_better(candidate_score: f64, candidate_order: usize, best_score: f64, best_order: usize) -> bool {
    match candidate_score.total_cmp(&best_score) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => candidate_order < best_order,
    }
}
pub(crate) async fn load_collapsed_source_fallback(state: &RuntimeState, source: &str, query: &str, ctx: &RecallContext, relevance: f64) -> Option<RecallItem> {
    let conn = state.db_read.lock().await;
    let payload = unfold_source(&conn, source, ctx)?;
    let canonical_source = payload.get("source").and_then(|value| value.as_str()).unwrap_or(source).to_string();
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
pub(crate) const SERVED_TTL_MS: i64 = 60_000;
pub(crate) async fn dedup_and_mark_served(state: &RuntimeState, agent: &str, query: &str, ctx: &RecallContext, results: Vec<RecallItem>) -> Vec<RecallItem> {
    if results.is_empty() {
        return results;
    }
    let now = Utc::now().timestamp_millis();
    let scope_key = served_content_scope(agent, query, ctx);
    let mut seen_hashes: HashSet<u32> = {
        let mut served = state.served_content.lock().await;
        let map = served.entry(scope_key.clone()).or_insert_with(HashMap::<u32, i64>::new);
        map.retain(|_, ts| now - *ts < SERVED_TTL_MS);
        map.keys().copied().collect()
    };
    let mut staged_hashes: Vec<u32> = Vec::with_capacity(results.len() * 2);
    let mut filtered = Vec::new();
    for result in results {
        let excerpt_hash = hash_content(&result.excerpt);
        let source_hash = source_dedup_hash(&result.source);
        let already_served = seen_hashes.contains(&excerpt_hash) || seen_hashes.contains(&source_hash);
        if already_served {
            if result.method == "crystal" && !result.collapsed_sources.is_empty() {
                let fallback_candidates: Vec<(usize, String, f64)> = if result.collapsed_source_scores.is_empty() {
                    result.collapsed_sources.iter().enumerate().map(|(idx, source)| (idx, source.clone(), 0.0)).collect()
                } else {
                    result.collapsed_source_scores.iter().enumerate().map(|(idx, (source, score))| (idx, source.clone(), *score)).collect()
                };
                let mut best_candidate: Option<(usize, f64, RecallItem)> = None;
                for (order, collapsed_source, collapsed_score) in fallback_candidates {
                    let collapsed_source_hash = source_dedup_hash(&collapsed_source);
                    if seen_hashes.contains(&collapsed_source_hash) {
                        continue;
                    }
                    let candidate_relevance = round4(collapsed_score.max(0.0));
                    let Some(candidate) = load_collapsed_source_fallback(state, &collapsed_source, query, ctx, candidate_relevance).await else {
                        continue;
                    };
                    let candidate_excerpt_hash = hash_content(&candidate.excerpt);
                    let candidate_source_hash = source_dedup_hash(&candidate.source);
                    if seen_hashes.contains(&candidate_excerpt_hash) || seen_hashes.contains(&candidate_source_hash) {
                        continue;
                    }
                    let replace = match &best_candidate {
                        None => true,
                        Some((best_order, best_score, _)) => collapse_score_is_better(candidate_relevance, order, *best_score, *best_order),
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
        let map = served.entry(scope_key).or_insert_with(HashMap::<u32, i64>::new);
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
    let normalized_query = query.split_whitespace().map(|segment| segment.to_ascii_lowercase()).collect::<Vec<_>>().join(" ");
    format!("{}::{agent}::{normalized_query}", recall_owner_scope(ctx))
}
pub(crate) fn maybe_apply_rerank(state: &RuntimeState, results: Vec<RecallItem>, budget: usize) -> (Vec<RecallItem>, Value) {
    let config = &state.rerank_config;
    let reason = if budget == 0 { "headlines_mode" } else { "mode_off" };
    (results, json!({"status":"skipped","reason":reason,"mode":config.mode.as_str()}))
}
pub(crate) fn is_benchmark_recall_scope(agent: &str, source_prefix: Option<&str>) -> bool {
    if agent.trim().to_ascii_lowercase().starts_with(BENCHMARK_SOURCE_AGENT_PREFIX) {
        return true;
    }
    source_prefix.map(str::trim).unwrap_or_default().to_ascii_lowercase().starts_with(BENCHMARK_SOURCE_SCOPE_PREFIX)
}
