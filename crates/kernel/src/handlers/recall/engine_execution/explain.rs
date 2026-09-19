use super::super::{
    RecallContext, compute_recall_budget_usage, enforce_budget_token_invariant,
    entity_alignment_metrics_with_terms, entity_signal_boost, format_recall_token_usage_line,
    query_entity_terms, recall_mode_for_budget, recall_to_json, round4,
};
use super::{run_budget_recall_trace_with_query_vector, run_recall_with_query_vector_trace};
use crate::handlers::estimate_tokens;
use crate::state::RuntimeState;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub async fn execute_recall_policy_explain(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    pool_k: usize,
    _query_vector_override: Option<&[f32]>,
) -> Result<Value, String> {
    let requested_k = k.max(1);
    let pool_k = pool_k.max(requested_k).min(128);
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
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
        let trace = run_recall_with_query_vector_trace(
            &conn,
            query_text,
            pool_k,
            None,
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
            true,
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
            trace.semantic_route,
        )
    } else {
        let trace = run_budget_recall_trace_with_query_vector(
            &conn,
            query_text,
            budget,
            requested_k,
            None,
            ctx,
            source_prefix,
            Some(&state.sqlite_vec_canary),
            true,
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
    let shadow_semantic =
        json!({"enabled":false,"status":"skipped","reason":"model_free","topK":pool_k});
    drop(conn);
    let rerank_route = json!({"status":"skipped","reason":"model_free","mode":"off"});
    let _ = agent;
    let final_results = enforce_budget_token_invariant(budgeted, budget, query_text);
    let usage = compute_recall_budget_usage(&final_results, budget);
    let mode = recall_mode_for_budget(budget);
    let family_compacted_count: usize = family_compactions
        .iter()
        .map(|entry| entry.dropped_sources.len())
        .sum();
    let family_compactions_json: Vec<Value> = family_compactions.iter().map(|entry| json!({"familyKey":entry.family_key,"keptSource":entry.kept_source,"droppedSources":entry.dropped_sources,})).collect();
    let returned_sources: HashSet<&str> = final_results
        .iter()
        .map(|item| item.source.as_str())
        .collect();
    let dropped_candidates: Vec<Value> = candidate_pool.iter().filter(|item| !returned_sources.contains(item.source.as_str())).take(24).map(|item| {
            let estimated_tokens = estimate_tokens(&format!("{}{}", item.source, item.excerpt));
            json!({"source":item.source,"relevance":item.relevance,"method":item.method,"estimatedTokens":estimated_tokens,"reason":"not_selected_under_current_budget_or_rank_cutoff"})
        }).collect();
    let query_entities = query_entity_terms(query_text);
    let mut entity_metrics_by_source: HashMap<String, (usize, f64, f64)> = HashMap::new();
    for candidate in &candidate_pool {
        let haystack = format!("{} {}", candidate.source, candidate.excerpt);
        let (entity_matches, entity_overlap) =
            entity_alignment_metrics_with_terms(&haystack, &query_entities);
        entity_metrics_by_source.insert(
            candidate.source.clone(),
            (
                entity_matches,
                round4(entity_overlap),
                round4(entity_signal_boost(entity_matches, entity_overlap)),
            ),
        );
    }
    let final_with_factors: Vec<Value> = final_results.clone().into_iter().enumerate().map(|(idx, item)| {
            let tokens = item.tokens.unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)));
            let budget_ratio = if budget == 0 { 0.0 } else { ((tokens as f64) / (budget as f64)).min(1.0) };
            let (entity_matches, entity_overlap, entity_boost) = entity_metrics_by_source.get(&item.source).copied().unwrap_or_else(|| {
                    let haystack = format!("{} {}", item.source, item.excerpt);
                    let (matches, overlap) = entity_alignment_metrics_with_terms(&haystack, &query_entities);
                    (matches, round4(overlap), round4(entity_signal_boost(matches, overlap)))
                });
            json!({"rank":idx+1,"source":item.source,"relevance":item.relevance,"method":item.method,"tokens":tokens,"rankingFactors":{"relevance":item.relevance,"method":item.method,"tokenCost":tokens,"budgetCostRatio":round4(budget_ratio),"entropy":item.entropy,"entityMatches":entity_matches,"entityOverlap":entity_overlap,"entityBoost":entity_boost}})
        }).collect();
    let post_compaction_dropped_count = candidate_pool
        .len()
        .saturating_sub(final_with_factors.len());
    Ok(
        json!({"query":query_text,"results":final_results.into_iter().map(recall_to_json).collect::<Vec<_>>(),"budget":budget,"spent":usage.spent,"saved":usage.saved,"overBudget":usage.over_budget,"tokenUsageLine":format_recall_token_usage_line(budget,usage),"mode":mode.as_str(),"policyMode":mode.as_str(),"policy":{"name":"adaptive-recall-policy","mode":mode.as_str(),"budget":budget,"requestedK":requested_k,"poolK":pool_k,"retrievalDepth":retrieval_depth,"candidateCutoff":{"topRelevance":round4(top_relevance),"minRelevance":round4(min_relevance),"maxItemsBeforeBudget":max_items},"budgetReasoning":{"requestedBudget":budget,"spent":usage.spent,"saved":usage.saved,"budgetPressure":if budget==0{0.0}else{round4((usage.spent as f64)/(budget as f64))},"candidateCountBeforeFamilyCompaction":pre_compaction_candidate_count,"candidateCount":candidate_pool.len(),"candidateCountAfterFamilyCompaction":candidate_pool.len(),"familyCompactedCount":family_compacted_count,"returnedCount":final_with_factors.len(),"droppedCount":post_compaction_dropped_count,"totalPreBudgetDrops":family_compacted_count+post_compaction_dropped_count},"semanticRoute":semantic_route,"rerankRoute":rerank_route.clone()},"explain":{"returned":final_with_factors,"familyCompactions":family_compactions_json,"droppedCandidates":dropped_candidates,"shadowSemantic":shadow_semantic,"rerank":rerank_route}}),
    )
}
