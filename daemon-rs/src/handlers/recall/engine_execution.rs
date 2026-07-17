pub(crate) async fn emit_recall_query_event(
    state: &RuntimeState,
    agent: &str,
    source_prefix: Option<&str>,
    payload: Value,
) {
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
pub(crate) fn shadow_semantic_telemetry_summary(shadow_semantic: &Value) -> Value {
    let status = shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    let mut summary = json!({
        "status": status,
    });
    if let Some(reason) = shadow_semantic.get("reason").and_then(Value::as_str) {
        summary["reason"] = json!(reason);
    }
    for key in [
        "topK",
        "vectorDimension",
        "baselineCandidateCount",
        "shadowCandidateCount",
        "overlapCount",
        "overlapRatio",
        "jaccard",
        "matchedRankPairs",
        "meanAbsRankDelta",
        "top1Match",
    ] {
        if let Some(value) = shadow_semantic.get(key) {
            summary[key] = value.clone();
        }
    }
    if status == "error" && summary.get("reason").is_none() {
        summary["reason"] = json!("shadow_payload_invalid");
    }
    summary
}
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
    apply_recall_ranking_boosts(&mut ranked, query_text, 0.05, 0.08);
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
pub(crate) fn run_recall(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    run_recall_with_engine(conn, query_text, k, None, ctx, source_prefix, None)
}
#[allow(clippy::type_complexity)]
pub(crate) fn run_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    engine: Option<&crate::embeddings::EmbeddingEngine>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let query_vector = engine.and_then(|engine| engine.embed_query(query_text));
    if engine.is_some() {
        update_semantic_search_health(degraded_flag, query_vector.is_some(), true);
    }
    Ok(run_recall_with_query_vector_trace(
        conn,
        query_text,
        k,
        query_vector.as_deref(),
        ctx,
        source_prefix,
        None,
    )?
    .ranked)
}
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
pub async fn execute_recall_policy_explain(
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
    // This function is the retrieval engine; caching is the caller's responsibility.
    // and should always surface regardless of FTS confidence.
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
                    relevance: scale_semantic_similarity(relevance as f32),
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
    // Run FTS5 first. If the top result is confident (score >= 0.93) with a
    // meaningful gap from #2 (delta >= 0.08), return immediately without
    // spending cycles on embedding inference. Target: 40%+ queries resolved here.
    const TIER2_CONFIDENCE: f64 = 0.78;
    const TIER2_GAP: f64 = 0.10;
    let raw_k = if ctx.team_mode { k.max(10) * 5 } else { 20 };
    let mut fts_limit = raw_k.max(20);
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
    // Assign stable integer indices to each unique source across both lists,
    // then fuse ranks. rrf_fuse() works on (i64, f64) so we map source → index.
    //
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
    // High-entropy (information-dense) excerpts get a relevance boost (+/-15%
    // around midpoint H=3.5). Applied after compound scoring so entropy acts as
    // a diversity signal on top of the RRF+compound base.
    let mut ranked: Vec<RecallItem> = merged.into_values().collect();
    apply_recall_ranking_boosts(&mut ranked, query_text, 0.08, 0.12);
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
pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Some(crystal_id) = parse_crystal_source_id(source) {
        if let Some((label, consolidated_text, member_count, owner_id, visibility)) =
            query_crystal_for_unfold(conn, crystal_id)
        {
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
                        full_text.push_str(&format!(
                            "... plus {} more hidden or archived member(s)",
                            (member_count as usize).saturating_sub(members.len())
                        ));
                    }
                }
                return Some(json!({
                    "source": crystal_source(crystal_id, &label),
                    "text": full_text.trim_end().to_string(),
                    "type": "crystal",
                    "label": label,
                    "clusterId": crystal_id,
                    "members": members,
                    "memberCount": member_count,
                }));
            }
        }
    }
    if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, source) {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            return Some(json!({"text": text, "type": ty}));
        }
    }
    if let Some(id_str) = source.strip_prefix("decision::") {
        if let Ok(id) = id_str.parse::<i64>() {
            if let Some((decision, context, owner_id, visibility)) =
                query_decision_by_id_for_unfold(conn, id)
            {
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
    if let Some((decision, context, owner_id, visibility)) =
        query_decision_by_context_for_unfold(conn, source)
    {
        if is_visible(owner_id, visibility.as_deref(), ctx) {
            let full = match context {
                Some(c) => format!("{decision}\n\nContext: {c}"),
                None => decision,
            };
            return Some(json!({"text": full, "type": "decision"}));
        }
    }
    let stripped = source.strip_prefix("memory::").unwrap_or(source);
    if stripped != source {
        if let Some((text, ty, owner_id, visibility)) = query_memory_for_unfold(conn, stripped) {
            if is_visible(owner_id, visibility.as_deref(), ctx) {
                return Some(json!({"text": text, "type": ty}));
            }
        }
    }
    None
}
pub(crate) type MemoryUnfoldRow = (String, String, Option<i64>, Option<String>);
pub(crate) type DecisionUnfoldRow = (String, Option<String>, Option<i64>, Option<String>);
pub(crate) fn query_memory_for_unfold(conn: &Connection, source: &str) -> Option<MemoryUnfoldRow> {
    let sql_with_visibility =
        "SELECT text, type, owner_id, visibility FROM memories WHERE source = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT text, type FROM memories WHERE source = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn query_decision_by_id_for_unfold(conn: &Connection, id: i64) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE id = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))";
    match conn.query_row(sql_with_visibility, params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE id = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now'))",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
pub(crate) fn query_decision_by_context_for_unfold(
    conn: &Connection,
    source: &str,
) -> Option<DecisionUnfoldRow> {
    let sql_with_visibility =
        "SELECT decision, context, owner_id, visibility FROM decisions WHERE context = ?1 \
         AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
         ORDER BY score DESC LIMIT 1";
    match conn.query_row(sql_with_visibility, params![source], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE context = ?1 \
                 AND status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now')) \
             AND (valid_from IS NULL OR valid_from <= datetime('now')) \
             AND (valid_until IS NULL OR valid_until > datetime('now')) \
                 ORDER BY score DESC LIMIT 1",
                params![source],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                        None,
                    ))
                },
            )
            .ok(),
        Err(_) => None,
    }
}
