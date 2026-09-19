use super::{
    RecallContext, RecallItem, RecallWithVectorTrace, bump_retrievals_batch,
    bump_retrievals_sources, compute_headlines_token_usage, compute_recall_budget_usage,
    enforce_budget_token_invariant, format_recall_token_usage_line, is_benchmark_recall_scope,
    recall_latency_budget_ms_for_mode, recall_mode_for_budget, recall_to_json,
    run_clock_quorum_recall, take_last_route_trace,
};
use crate::db::checkpoint_wal_best_effort;
use crate::handlers::truncate_chars;
use crate::state::{RuntimeState, SqliteVecCanaryConfig};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

#[path = "engine_execution/budget.rs"]
mod budget;
pub use budget::*;
#[path = "engine_execution/explain.rs"]
mod explain;
pub use explain::*;

pub async fn emit_recall_query_event(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    agent: &str,
    source_prefix: Option<&str>,
    payload: Value,
) -> Result<(), String> {
    if is_benchmark_recall_scope(agent, source_prefix) {
        return Ok(());
    }
    // Recall telemetry never waits behind a background pass: bounded lock
    // wait, then defer to the next writer.
    let deferred = crate::state::DeferredSideEffect::Event {
        event_type: "recall_query".into(),
        payload: payload.clone(),
        agent: agent.to_string(),
    };
    let agent = agent.to_string();
    state
        .with_write_or_defer(
            cx,
            move |conn| {
                if crate::handlers::log_event(conn, "recall_query", payload, &agent).is_ok() {
                    checkpoint_wal_best_effort(conn);
                }
            },
            deferred,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub async fn bump_retrievals_bounded(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    results: &[RecallItem],
) -> Result<(), String> {
    let sources: Vec<String> = results.iter().map(|r| r.source.clone()).collect();
    if sources.is_empty() {
        return Ok(());
    }
    let deferred = crate::state::DeferredSideEffect::Retrievals {
        sources: sources.clone(),
    };
    state
        .with_write_or_defer(
            cx,
            move |conn| bump_retrievals_sources(conn, &sources),
            deferred,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
pub fn build_method_breakdown(results: &[RecallItem]) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for item in results {
        *counts.entry(item.method.clone()).or_insert(0) += 1;
    }
    json!(counts)
}
pub fn method_count(methods: &Value, method: &str) -> i64 {
    methods.get(method).and_then(|v| v.as_i64()).unwrap_or(0)
}
pub fn classify_recall_tier(cached: bool, mode: &str, methods: &Value) -> &'static str {
    if cached {
        return "cache_hit";
    }
    match mode {
        "headlines" => return "headlines",
        "semantic" => return "semantic_only",
        _ => {}
    }
    let keyword = method_count(methods, "keyword");
    let semantic = method_count(methods, "semantic");
    let hybrid = method_count(methods, "hybrid");
    let crystal = method_count(methods, "crystal");
    let associative = method_count(methods, "associative");
    let labeled = |plain, crystal_label| if crystal > 0 { crystal_label } else { plain };
    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        return labeled("hybrid_fusion", "hybrid_crystal");
    }
    if associative > 0 && (keyword > 0 || semantic > 0 || crystal > 0) {
        return "associative_blend";
    }
    if keyword > 0 {
        return labeled("keyword_only", "keyword_crystal");
    }
    if semantic > 0 {
        return labeled("semantic_only", "semantic_crystal");
    }
    if crystal > 0 {
        return "crystal_only";
    }
    if associative > 0 {
        return "associative_only";
    }
    "unknown"
}
pub async fn execute_unified_recall(
    cx: &asupersync::Cx,
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
    let (results, semantic_route, fail_closed, route_trace) = {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        let (mut results, mut semantic_route) = if budget == 0 {
            let trace = run_recall_with_query_vector_trace(
                &conn,
                query_text,
                k,
                None,
                ctx,
                source_prefix,
                Some(&state.sqlite_vec_canary),
                false,
            )?;
            (trace.ranked, trace.semantic_route)
        } else {
            let trace = run_budget_recall_trace_with_query_vector(
                &conn,
                query_text,
                budget,
                k,
                None,
                ctx,
                source_prefix,
                Some(&state.sqlite_vec_canary),
                false,
            )?;
            (trace.budgeted, trace.semantic_route)
        };
        let mut fail_closed = Value::Null;
        if budget > 0 {
            let elapsed_before_fallback = started_at.elapsed().as_millis();
            if elapsed_before_fallback >= latency_budget_ms {
                let fallback_trace = run_budget_recall_trace_with_query_vector(
                    &conn,
                    query_text,
                    budget,
                    k,
                    None,
                    ctx,
                    source_prefix,
                    Some(&state.sqlite_vec_canary),
                    false,
                )?;
                results = fallback_trace.budgeted;
                semantic_route = json!({"engine":"clock-quorum","modelFree":true,"reason":"latency_budget_fail_closed","elapsedMsBeforeFallback":elapsed_before_fallback,"latencyBudgetMs":latency_budget_ms});
                fail_closed = json!({"triggered":true,"elapsedMsBeforeFallback":elapsed_before_fallback,"latencyBudgetMs":latency_budget_ms,"fallback":"clock_quorum"});
            }
        }
        let route_trace = take_last_route_trace()
            .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        (results, semantic_route, fail_closed, route_trace)
    };
    let shadow_semantic = json!({"status": "skipped", "reason": "model_free"});
    let rerank_route = json!({"status":"skipped","reason":"model_free","mode":"off"});
    bump_retrievals_bounded(cx, state, &results).await?;
    // CQR must be byte-stable against an unchanged database. Served-content
    // filtering would drop the same excerpt on a repeat query.
    let results = if budget == 0 {
        results
    } else {
        enforce_budget_token_invariant(results, budget, query_text)
    };
    let usage = if budget == 0 {
        compute_headlines_token_usage(&results)
    } else {
        compute_recall_budget_usage(&results, budget)
    };
    let mode = policy_mode.as_str();
    let method_breakdown = build_method_breakdown(&results);
    let tier = classify_recall_tier(false, mode, &method_breakdown);
    let latency_ms = started_at.elapsed().as_millis() as i64;
    let mut event = json!({"agent":agent,"query":truncate_chars(query_text,120),"budget":budget,"spent":usage.spent,"saved":usage.saved,"hits":results.len(),"mode":mode,"cached":false,"method_breakdown":method_breakdown,"tier":tier,"latency_ms":latency_ms,"latency_budget_ms":latency_budget_ms,"semantic_route":semantic_route.clone(),"shadow_semantic":shadow_semantic,"fail_closed":fail_closed,"rerank":rerank_route.clone()});
    if budget > 0 {
        event["over_budget"] = json!(usage.over_budget);
    }
    emit_recall_query_event(cx, state, agent, source_prefix, event).await?;
    let mut out = json!({"budget":budget,"spent":usage.spent,"saved":usage.saved,"overBudget":usage.over_budget,"tokenUsageLine":format_recall_token_usage_line(budget, usage),"mode":mode,"policyMode":mode,"tier":tier,"latencyMs":latency_ms,"latencyBudgetMs":latency_budget_ms,"failClosed":fail_closed,"semanticRoute":semantic_route,"rerankRoute":rerank_route});
    if budget == 0 {
        let headlines = results.iter().map(|item| json!({"source": item.source, "relevance": item.relevance, "method": item.method})).collect::<Vec<_>>();
        out["count"] = json!(headlines.len());
        out["results"] = json!(headlines);
    } else {
        out["results"] = json!(results.into_iter().map(recall_to_json).collect::<Vec<_>>());
        out["routes"] = route_trace;
    }
    Ok(out)
}
pub async fn execute_semantic_recall(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    query_text: &str,
    budget: usize,
    k: usize,
    agent: &str,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
) -> Result<Value, String> {
    let started_at = Instant::now();
    let semantic_available = true;
    let (budgeted, semantic_route) = {
        let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
        let results = run_clock_quorum_recall(&conn, query_text, budget, k, ctx, source_prefix)?;
        (results, json!({"engine":"clock-quorum","modelFree":true}))
    };
    {
        let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
        bump_retrievals_batch(&conn, &budgeted);
    }
    let budgeted = enforce_budget_token_invariant(budgeted, budget, query_text);
    let usage = compute_recall_budget_usage(&budgeted, budget);
    let mode = "semantic";
    let method_breakdown = build_method_breakdown(&budgeted);
    let tier = classify_recall_tier(false, mode, &method_breakdown);
    let latency_ms = started_at.elapsed().as_millis() as i64;
    emit_recall_query_event(cx,state,agent,source_prefix,json!({"agent":agent,"query":truncate_chars(query_text,120),"mode":mode,"k":k,"budget":budget,"spent":usage.spent,"saved":usage.saved,"over_budget":usage.over_budget,"hits":budgeted.len(),"results":budgeted.len(),"semantic_available":semantic_available,"cached":false,"method_breakdown":method_breakdown,"tier":tier,"latency_ms":latency_ms,"semantic_route":semantic_route.clone(),}),).await?;
    Ok(
        json!({"results":budgeted.into_iter().map(recall_to_json).collect::<Vec<_>>(),"mode":"semantic","budget":budget,"spent":usage.spent,"saved":usage.saved,"overBudget":usage.over_budget,"tokenUsageLine":format_recall_token_usage_line(budget,usage),"semanticAvailable":semantic_available,"semanticRoute":semantic_route,"tier":tier,"latencyMs":latency_ms,}),
    )
}
#[allow(clippy::type_complexity)]
pub fn run_recall_with_query_vector_trace(
    conn: &Connection,
    query_text: &str,
    k: usize,
    query_vector: Option<&[f32]>,
    ctx: &RecallContext,
    source_prefix: Option<&str>,
    canary: Option<&SqliteVecCanaryConfig>,
    sqlite_vec_shadow_enabled: bool,
) -> Result<RecallWithVectorTrace, String> {
    let ranked = run_clock_quorum_recall(conn, query_text, 0, k, ctx, source_prefix)?;
    let _ = (query_vector, canary, sqlite_vec_shadow_enabled);
    Ok(RecallWithVectorTrace {
        ranked,
        semantic_route: json!({"engine":"clock-quorum","modelFree":true}),
    })
}
