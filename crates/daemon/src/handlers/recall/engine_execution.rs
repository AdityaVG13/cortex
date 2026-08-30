pub(crate) async fn emit_recall_query_event(state: &RuntimeState, agent: &str, source_prefix: Option<&str>, payload: Value) {
    if is_benchmark_recall_scope(agent, source_prefix) {
        return;
    }
    let conn = state.db.lock().await;
    if crate::handlers::log_event(&conn, "recall_query", payload, agent).is_ok() {
        checkpoint_wal_best_effort(&conn);
    }
}
pub(crate) fn build_method_breakdown(results: &[RecallItem]) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for item in results {
        *counts.entry(item.method.clone()).or_insert(0) += 1;
    }
    json!(counts)
}
pub(crate) fn method_count(methods: &Value, method: &str) -> i64 {
    methods.get(method).and_then(|v| v.as_i64()).unwrap_or(0)
}
pub(crate) fn classify_recall_tier(cached: bool, mode: &str, methods: &Value) -> &'static str {
    if cached {
        return "cache_hit";
    }
    if mode == "headlines" {
        return "headlines";
    }
    if mode == "semantic" {
        return "semantic_only";
    }
    let keyword = method_count(methods, "keyword");
    let semantic = method_count(methods, "semantic");
    let hybrid = method_count(methods, "hybrid");
    let crystal = method_count(methods, "crystal");
    let associative = method_count(methods, "associative");
    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        if crystal > 0 {
            return "hybrid_crystal";
        }
        return "hybrid_fusion";
    }
    if associative > 0 && (keyword > 0 || semantic > 0 || crystal > 0) {
        return "associative_blend";
    }
    if keyword > 0 {
        if crystal > 0 {
            return "keyword_crystal";
        }
        return "keyword_only";
    }
    if semantic > 0 {
        if crystal > 0 {
            return "semantic_crystal";
        }
        return "semantic_only";
    }
    if crystal > 0 {
        return "crystal_only";
    }
    if associative > 0 {
        return "associative_only";
    }
    "unknown"
}
pub(crate) fn run_budget_recall(
    conn: &mut Connection, query_text: &str, token_budget: usize, k: usize, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    run_budget_recall_with_engine(conn, query_text, token_budget, k, ctx, source_prefix, None)
}
#[allow(dead_code)]
pub(crate) fn run_semantic_recall_with_query_vector(
    conn: &Connection, query_text: &str, k: usize, query_vector: Option<&[f32]>, ctx: &RecallContext, source_prefix: Option<&str>,
    _canary: Option<&SqliteVecCanaryConfig>, _sqlite_vec_shadow_enabled: bool,
) -> (Vec<RecallItem>, Value) {
    let _ = query_vector;
    match run_clock_quorum_recall(conn, query_text, 0, k, ctx, source_prefix) {
        Ok(ranked) => (ranked, json!({"engine":"clock-quorum","modelFree":true})),
        Err(_) => (Vec::new(), json!({"engine":"clock-quorum","modelFree":true,"error":true})),
    }
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
    source: &str, excerpt: &str, query_text: &str, char_cap: usize, remaining_tokens: usize,
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
pub(crate) fn prefer_family_candidate(candidate: &RecallItem, current: &RecallItem, alignment_profile: &QueryAlignmentProfile) -> bool {
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
    candidates: Vec<RecallItem>, query_text: &str, token_budget: usize,
) -> (Vec<RecallItem>, Vec<RecallItem>, Vec<RecallFamilyCompaction>) {
    if token_budget > 400 || candidates.len() <= 1 {
        return (candidates, Vec::new(), Vec::new());
    }
    let mut family_lookup = HashMap::new();
    for item in &candidates {
        if item.family_members.is_empty() {
            continue;
        }
        for member in &item.family_members {
            family_lookup.entry(member.clone()).or_insert_with(|| item.source.clone());
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
        let family_key =
            if !item.family_members.is_empty() { item.source.clone() } else { family_lookup.get(&item.source).cloned().unwrap_or_else(|| item.source.clone()) };
        match compacted.entry(family_key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if prefer_family_candidate(&item, entry.get(), &alignment_profile) {
                    let replaced = entry.insert(item);
                    dropped_by_family.entry(entry.key().clone()).or_default().push(replaced.source.clone());
                    dropped.push(replaced);
                } else {
                    dropped_by_family.entry(entry.key().clone()).or_default().push(item.source.clone());
                    dropped.push(item);
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(item);
            }
        }
    }
    dropped.sort_by(|a, b| compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source));
    let mut family_compactions = Vec::new();
    for (family_key, mut dropped_sources) in dropped_by_family {
        if dropped_sources.is_empty() {
            continue;
        }
        dedup_preserve_order(&mut dropped_sources);
        let Some(kept_source) = compacted.get(&family_key).map(|item| item.source.clone()) else {
            continue;
        };
        family_compactions.push(RecallFamilyCompaction { family_key, kept_source, dropped_sources });
    }
    family_compactions.sort_by(|a, b| a.family_key.cmp(&b.family_key));
    let mut compacted_items: Vec<RecallItem> = compacted.into_values().collect();
    compacted_items.sort_by(|a, b| compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source));
    (compacted_items, dropped, family_compactions)
}
pub(crate) fn compact_budget_family_candidates(candidates: Vec<RecallItem>, query_text: &str, token_budget: usize) -> Vec<RecallItem> {
    compact_budget_family_candidates_with_trace(candidates, query_text, token_budget).0
}
pub(crate) fn apply_semantic_budget(raw: Vec<RecallItem>, token_budget: usize, query_text: &str) -> Vec<RecallItem> {
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
    let mut candidates: Vec<RecallItem> = raw.iter().filter(|item| item.relevance >= min_relevance).take(max_items).cloned().collect();
    if candidates.is_empty() {
        candidates = raw.iter().take(max_items.max(1)).cloned().collect();
    }
    let query_terms: HashSet<String> = query_focus_terms_for_excerpt(query_text).into_iter().collect();
    let mut covered_terms: HashSet<String> = HashSet::new();
    let mut selected_signatures: Vec<HashSet<String>> = Vec::new();
    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, mut item) in candidates.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text).min((remaining as f64 * 3.6) as usize).max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) = fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining) {
            let signature_terms = excerpt_signature_terms(&item.source, &excerpt);
            if should_skip_redundant_budget_candidate(&signature_terms, &selected_signatures, &query_terms, &covered_terms) {
                continue;
            }
            item.excerpt = excerpt;
            item.tokens = Some(tokens);
            spent += tokens;
            update_query_term_coverage(&signature_terms, &query_terms, &mut covered_terms);
            selected_signatures.push(signature_terms);
            budgeted.push(item);
            if should_early_stop_budget_selection(token_budget, spent, budgeted.len(), &query_terms, &covered_terms) {
                break;
            }
        }
    }
    budgeted
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
    conn: &Connection, query_text: &str, token_budget: usize, k: usize, query_vector: Option<&[f32]>, ctx: &RecallContext, source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>, sqlite_vec_shadow_enabled: bool,
) -> Result<RecallBudgetTrace, String> {
    let retrieval_depth = if token_budget <= 220 {
        (k.max(10) * 3).min(30)
    } else if token_budget <= 400 {
        (k.max(10) * 2).min(28)
    } else {
        k.max(12)
    };
    let recall_trace =
        run_recall_with_query_vector_trace(conn, query_text, retrieval_depth, query_vector, ctx, source_prefix, canary, sqlite_vec_shadow_enabled)?;
    let raw = recall_trace.ranked;
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
            semantic_route,
        });
    }
    let pre_compaction_pool = raw;
    let pre_compaction_candidate_count = pre_compaction_pool.len();
    let (raw, _family_compaction_dropped, family_compactions) = compact_budget_family_candidates_with_trace(pre_compaction_pool, query_text, token_budget);
    let top_relevance = raw.first().map(|item| item.relevance).unwrap_or(0.0);
    let min_relevance = semantic_budget_min_relevance(top_relevance, query_text);
    let max_items = semantic_budget_max_items(token_budget, query_text, k.max(1));
    let mut candidates: Vec<RecallItem> = raw.iter().filter(|item| item.relevance >= min_relevance).take(max_items).cloned().collect();
    if candidates.is_empty() {
        candidates = raw.iter().take(max_items).cloned().collect();
    }
    let query_terms: HashSet<String> = query_focus_terms_for_excerpt(query_text).into_iter().collect();
    let mut covered_terms: HashSet<String> = HashSet::new();
    let mut selected_signatures: Vec<HashSet<String>> = Vec::new();
    let mut spent = 0usize;
    let mut budgeted = Vec::new();
    for (idx, mut item) in candidates.into_iter().enumerate() {
        let remaining = token_budget.saturating_sub(spent);
        if remaining <= 10 {
            break;
        }
        let cap = budget_rank_char_cap(token_budget, idx, query_text).min((remaining as f64 * 3.6) as usize).max(MIN_EXCERPT_CHARS);
        if let Some((excerpt, tokens)) = fit_excerpt_to_remaining_budget(&item.source, &item.excerpt, query_text, cap, remaining) {
            let signature_terms = excerpt_signature_terms(&item.source, &excerpt);
            if should_skip_redundant_budget_candidate(&signature_terms, &selected_signatures, &query_terms, &covered_terms) {
                continue;
            }
            item.excerpt = excerpt;
            item.tokens = Some(tokens);
            spent += tokens;
            update_query_term_coverage(&signature_terms, &query_terms, &mut covered_terms);
            selected_signatures.push(signature_terms);
            budgeted.push(item);
            if should_early_stop_budget_selection(token_budget, spent, budgeted.len(), &query_terms, &covered_terms) {
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
        semantic_route,
    })
}
pub(crate) fn run_budget_recall_with_engine(
    conn: &mut Connection, query_text: &str, token_budget: usize, k: usize, ctx: &RecallContext, source_prefix: Option<&str>,
    _degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let trace = run_budget_recall_trace_with_query_vector(conn, query_text, token_budget, k, None, ctx, source_prefix, None, false)?;
    bump_retrievals_batch(conn, &trace.budgeted);
    Ok(trace.budgeted)
}
pub(crate) fn run_recall(
    conn: &mut Connection, query_text: &str, k: usize, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    let trace = run_recall_with_query_vector_trace(conn, query_text, k, None, ctx, source_prefix, None, false)?;
    bump_retrievals_batch(conn, &trace.ranked);
    Ok(trace.ranked)
}
pub async fn execute_unified_recall(
    state: &RuntimeState, query_text: &str, budget: usize, k: usize, agent: &str, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Value, String> {
    let started_at = Instant::now();
    let policy_mode = recall_mode_for_budget(budget);
    let latency_budget_ms = recall_latency_budget_ms_for_mode(policy_mode);
    let (mut results, semantic_route, fail_closed) = {
        let conn = state.db_read.lock().await;
        let (mut results, mut semantic_route) = if budget == 0 {
            let trace = run_recall_with_query_vector_trace(&conn, query_text, k, None, ctx, source_prefix, Some(&state.sqlite_vec_canary), false)?;
            (trace.ranked, trace.semantic_route)
        } else {
            let trace = run_budget_recall_trace_with_query_vector(
                &conn, query_text, budget, k, None, ctx, source_prefix, Some(&state.sqlite_vec_canary), false,
            )?;
            (trace.budgeted, trace.semantic_route)
        };
        let mut fail_closed = Value::Null;
        if budget > 0 {
            let elapsed_before_fallback = started_at.elapsed().as_millis();
            if elapsed_before_fallback >= latency_budget_ms {
                let fallback_trace = run_budget_recall_trace_with_query_vector(
                    &conn, query_text, budget, k, None, ctx, source_prefix, Some(&state.sqlite_vec_canary), false,
                )?;
                results = fallback_trace.budgeted;
                semantic_route = json!({
                    "engine": "clock-quorum",
                    "modelFree": true,
                    "reason": "latency_budget_fail_closed",
                    "elapsedMsBeforeFallback": elapsed_before_fallback,
                    "latencyBudgetMs": latency_budget_ms
                });
                fail_closed = json!({
                    "triggered": true,
                    "elapsedMsBeforeFallback": elapsed_before_fallback,
                    "latencyBudgetMs": latency_budget_ms,
                    "fallback": "clock_quorum"
                });
            }
        }
        (results, semantic_route, fail_closed)
    };
    let shadow_semantic = json!({"status": "skipped", "reason": "model_free"});
    let rerank_route = json!({"status":"skipped","reason":"model_free","mode":"off"});
    {
        let conn = state.db.lock().await;
        bump_retrievals_batch(&conn, &results);
    }
    if budget == 0 {
        let method_breakdown = build_method_breakdown(&results);
        let tier = classify_recall_tier(false, "headlines", &method_breakdown);
        let latency_ms = started_at.elapsed().as_millis() as i64;
        let headlines = results
            .iter()
            .map(|item| json!({"source": item.source, "relevance": item.relevance, "method": item.method}))
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
    // CQR must be byte-stable against an unchanged database. Served-content
    // filtering would drop the same excerpt on a repeat query.
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
    Ok(json!({
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
    }))
}
pub async fn execute_recall_policy_explain(
    state: &RuntimeState, query_text: &str, budget: usize, k: usize, agent: &str, ctx: &RecallContext, source_prefix: Option<&str>, pool_k: usize,
    _query_vector_override: Option<&[f32]>,
) -> Result<Value, String> {
    let requested_k = k.max(1);
    let pool_k = pool_k.max(requested_k).min(128);
    let conn = state.db_read.lock().await;
    let (
        budgeted,
        candidate_pool,
        pre_compaction_candidate_count,
        family_compactions,
        retrieval_depth,
        min_relevance,
        top_relevance,
        max_items,
        semantic_route,
    ) = if budget == 0 {
        let trace = run_recall_with_query_vector_trace(&conn, query_text, pool_k, None, ctx, source_prefix, Some(&state.sqlite_vec_canary), true)?;
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
        (budgeted, raw_pool, raw_pool_len, Vec::new(), pool_k, 0.0_f64, 0.0_f64, requested_k, trace.semantic_route)
    } else {
        let trace = run_budget_recall_trace_with_query_vector(
            &conn, query_text, budget, requested_k, None, ctx, source_prefix, Some(&state.sqlite_vec_canary), true,
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
            trace.semantic_route,
        )
    };
    let shadow_semantic = json!({"enabled":false,"status":"skipped","reason":"model_free","topK":pool_k});
    drop(conn);
    let rerank_route = json!({"status":"skipped","reason":"model_free","mode":"off"});
    let _ = agent;
    let final_results = enforce_budget_token_invariant(budgeted, budget, query_text);
    let usage = compute_recall_budget_usage(&final_results, budget);
    let mode = recall_mode_for_budget(budget);
    let family_compacted_count: usize = family_compactions.iter().map(|entry| entry.dropped_sources.len()).sum();
    let family_compactions_json: Vec<Value> = family_compactions
        .iter()
        .map(|entry| json!({"familyKey":entry.family_key,"keptSource":entry.kept_source,"droppedSources":entry.dropped_sources,}))
        .collect();
    let returned_sources: HashSet<&str> = final_results.iter().map(|item| item.source.as_str()).collect();
    let dropped_candidates:Vec<Value>=candidate_pool.iter().filter(|item|!returned_sources.contains(item.source.as_str())).take(24).map(|item|{let estimated_tokens=estimate_tokens(&format!("{}{}",item.source,item.excerpt));json!({"source":item.source,"relevance":item.relevance,"method":item.method,"estimatedTokens":estimated_tokens,"reason":"not_selected_under_current_budget_or_rank_cutoff"})}).collect();
    let query_entities = query_entity_terms(query_text);
    let mut entity_metrics_by_source: HashMap<String, (usize, f64, f64)> = HashMap::new();
    for candidate in &candidate_pool {
        let haystack = format!("{} {}", candidate.source, candidate.excerpt);
        let (entity_matches, entity_overlap) = entity_alignment_metrics_with_terms(&haystack, &query_entities);
        let entity_boost = entity_signal_boost(entity_matches, entity_overlap);
        entity_metrics_by_source.insert(candidate.source.clone(), (entity_matches, round4(entity_overlap), round4(entity_boost)));
    }
    let final_with_factors:Vec<Value>=final_results.clone().into_iter().enumerate().map(|(idx,item)|{let tokens=item.tokens.unwrap_or_else(||estimate_tokens(&format!("{}{}",item.source,item.excerpt)));let budget_ratio=if budget==0{0.0}else{((tokens as f64)/(budget as f64)).min(1.0)};let(entity_matches,entity_overlap,entity_boost)=entity_metrics_by_source.get(&item.source).copied().unwrap_or_else(||{let haystack=format!("{} {}",item.source,item.excerpt);let(matches,overlap)=entity_alignment_metrics_with_terms(&haystack,&query_entities);(matches,round4(overlap),round4(entity_signal_boost(matches,overlap)),)});json!({"rank":idx+1,"source":item.source,"relevance":item.relevance,"method":item.method,"tokens":tokens,"rankingFactors":{"relevance":item.relevance,"method":item.method,"tokenCost":tokens,"budgetCostRatio":round4(budget_ratio),"entropy":item.entropy,"entityMatches":entity_matches,"entityOverlap":entity_overlap,"entityBoost":entity_boost}})}).collect();
    let post_compaction_dropped_count = candidate_pool.len().saturating_sub(final_with_factors.len());
    Ok(
        json!({"query":query_text,"results":final_results.into_iter().map(recall_to_json).collect::<Vec<_>>(),"budget":budget,"spent":usage.spent,"saved":usage.saved,"overBudget":usage.over_budget,"tokenUsageLine":format_recall_token_usage_line(budget,usage),"mode":mode.as_str(),"policyMode":mode.as_str(),"policy":{"name":"adaptive-recall-policy","mode":mode.as_str(),"budget":budget,"requestedK":requested_k,"poolK":pool_k,"retrievalDepth":retrieval_depth,"candidateCutoff":{"topRelevance":round4(top_relevance),"minRelevance":round4(min_relevance),"maxItemsBeforeBudget":max_items},"budgetReasoning":{"requestedBudget":budget,"spent":usage.spent,"saved":usage.saved,"budgetPressure":if budget==0{0.0}else{round4((usage.spent as f64)/(budget as f64))},"candidateCountBeforeFamilyCompaction":pre_compaction_candidate_count,"candidateCount":candidate_pool.len(),"candidateCountAfterFamilyCompaction":candidate_pool.len(),"familyCompactedCount":family_compacted_count,"returnedCount":final_with_factors.len(),"droppedCount":post_compaction_dropped_count,"totalPreBudgetDrops":family_compacted_count+post_compaction_dropped_count},"semanticRoute":semantic_route,"rerankRoute":rerank_route.clone()},"explain":{"returned":final_with_factors,"familyCompactions":family_compactions_json,"droppedCandidates":dropped_candidates,"shadowSemantic":shadow_semantic,"rerank":rerank_route}}),
    )
}
pub async fn execute_semantic_recall(
    state: &RuntimeState, query_text: &str, budget: usize, k: usize, agent: &str, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Result<Value, String> {
    let started_at = Instant::now();
    let semantic_available = true;
    let (budgeted, semantic_route) = {
        let conn = state.db_read.lock().await;
        let results = run_clock_quorum_recall(&conn, query_text, budget, k, ctx, source_prefix)?;
        (results, json!({"engine":"clock-quorum","modelFree":true}))
    };
    {
        let conn = state.db.lock().await;
        bump_retrievals_batch(&conn, &budgeted);
    }
    let budgeted = enforce_budget_token_invariant(budgeted, budget, query_text);
    let usage = compute_recall_budget_usage(&budgeted, budget);
    let mode = "semantic";
    let method_breakdown = build_method_breakdown(&budgeted);
    let tier = classify_recall_tier(false, mode, &method_breakdown);
    let latency_ms = started_at.elapsed().as_millis() as i64;
    emit_recall_query_event(state,agent,source_prefix,json!({"agent":agent,"query":truncate_chars(query_text,120),"mode":mode,"k":k,"budget":budget,"spent":usage.spent,"saved":usage.saved,"over_budget":usage.over_budget,"hits":budgeted.len(),"results":budgeted.len(),"semantic_available":semantic_available,"cached":false,"method_breakdown":method_breakdown,"tier":tier,"latency_ms":latency_ms,"semantic_route":semantic_route.clone(),}),).await;
    Ok(
        json!({"results":budgeted.into_iter().map(recall_to_json).collect::<Vec<_>>(),"mode":"semantic","budget":budget,"spent":usage.spent,"saved":usage.saved,"overBudget":usage.over_budget,"tokenUsageLine":format_recall_token_usage_line(budget,usage),"semanticAvailable":semantic_available,"semanticRoute":semantic_route,"tier":tier,"latencyMs":latency_ms,}),
    )
}
#[allow(clippy::type_complexity)]
pub(crate) fn run_recall_with_query_vector_trace(
    conn: &Connection, query_text: &str, k: usize, query_vector: Option<&[f32]>, ctx: &RecallContext, source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>, sqlite_vec_shadow_enabled: bool,
) -> Result<RecallWithVectorTrace, String> {
    let ranked = run_clock_quorum_recall(conn, query_text, 0, k, ctx, source_prefix)?;
    let _ = (query_vector, canary, sqlite_vec_shadow_enabled);
    Ok(RecallWithVectorTrace {
        ranked,
        semantic_route: json!({"engine":"clock-quorum","modelFree":true}),
    })
}
pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Some(crystal_id) = parse_crystal_source_id(source) {
        if let Some((label, consolidated_text, member_count, owner_id, visibility)) = query_crystal_for_unfold(conn, crystal_id) {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                let members = crystal_member_sources(conn, crystal_id, ctx);
                let mut full_text = consolidated_text.clone();
                if !members.is_empty() {
                    full_text.push_str("\n\nFamily members:\n");
                    for member in members.iter().take(16) {
                        full_text.push_str("- ");
                        full_text.push_str(member);
                        full_text.push('\n');
                    }
                    if member_count as usize > members.len() {
                        full_text.push_str(&format!("... plus {} more hidden or archived member(s)", (member_count as usize).saturating_sub(members.len())));
                    }
                }
                return Some(
                    json!({"source":crystal_source(crystal_id,&label),"text":full_text.trim_end().to_string(),"type":"crystal","label":label,"clusterId":crystal_id,"members":members,"memberCount":member_count,}),
                );
            }
        }
    }
    if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, source) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text":text,"type":ty}));
        }
    }
    if let Some(id_str) = source.strip_prefix("decision::") {
        if let Ok(id) = id_str.parse::<i64>() {
            if let Some((decision, context, owner_id, visibility)) = query_decision_by_id_for_unfold(conn, id) {
                if is_visible(owner_id, visibility.as_deref(), ctx) {
                    let full = match context {
                        Some(c) => format!("{decision}\n\nContext: {c}"),
                        None => decision,
                    };
                    return Some(json!({"text":full,"type":"decision"}));
                }
            }
        }
    }
    if let Some((decision, context, owner_id, visibility)) = query_decision_by_context_for_unfold(conn, source) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            let full = match context {
                Some(c) => format!("{decision}\n\nContext: {c}"),
                None => decision,
            };
            return Some(json!({"text":full,"type":"decision"}));
        }
    }
    let stripped = source.strip_prefix("memory::").unwrap_or(source);
    if stripped != source {
        if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, stripped) {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                return Some(json!({"text":text,"type":ty}));
            }
        }
    }
    None
}
const UNFOLD_ACTIVE:&str="status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) AND (valid_from IS NULL OR valid_from <= datetime('now')) AND (valid_until IS NULL OR valid_until > datetime('now'))";
pub(crate) type MemoryUnfoldRow = (String, String, Option<i64>, Option<String>);
pub(crate) type DecisionUnfoldRow = (String, Option<String>, Option<i64>, Option<String>);
fn query_acl_row<T, F, G>(conn: &Connection, with_sql: &str, without_sql: &str, bind: &[&dyn rusqlite::types::ToSql], map_with: F, map_without: G) -> Option<T>
where
    F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    G: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    match conn.query_row(with_sql, bind, map_with) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn.query_row(without_sql, bind, map_without).ok(),
        Err(_) => None,
    }
}
pub(crate) fn query_memory_for_unfold(conn: &Connection, source: &str) -> Option<MemoryUnfoldRow> {
    let bind: Vec<&dyn rusqlite::types::ToSql> = vec![&source];
    query_acl_row(
        conn,
        &format!("SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 AND {UNFOLD_ACTIVE} ORDER BY score DESC LIMIT 1"),
        &format!("SELECT text, type FROM memories WHERE source = ?1 AND {UNFOLD_ACTIVE} ORDER BY score DESC LIMIT 1"),
        &bind,
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        |row| Ok((row.get(0)?, row.get(1)?, None, None)),
    )
}
fn query_decision_for_unfold(conn: &Connection, predicate: &str, bind: &[&dyn rusqlite::types::ToSql], order_limit: &str) -> Option<DecisionUnfoldRow> {
    query_acl_row(
        conn,
        &format!("SELECT decision, context, owner_id, visibility FROM decisions WHERE {predicate} AND {UNFOLD_ACTIVE}{order_limit}"),
        &format!("SELECT decision, context FROM decisions WHERE {predicate} AND {UNFOLD_ACTIVE}{order_limit}"),
        bind,
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        |row| Ok((row.get(0)?, row.get(1)?, None, None)),
    )
}
pub(crate) fn query_decision_by_id_for_unfold(conn: &Connection, id: i64) -> Option<DecisionUnfoldRow> {
    query_decision_for_unfold(conn, "id = ?1", &[&id], "")
}
pub(crate) fn query_decision_by_context_for_unfold(conn: &Connection, source: &str) -> Option<DecisionUnfoldRow> {
    query_decision_for_unfold(conn, "context = ?1", &[&source], " ORDER BY score DESC LIMIT 1")
}
