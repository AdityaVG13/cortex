pub(crate) fn round4(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value * 10000.0).round() / 10000.0
}
fn bump_retrievals_keys(conn: &Connection, table: &str, key_col: &str, now: &str, keys: &[String]) {
    if keys.is_empty() {
        return;
    }
    let placeholders: String = (2..=keys.len() + 1)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql=format!("UPDATE {table} SET retrievals = retrievals + 1, last_accessed = ?1, score = MIN(1.0, score + 0.15 / (1.0 + 0.1 * retrievals)) WHERE {key_col} IN ({placeholders})");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(keys.len() + 1);
    params.push(Box::new(now.to_string()));
    for key in keys {
        params.push(Box::new(key.clone()));
    }
    let refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let _ = conn.execute(&sql, refs.as_slice());
}
pub(crate) fn bump_retrievals_batch(conn: &Connection, items: &[RecallItem]) {
    if items.is_empty() {
        return;
    }
    let now = now_iso();
    let sources: Vec<String> = items.iter().map(|i| i.source.clone()).collect();
    bump_retrievals_keys(conn, "memories", "source", &now, &sources);
    let decision_ids: Vec<String> = sources
        .iter()
        .filter_map(|s| {
            s.strip_prefix("decision::")
                .and_then(|id| id.parse::<i64>().ok())
                .map(|id| id.to_string())
        })
        .collect();
    bump_retrievals_keys(conn, "decisions", "id", &now, &decision_ids);
    let context_sources: Vec<String> = sources
        .iter()
        .filter(|s| !s.starts_with("decision::"))
        .cloned()
        .collect();
    bump_retrievals_keys(conn, "decisions", "context", &now, &context_sources);
}
pub(crate) fn recall_to_json(item: RecallItem) -> Value {
    let mut payload = json!({"source":item.source,"relevance":item.relevance,"excerpt":item.excerpt,"method":item.method});
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
    item.tokens
        .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
}
pub(crate) fn compute_recall_budget_usage(
    items: &[RecallItem],
    budget: usize,
) -> RecallBudgetUsage {
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
pub(crate) const SERVED_TTL_MS: i64 = 60_000;
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
    if let Some(entry) = cache.get(scope_key) {
        if entry.query == query && entry.expires_at > now {
            return deserialize_cache_entry(&entry.results);
        }
    }
    if cache
        .get(scope_key)
        .map(|e| e.expires_at <= now)
        .unwrap_or(false)
    {
        cache.remove(scope_key);
    }
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
    let results_json: Value = results.into_iter().map(recall_to_json).collect();
    let now_ms = Utc::now().timestamp_millis();
    let mut cache = state.pre_cache.lock().await;
    cache.retain(|_, entry| entry.expires_at > now_ms);
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
pub(crate) fn rerank_candidate_text(item: &RecallItem) -> String {
    let text = if item.excerpt.trim().is_empty() {
        item.source.clone()
    } else {
        format!("{} {}", item.source, item.excerpt)
    };
    truncate_chars(&text, 1800)
}
pub(crate) fn build_rerank_candidates(
    results: &[RecallItem],
    top_n: usize,
) -> Vec<RerankCandidate> {
    results
        .iter()
        .take(top_n.max(1))
        .map(|item| RerankCandidate {
            id: item.source.clone(),
            text: rerank_candidate_text(item),
            base_score: item.relevance,
        })
        .collect()
}
pub(crate) fn remap_fused_score_to_relevance(fused_score: f64, window: &[RecallItem]) -> f64 {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for relevance in window.iter().map(|item| item.relevance) {
        if relevance.is_finite() {
            min = min.min(relevance);
            max = max.max(relevance);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        return round4(fused_score.clamp(0.0, 1.0));
    }
    let span = (max - min).max(0.01);
    round4(min + (span * fused_score.clamp(0.0, 1.0)))
}
pub(crate) fn apply_primary_rerank(
    results: Vec<RecallItem>,
    reranked: &[RerankedScore],
) -> Vec<RecallItem> {
    if reranked.is_empty() {
        return results;
    }
    let window_len = reranked.len().min(results.len());
    let window = &results[..window_len];
    let mut by_source: HashMap<String, RecallItem> = results
        .iter()
        .take(window_len)
        .cloned()
        .map(|item| (item.source.clone(), item))
        .collect();
    let mut output = Vec::with_capacity(results.len());
    for score in reranked {
        if let Some(mut item) = by_source.remove(&score.id) {
            item.relevance = remap_fused_score_to_relevance(score.fused_score, window);
            if !item.method.contains("rerank") {
                item.method = format!("{}+rerank", item.method);
            }
            output.push(item);
        }
    }
    for item in results.iter().take(window_len) {
        if let Some(item) = by_source.remove(&item.source) {
            output.push(item);
        }
    }
    output.extend(results.into_iter().skip(window_len));
    output
}
pub(crate) fn rerank_scores_json(reranked: &[RerankedScore]) -> Vec<Value> {
    reranked.iter().take(12).enumerate().map(|(idx,score)|{json!({"rank":idx+1,"source":score.id,"baseScore":round4(score.base_score),"rerankScore":round4(score.rerank_score),"fusedScore":round4(score.fused_score),})}).collect()
}
pub(crate) fn maybe_apply_rerank(
    state: &RuntimeState,
    query_text: &str,
    results: Vec<RecallItem>,
    budget: usize,
) -> (Vec<RecallItem>, Value) {
    let config = &state.rerank_config;
    if budget == 0 {
        return (
            results,
            json!({"status":"skipped","reason":"headlines_mode","mode":config.mode.as_str(),}),
        );
    }
    if !config.is_active() {
        return (
            results,
            json!({"status":"skipped","reason":"mode_off","mode":config.mode.as_str(),}),
        );
    }
    if results.len() < 2 {
        let candidate_count = results.len();
        return (
            results,
            json!({"status":"skipped","reason":"not_enough_candidates","mode":config.mode.as_str(),"candidateCount":candidate_count,}),
        );
    }
    let Some(reranker) = state.reranker.as_ref() else {
        return (
            results,
            json!({"status":"unavailable","reason":"model_not_loaded","mode":config.mode.as_str(),"configuredModel":crate::rerank::selected_reranker_selection().key,}),
        );
    };
    let top_n = config.top_n.min(results.len());
    let candidates = build_rerank_candidates(&results, top_n);
    let baseline_top_sources = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    match reranker.rerank(query_text, &candidates, config.fusion_alpha) {
        Ok(reranked) => {
            let reranked_top_sources = reranked
                .iter()
                .map(|score| score.id.clone())
                .collect::<Vec<_>>();
            let telemetry = json!({"status":"ok","mode":config.mode.as_str(),"applied":config.is_primary(),"model":reranker.name(),"modelSizeMb":reranker.model_size_mb(),"topN":top_n,"fusionAlpha":round4(config.fusion_alpha),"baselineTopSources":baseline_top_sources,"rerankedTopSources":reranked_top_sources,"scores":rerank_scores_json(&reranked),});
            let results = if config.is_primary() {
                apply_primary_rerank(results, &reranked)
            } else {
                results
            };
            (results, telemetry)
        }
        Err(error) => (
            results,
            json!({"status":"error","mode":config.mode.as_str(),"applied":false,"model":reranker.name(),"reason":truncate_chars(&error,240),}),
        ),
    }
}
pub(crate) fn sqlite_vec_trial_sampled(
    query_text: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    trial_percent: u8,
) -> bool {
    if trial_percent == 0 {
        return false;
    }
    if trial_percent >= 100 {
        return true;
    }
    let mut hasher = DefaultHasher::new();
    query_text.hash(&mut hasher);
    ctx.team_mode.hash(&mut hasher);
    ctx.caller_id.hash(&mut hasher);
    source_prefix.unwrap_or_default().hash(&mut hasher);
    let bucket = (hasher.finish() % 100) as u8;
    bucket < trial_percent
}
pub(crate) fn parse_shadow_sources(shadow_semantic: &Value, field: &str) -> Vec<String> {
    shadow_semantic
        .get(field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
pub(crate) fn shadow_guard_failure_reason(shadow_semantic: &Value) -> Option<&'static str> {
    if shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error")
        != "ok"
    {
        return Some("shadow_not_ok");
    }
    let overlap_ratio = shadow_semantic
        .get("overlapRatio")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if overlap_ratio < SQLITE_VEC_TRIAL_MIN_OVERLAP_RATIO {
        return Some("overlap_ratio_below_gate");
    }
    let jaccard = shadow_semantic
        .get("jaccard")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    if jaccard < SQLITE_VEC_TRIAL_MIN_JACCARD {
        return Some("jaccard_below_gate");
    }
    let mean_abs_rank_delta = shadow_semantic
        .get("meanAbsRankDelta")
        .and_then(Value::as_f64)
        .unwrap_or(f64::INFINITY);
    if mean_abs_rank_delta > SQLITE_VEC_TRIAL_MAX_MEAN_ABS_RANK_DELTA {
        return Some("rank_delta_above_gate");
    }
    if SQLITE_VEC_TRIAL_TOP1_MATCH_REQUIRED {
        let top1_match = shadow_semantic
            .get("top1Match")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !top1_match {
            return Some("top1_match_required");
        }
    }
    None
}
pub(crate) fn sqlite_vec_source_fallback_candidate(
    conn: &Connection,
    source: &str,
    query_text: &str,
    fallback_relevance: f64,
) -> Option<SemanticCandidate> {
    const ACTIVE:&str="status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) AND (valid_from IS NULL OR valid_from <= datetime('now')) AND (valid_until IS NULL OR valid_until > datetime('now'))";
    let build = |text: String,
                 score: Option<f64>,
                 trust_score: Option<f64>,
                 last_accessed: Option<String>,
                 created_at: Option<String>| SemanticCandidate {
        source: source.to_string(),
        excerpt: query_focused_excerpt(&text, query_text, 280),
        relevance: fallback_relevance,
        importance: blend_importance(score, trust_score),
        ts: parse_timestamp_ms(
            last_accessed
                .as_deref()
                .or(created_at.as_deref())
                .unwrap_or_default(),
        ),
    };
    let query_text_row = |sql: &str, bind: &[&dyn rusqlite::types::ToSql]| {
        conn.query_row(sql, bind, |row| {
            Ok(build(
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .optional()
        .ok()
        .flatten()
    };
    if let Some(id) = source
        .strip_prefix("memory::")
        .and_then(|raw| raw.parse::<i64>().ok())
    {
        if let Some(candidate)=query_text_row(&format!("SELECT text, score, trust_score, last_accessed, created_at FROM memories WHERE id = ?1 AND {ACTIVE} LIMIT 1"),&[&id]){return Some(candidate);}
    }
    if let Some(candidate)=query_text_row(&format!("SELECT text, score, trust_score, last_accessed, created_at FROM memories WHERE source = ?1 AND {ACTIVE} ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT 1"),&[&source]){return Some(candidate);}
    if let Some(id) = source
        .strip_prefix("decision::")
        .and_then(|raw| raw.parse::<i64>().ok())
    {
        if let Some(candidate)=query_text_row(&format!("SELECT decision, score, trust_score, last_accessed, created_at FROM decisions WHERE id = ?1 AND {ACTIVE} LIMIT 1"),&[&id]){return Some(candidate);}
    }
    query_text_row(&format!("SELECT decision, score, trust_score, last_accessed, created_at FROM decisions WHERE context = ?1 AND {ACTIVE} ORDER BY COALESCE(last_accessed, created_at) DESC LIMIT 1"),&[&source])
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn maybe_apply_sqlite_vec_trial(
    conn: &Connection,
    query_text: &str,
    query_vector: Option<&[f32]>,
    semantic_candidates: Vec<SemanticCandidate>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    top_k: usize,
    canary: Option<&SqliteVecCanaryConfig>,
    sqlite_vec_shadow_enabled: bool,
) -> (Vec<SemanticCandidate>, Value) {
    let Some(canary) = canary else {
        return (
            semantic_candidates,
            json!({"mode":"baseline","reason":"trial_not_configured","sampled":false,"trialPercent":0,"routeMode":"baseline"}),
        );
    };
    let effective_route_mode = canary.effective_route_mode();
    let route_mode = effective_route_mode.as_str();
    let active_trial_percent = if matches!(effective_route_mode, SqliteVecRouteMode::Primary) {
        100
    } else {
        canary.trial_percent
    };
    let baseline_route = |reason: &str, sampled: bool, trial_percent: u8| json!({"mode":"baseline","reason":reason,"sampled":sampled,"trialPercent":trial_percent,"routeMode":route_mode});
    if matches!(effective_route_mode, SqliteVecRouteMode::Baseline) {
        let reason = if canary.force_off {
            "trial_force_off"
        } else {
            "route_mode_baseline"
        };
        return (
            semantic_candidates,
            baseline_route(reason, false, active_trial_percent),
        );
    }
    let Some(query_vector) = query_vector else {
        return (
            semantic_candidates,
            baseline_route("query_embedding_unavailable", false, active_trial_percent),
        );
    };
    if semantic_candidates.is_empty() {
        return (
            semantic_candidates,
            baseline_route("no_semantic_candidates", false, active_trial_percent),
        );
    }
    let sampled = if matches!(effective_route_mode, SqliteVecRouteMode::Trial) {
        if canary.trial_percent == 0 {
            return (
                semantic_candidates,
                baseline_route("trial_percent_zero", false, active_trial_percent),
            );
        }
        let sampled =
            sqlite_vec_trial_sampled(query_text, ctx, source_prefix, canary.trial_percent);
        if !sampled {
            return (
                semantic_candidates,
                baseline_route("not_sampled", false, active_trial_percent),
            );
        }
        true
    } else {
        true
    };
    if !sqlite_vec_shadow_enabled {
        return (
            semantic_candidates,
            baseline_route("hot_path_shadow_skipped", sampled, active_trial_percent),
        );
    }
    let baseline = ShadowSemanticBaseline {
        candidate_count: semantic_candidates.len(),
        ranked_sources: semantic_candidates
            .iter()
            .take(MAX_SEMANTIC_RRF_CANDIDATES)
            .map(|candidate| candidate.source.clone())
            .collect(),
    };
    let shadow_semantic = build_shadow_semantic_explain(
        conn,
        Some(query_vector),
        query_text,
        ctx,
        source_prefix,
        top_k,
        Some(&baseline),
    );
    if let Some(reason) = shadow_guard_failure_reason(&shadow_semantic) {
        return (
            semantic_candidates,
            baseline_route(reason, sampled, active_trial_percent),
        );
    }
    let shadow_sources = parse_shadow_sources(&shadow_semantic, "shadowTopSources");
    if shadow_sources.is_empty() {
        return (
            semantic_candidates,
            baseline_route("shadow_top_sources_empty", sampled, active_trial_percent),
        );
    }
    let mut by_source: HashMap<String, SemanticCandidate> = semantic_candidates
        .iter()
        .cloned()
        .map(|candidate| (candidate.source.clone(), candidate))
        .collect();
    let mut reordered: Vec<SemanticCandidate> = Vec::new();
    let baseline_max = semantic_candidates
        .first()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SCALE_BASE);
    let baseline_min = semantic_candidates
        .last()
        .map(|candidate| candidate.relevance)
        .unwrap_or(SEMANTIC_SIM_FLOOR);
    let relevance_span = (baseline_max - baseline_min).abs().max(0.02);
    let rank_denominator = shadow_sources.len().saturating_sub(1).max(1) as f64;
    let fallback_relevance_for_rank = |rank_idx: usize| {
        let rank_weight = 1.0 - (rank_idx as f64 / rank_denominator);
        round4(
            (baseline_min + (relevance_span * rank_weight))
                .clamp(SEMANTIC_SIM_FLOOR, baseline_max.max(SEMANTIC_SIM_FLOOR)),
        )
    };
    for (rank_idx, source) in shadow_sources.iter().enumerate() {
        if let Some(candidate) = by_source.remove(source) {
            reordered.push(candidate);
            continue;
        }
        let fallback_relevance = fallback_relevance_for_rank(rank_idx);
        if let Some(candidate) =
            sqlite_vec_source_fallback_candidate(conn, source, query_text, fallback_relevance)
        {
            reordered.push(candidate);
            continue;
        }
        reordered.push(SemanticCandidate {
            source: source.clone(),
            excerpt: query_focused_excerpt(source, query_text, 160),
            relevance: fallback_relevance,
            importance: 0.5,
            ts: 0,
        });
    }
    for candidate in &semantic_candidates {
        if let Some(remaining) = by_source.remove(&candidate.source) {
            reordered.push(remaining);
        }
    }
    reordered.truncate(semantic_candidates.len());
    (
        reordered,
        json!({"mode":if matches!(effective_route_mode,SqliteVecRouteMode::Primary){"vec0_primary"}else{"vec0_trial"},"reason":if matches!(effective_route_mode,SqliteVecRouteMode::Primary){"route_mode_primary"}else{"guard_passed"},"sampled":sampled,"trialPercent":active_trial_percent,"routeMode":route_mode}),
    )
}
pub(crate) fn is_benchmark_recall_scope(agent: &str, source_prefix: Option<&str>) -> bool {
    if agent
        .trim()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_AGENT_PREFIX)
    {
        return true;
    }
    source_prefix
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_SCOPE_PREFIX)
}
