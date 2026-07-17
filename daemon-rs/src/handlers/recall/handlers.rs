// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::budgets::BudgetEndpoint;
use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, estimate_tokens, json_response, resolve_source_identity};
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;
use super::*;
pub(crate) const MAX_UNFOLD_SOURCES: usize = 50;
#[derive(Deserialize, Default)]
pub struct UnfoldQuery {
    pub sources: Option<String>,
}
async fn auth_recall_caller(headers: &HeaderMap, state: &RuntimeState) -> Result<Option<i64>, Response> {
    let caller_id = ensure_auth_with_caller_rated_for_class(headers, state, RequestClass::Recall).await?;
    if state.team_mode && caller_id.is_none() {
        return Err(json_response(StatusCode::FORBIDDEN, json!({ "error": "Team mode requires a caller-scoped ctx_ API key" })));
    }
    Ok(caller_id)
}
fn trim_source_prefix(source_prefix: Option<&str>) -> Option<&str> {
    source_prefix.map(str::trim).filter(|s| !s.is_empty())
}
fn attach_policy_modes(payload: &mut Value, resolved: RecallPolicyMode, requested: Option<RecallPolicyMode>) {
    if let Value::Object(map) = payload {
        map.insert("policyMode".to_string(), Value::String(resolved.as_str().to_string()));
        if let Some(mode) = requested {
            map.insert("requestedPolicyMode".to_string(), Value::String(mode.as_str().to_string()));
        }
    }
}
fn fire_recall_brain_event(state: &RuntimeState, payload: &Value, agent: &str) {
    let node_ids = extract_recall_node_ids(payload);
    let _ = state.brain_firing.send(crate::state::BrainFiringEvent {
        kind: crate::state::BrainKind::Recall,
        payload: json!({ "node_ids": node_ids, "agent": agent }),
        owner_id: state.default_owner_id,
    });
}
async fn run_unified_recall_handler(
    state: &RuntimeState,
    headers: &HeaderMap,
    q: String,
    requested_policy_mode: Option<RecallPolicyMode>,
    budget: Option<usize>,
    k: Option<usize>,
    source_prefix: Option<String>,
    agent_default: Option<&str>,
    missing_q_error: &str,
    failure_prefix: &str,
) -> Response {
    let caller_id = match auth_recall_caller(headers, state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let source_prefix = trim_source_prefix(source_prefix.as_deref());
    let agent = resolve_source_identity(headers, agent_default.unwrap_or("http")).agent;
    if q.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": missing_q_error }));
    }
    if let Err(resp) = ensure_endpoint_budget(headers, state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    let requested_budget = budget;
    let (mut budget, k, _) = resolve_recall_budget_k(requested_policy_mode, requested_budget, k);
    budget = maybe_apply_adaptive_default_budget(q.trim(), requested_policy_mode, requested_budget, budget, k);
    let resolved_policy_mode = recall_mode_for_budget(budget);
    let ctx = RecallContext::from_caller(caller_id, state);
    match execute_unified_recall(state, q.trim(), budget, k, &agent, &ctx, source_prefix).await {
        Ok(mut payload) => {
            attach_policy_modes(&mut payload, resolved_policy_mode, requested_policy_mode);
            fire_recall_brain_event(state, &payload, &agent);
            json_response(StatusCode::OK, payload)
        }
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("{failure_prefix}: {err}") })),
    }
}
pub async fn handle_recall(State(state): State<RuntimeState>, Query(query): Query<RecallQuery>, headers: HeaderMap) -> Response {
    let requested_policy_mode = match parse_recall_policy_mode(query.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => return json_response(StatusCode::BAD_REQUEST, json!({ "error": err })),
    };
    run_unified_recall_handler(&state, &headers, query.q.unwrap_or_default(), requested_policy_mode, query.budget, query.k, query.source_prefix, query.agent.as_deref(), "Missing query parameter: q", "Recall failed").await
}
pub async fn handle_recall_post(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<RecallQuery>) -> Response {
    let requested_policy_mode = match parse_recall_policy_mode(body.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => return json_response(StatusCode::BAD_REQUEST, json!({ "error": err })),
    };
    run_unified_recall_handler(&state, &headers, body.q.unwrap_or_default(), requested_policy_mode, body.budget, body.k, body.source_prefix, body.agent.as_deref(), "Missing recall payload field: q", "Recall failed").await
}
pub async fn handle_semantic_recall(State(state): State<RuntimeState>, Query(query): Query<RecallQuery>, headers: HeaderMap) -> Response {
    let caller_id = match auth_recall_caller(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    let k = query.k.unwrap_or(10);
    let budget = query.budget.unwrap_or(200);
    let source_prefix = trim_source_prefix(query.source_prefix.as_deref());
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if q.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing query parameter: q" }));
    }
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_semantic_recall(&state, q.trim(), budget, k, &agent, &ctx, source_prefix).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Semantic recall failed: {err}") })),
    }
}
pub async fn handle_budget_recall(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<RecallQuery>) -> Response {
    let caller_id = match auth_recall_caller(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = match query.q.as_deref() {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing query parameter: q" })),
    };
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    let budget = query.budget.unwrap_or(300);
    let k = query.k.unwrap_or(10);
    let source_prefix = trim_source_prefix(query.source_prefix.as_deref());
    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    let engine = state.embedding_engine.as_deref();
    match run_budget_recall_with_engine(&mut conn, &q, budget, k, engine, &ctx, source_prefix, Some(&state.degraded_mode)) {
        Ok(results) => {
            let usage = compute_recall_budget_usage(&results, budget);
            json_response(StatusCode::OK, json!({
                "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
                "budget": budget,
                "spent": usage.spent,
                "saved": usage.saved,
                "tokenUsageLine": format_recall_token_usage_line(budget, usage),
            }))
        }
        Err(e) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Budget recall failed: {e}") })),
    }
}
pub async fn handle_recall_explain(State(state): State<RuntimeState>, Query(query): Query<RecallQuery>, headers: HeaderMap) -> Response {
    let caller_id = match auth_recall_caller(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    if q.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing query parameter: q" }));
    }
    let requested_policy_mode = match parse_recall_policy_mode(query.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => return json_response(StatusCode::BAD_REQUEST, json!({ "error": err })),
    };
    let (mut budget, k, _) = resolve_recall_budget_k(requested_policy_mode, query.budget, query.k);
    let pool_k = query.pool_k.unwrap_or((k.max(8) * 3).min(64));
    let source_prefix = trim_source_prefix(query.source_prefix.as_deref());
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    budget = maybe_apply_adaptive_default_budget(q.trim(), requested_policy_mode, query.budget, budget, k);
    let resolved_policy_mode = recall_mode_for_budget(budget);
    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_recall_policy_explain(&state, q.trim(), budget, k, &agent, &ctx, source_prefix, pool_k, None).await {
        Ok(mut payload) => {
            attach_policy_modes(&mut payload, resolved_policy_mode, requested_policy_mode);
            json_response(StatusCode::OK, payload)
        }
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Recall explain failed: {err}") })),
    }
}
pub async fn handle_peek(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<RecallQuery>) -> Response {
    let caller_id = match auth_recall_caller(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let q = match &query.q {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => return json_response(StatusCode::BAD_REQUEST, json!({"error": "Missing query parameter: q"})),
    };
    let source_prefix = trim_source_prefix(query.source_prefix.as_deref());
    let k = query.k.unwrap_or(10);
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    match run_recall(&mut conn, &q, k, &ctx, source_prefix) {
        Ok(results) => {
            let matches: Vec<Value> = results.iter().map(|r| json!({ "source": r.source, "relevance": r.relevance, "method": r.method })).collect();
            let usage = compute_headlines_token_usage(&results);
            json_response(StatusCode::OK, json!({
                "count": matches.len(),
                "matches": matches,
                "tokenUsage": { "used": usage.spent, "saved": usage.saved },
                "tokenUsageLine": format!("Token usage: used {} tokens, saved {} vs full recall excerpts.", usage.spent, usage.saved)
            }))
        }
        Err(e) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    }
}
pub async fn handle_unfold(State(state): State<RuntimeState>, Query(query): Query<UnfoldQuery>, headers: HeaderMap) -> Response {
    let caller_id = match auth_recall_caller(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let ctx = RecallContext::from_caller(caller_id, &state);
    let sources_str = match &query.sources {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return json_response(StatusCode::BAD_REQUEST, json!({"error": "Missing query parameter: sources (comma-separated)"})),
    };
    let requested: Vec<&str> = sources_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if requested.is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "No valid sources provided"}));
    }
    if requested.len() > MAX_UNFOLD_SOURCES {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": format!("Too many sources (max {MAX_UNFOLD_SOURCES})")}));
    }
    let agent = resolve_source_identity(&headers, "http").agent;
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let mut results: Vec<Value> = Vec::new();
    let mut total_tokens = 0usize;
    for source in &requested {
        if let Some(mut item) = unfold_source(&conn, source, &ctx) {
            let tokens = estimate_tokens(item["text"].as_str().unwrap_or(""));
            total_tokens += tokens;
            if let Value::Object(ref mut map) = item {
                if !map.contains_key("source") { map.insert("source".to_string(), Value::String(source.to_string())); }
                map.insert("tokens".to_string(), Value::Number((tokens as u64).into()));
            }
            results.push(item);
        } else {
            results.push(json!({ "source": source, "text": null, "type": "not_found", "tokens": 0 }));
        }
    }
    json_response(StatusCode::OK, json!({
        "results": results,
        "totalTokens": total_tokens,
        "count": results.iter().filter(|r| r["type"] != "not_found").count(),
    }))
}
pub(crate) fn extract_recall_node_ids(payload: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>, limit: usize) {
        if out.len() >= limit { return; }
        match v {
            Value::Object(map) => {
                if let (Some(target_type), Some(target_id)) = (map.get("type").and_then(|t| t.as_str()), map.get("id").and_then(|t| t.as_i64())) {
                    if matches!(target_type, "memory" | "decision" | "crystal") {
                        out.push(format!("{target_type}-{target_id}"));
                        if out.len() >= limit { return; }
                    }
                }
                for (_, child) in map.iter() { walk(child, out, limit); }
            }
            Value::Array(arr) => { for child in arr.iter() { walk(child, out, limit); } }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(payload, &mut out, 16);
    out
}
