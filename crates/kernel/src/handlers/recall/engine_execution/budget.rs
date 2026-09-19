use super::super::{
    MIN_EXCERPT_CHARS, RecallContext, RecallFamilyCompaction, RecallItem, budget_rank_char_cap,
    bump_retrievals_batch, compact_budget_family_candidates,
    compact_budget_family_candidates_with_trace, excerpt_signature_terms,
    fit_excerpt_to_remaining_budget, query_focus_terms_for_excerpt, run_clock_quorum_recall,
    semantic_budget_max_items, semantic_budget_min_relevance, should_early_stop_budget_selection,
    should_skip_redundant_budget_candidate, update_query_term_coverage,
};
use super::run_recall_with_query_vector_trace;
use crate::handlers::estimate_tokens;
use crate::state::SqliteVecCanaryConfig;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::HashSet;

fn select_budgeted_items(
    raw: &[RecallItem],
    token_budget: usize,
    query_text: &str,
    min_relevance: f64,
    max_items: usize,
    empty_take: usize,
) -> Vec<RecallItem> {
    let mut candidates: Vec<RecallItem> = raw
        .iter()
        .filter(|item| item.relevance >= min_relevance)
        .take(max_items)
        .cloned()
        .collect();
    if candidates.is_empty() {
        candidates = raw.iter().take(empty_take).cloned().collect();
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

pub fn apply_semantic_budget(
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
    select_budgeted_items(
        &raw,
        token_budget,
        query_text,
        min_relevance,
        max_items,
        max_items.max(1),
    )
}

pub struct RecallBudgetTrace {
    pub budgeted: Vec<RecallItem>,
    pub candidate_pool: Vec<RecallItem>,
    pub pre_compaction_candidate_count: usize,
    pub family_compactions: Vec<RecallFamilyCompaction>,
    pub retrieval_depth: usize,
    pub top_relevance: f64,
    pub min_relevance: f64,
    pub max_items: usize,
    pub semantic_route: Value,
}

fn semantic_retrieval_depth(token_budget: usize, k: usize) -> usize {
    match token_budget {
        0..=220 => k.max(10).saturating_mul(3).min(30),
        221..=400 => k.max(10).saturating_mul(2).min(28),
        _ => k.max(12),
    }
}

pub fn run_budget_recall(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    run_budget_recall_with_engine(conn, query_text, token_budget, k, ctx, source_prefix, None)
}

#[allow(dead_code)]
pub fn run_semantic_recall_with_query_vector(
    conn: &Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    _canary: Option<&SqliteVecCanaryConfig>,
    _sqlite_vec_shadow_enabled: bool,
) -> (Vec<RecallItem>, Value) {
    let _ = query_vector;
    match run_clock_quorum_recall(conn, query_text, 0, k, ctx, source_prefix) {
        Ok(ranked) => (ranked, json!({"engine":"clock-quorum","modelFree":true})),
        Err(_) => (
            Vec::new(),
            json!({"engine":"clock-quorum","modelFree":true,"error":true}),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_budget_recall_trace_with_query_vector(
    conn: &Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>,
    sqlite_vec_shadow_enabled: bool,
) -> Result<RecallBudgetTrace, String> {
    let retrieval_depth = semantic_retrieval_depth(token_budget, k);
    let recall_trace = run_recall_with_query_vector_trace(
        conn,
        query_text,
        retrieval_depth,
        query_vector,
        ctx,
        source_prefix,
        canary,
        sqlite_vec_shadow_enabled,
    )?;
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
    let (raw, _family_compaction_dropped, family_compactions) =
        compact_budget_family_candidates_with_trace(pre_compaction_pool, query_text, token_budget);
    let top_relevance = raw.first().map(|item| item.relevance).unwrap_or(0.0);
    let min_relevance = semantic_budget_min_relevance(top_relevance, query_text);
    let max_items = semantic_budget_max_items(token_budget, query_text, k.max(1));
    let budgeted = select_budgeted_items(
        &raw,
        token_budget,
        query_text,
        min_relevance,
        max_items,
        max_items,
    );
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

pub fn run_budget_recall_with_engine(
    conn: &mut Connection,
    query_text: &str,
    token_budget: usize,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    _degraded_flag: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Vec<RecallItem>, String> {
    let trace = run_budget_recall_trace_with_query_vector(
        conn,
        query_text,
        token_budget,
        k,
        None,
        ctx,
        source_prefix,
        None,
        false,
    )?;
    bump_retrievals_batch(conn, &trace.budgeted);
    Ok(trace.budgeted)
}

pub fn run_recall(
    conn: &mut Connection,
    query_text: &str,
    k: usize,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Vec<RecallItem>, String> {
    let trace = run_recall_with_query_vector_trace(
        conn,
        query_text,
        k,
        None,
        ctx,
        source_prefix,
        None,
        false,
    )?;
    bump_retrievals_batch(conn, &trace.ranked);
    Ok(trace.ranked)
}
