// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget};
use crate::handlers::{
    estimate_tokens, json_response, now_iso, parse_timestamp_ms, resolve_source_identity,
    truncate_chars,
};

use super::*;
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

// ─── GET /recall ─────────────────────────────────────────────────────────────

pub async fn handle_recall(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    let requested_policy_mode = match parse_recall_policy_mode(query.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": err }));
        }
    };
    let (mut budget, k, _resolved_policy_mode) =
        resolve_recall_budget_k(requested_policy_mode, query.budget, query.k);
    let source_prefix = query
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;

    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing query parameter: q" }),
        );
    }
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }
    budget = maybe_apply_adaptive_default_budget(
        q.trim(),
        requested_policy_mode,
        query.budget,
        budget,
        k,
    );
    let resolved_policy_mode = recall_mode_for_budget(budget);

    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_unified_recall(&state, q.trim(), budget, k, &agent, &ctx, source_prefix).await {
        Ok(mut payload) => {
            if let Value::Object(map) = &mut payload {
                map.insert(
                    "policyMode".to_string(),
                    Value::String(resolved_policy_mode.as_str().to_string()),
                );
                if let Some(mode) = requested_policy_mode {
                    map.insert(
                        "requestedPolicyMode".to_string(),
                        Value::String(mode.as_str().to_string()),
                    );
                }
            }
            let node_ids = extract_recall_node_ids(&payload);
            let _ = state.brain_firing.send(crate::state::BrainFiringEvent {
                kind: crate::state::BrainKind::Recall,
                payload: json!({ "node_ids": node_ids, "agent": agent }),
                owner_id: state.default_owner_id,
            });
            json_response(StatusCode::OK, payload)
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Recall failed: {err}") }),
        ),
    }
}

/// Extract a flat list of node ID strings from a recall payload for Brain
/// telemetry. Returns up to 16 IDs to keep the SSE event small. Looks at
/// "memories", "decisions", "crystals", and "results" arrays under any nesting.
pub(crate) fn extract_recall_node_ids(payload: &Value) -> Vec<String> {
    fn walk(v: &Value, out: &mut Vec<String>, limit: usize) {
        if out.len() >= limit {
            return;
        }
        match v {
            Value::Object(map) => {
                if let (Some(target_type), Some(target_id)) = (
                    map.get("type").and_then(|t| t.as_str()),
                    map.get("id").and_then(|t| t.as_i64()),
                ) {
                    if matches!(target_type, "memory" | "decision" | "crystal") {
                        out.push(format!("{target_type}-{target_id}"));
                        if out.len() >= limit {
                            return;
                        }
                    }
                }
                for (_, child) in map.iter() {
                    walk(child, out, limit);
                }
            }
            Value::Array(arr) => {
                for child in arr.iter() {
                    walk(child, out, limit);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(payload, &mut out, 16);
    out
}

// ─── POST /recall ────────────────────────────────────────────────────────────

pub async fn handle_recall_post(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<RecallBody>,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = body.q.unwrap_or_default();
    let requested_policy_mode = match parse_recall_policy_mode(body.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": err }));
        }
    };
    let (mut budget, k, _resolved_policy_mode) =
        resolve_recall_budget_k(requested_policy_mode, body.budget, body.k);
    let source_prefix = body
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent = resolve_source_identity(&headers, body.agent.as_deref().unwrap_or("http")).agent;

    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing recall payload field: q" }),
        );
    }
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }
    budget = maybe_apply_adaptive_default_budget(
        q.trim(),
        requested_policy_mode,
        body.budget,
        budget,
        k,
    );
    let resolved_policy_mode = recall_mode_for_budget(budget);

    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_unified_recall(&state, q.trim(), budget, k, &agent, &ctx, source_prefix).await {
        Ok(mut payload) => {
            if let Value::Object(map) = &mut payload {
                map.insert(
                    "policyMode".to_string(),
                    Value::String(resolved_policy_mode.as_str().to_string()),
                );
                if let Some(mode) = requested_policy_mode {
                    map.insert(
                        "requestedPolicyMode".to_string(),
                        Value::String(mode.as_str().to_string()),
                    );
                }
            }
            let node_ids = extract_recall_node_ids(&payload);
            let _ = state.brain_firing.send(crate::state::BrainFiringEvent {
                kind: crate::state::BrainKind::Recall,
                payload: json!({ "node_ids": node_ids, "agent": agent }),
                owner_id: state.default_owner_id,
            });
            json_response(StatusCode::OK, payload)
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Recall failed: {err}") }),
        ),
    }
}

pub async fn handle_semantic_recall(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    let k = query.k.unwrap_or(10);
    let budget = query.budget.unwrap_or(200);
    let source_prefix = query
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;

    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing query parameter: q" }),
        );
    }
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }

    let ctx = RecallContext::from_caller(caller_id, &state);
    match execute_semantic_recall(&state, q.trim(), budget, k, &agent, &ctx, source_prefix).await {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Semantic recall failed: {err}") }),
        ),
    }
}

// ─── GET /recall/budget ──────────────────────────────────────────────────────

pub async fn handle_budget_recall(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<RecallQuery>,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = match query.q.as_deref() {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing query parameter: q" }),
            );
        }
    };

    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }
    let budget = query.budget.unwrap_or(300);
    let k = query.k.unwrap_or(10);
    let source_prefix = query
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    let engine = state.embedding_engine.as_deref();
    match run_budget_recall_with_engine(
        &mut conn,
        &q,
        budget,
        k,
        engine,
        &ctx,
        source_prefix,
        Some(&state.degraded_mode),
    ) {
        Ok(results) => {
            let usage = compute_recall_budget_usage(&results, budget);
            json_response(
                StatusCode::OK,
                json!({
                    "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
                    "budget": budget,
                    "spent": usage.spent,
                    "saved": usage.saved,
                    "tokenUsageLine": format_recall_token_usage_line(budget, usage),
                }),
            )
        }
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Budget recall failed: {e}") }),
        ),
    }
}

pub async fn handle_recall_explain(
    State(state): State<RuntimeState>,
    Query(query): Query<RecallQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = query.q.unwrap_or_default();
    if q.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing query parameter: q" }),
        );
    }

    let requested_policy_mode = match parse_recall_policy_mode(query.policy_mode.as_deref()) {
        Ok(mode) => mode,
        Err(err) => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": err }));
        }
    };
    let (mut budget, k, _resolved_policy_mode) =
        resolve_recall_budget_k(requested_policy_mode, query.budget, query.k);
    let pool_k = query.pool_k.unwrap_or((k.max(8) * 3).min(64));
    let source_prefix = query
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }
    let ctx = RecallContext::from_caller(caller_id, &state);
    budget = maybe_apply_adaptive_default_budget(
        q.trim(),
        requested_policy_mode,
        query.budget,
        budget,
        k,
    );
    let resolved_policy_mode = recall_mode_for_budget(budget);

    match execute_recall_policy_explain(
        &state,
        q.trim(),
        budget,
        k,
        &agent,
        &ctx,
        source_prefix,
        pool_k,
    )
    .await
    {
        Ok(mut payload) => {
            if let Value::Object(map) = &mut payload {
                map.insert(
                    "policyMode".to_string(),
                    Value::String(resolved_policy_mode.as_str().to_string()),
                );
                if let Some(mode) = requested_policy_mode {
                    map.insert(
                        "requestedPolicyMode".to_string(),
                        Value::String(mode.as_str().to_string()),
                    );
                }
            }
            json_response(StatusCode::OK, payload)
        }
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Recall explain failed: {err}") }),
        ),
    }
}

// ─── GET /peek ───────────────────────────────────────────────────────────────

pub async fn handle_peek(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<RecallQuery>,
) -> Response {
    let caller_id =
        match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Recall).await
        {
            Ok(id) => id,
            Err(resp) => return resp,
        };
    let caller_id = match require_team_caller(&state, caller_id) {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    let q = match &query.q {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "Missing query parameter: q"}),
            );
        }
    };
    let source_prefix = query
        .source_prefix
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let k = query.k.unwrap_or(10);
    let agent = resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("http")).agent;
    if let Err(resp) =
        ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Recall, &agent).await
    {
        return resp;
    }
    let ctx = RecallContext::from_caller(caller_id, &state);
    let mut conn = state.db.lock().await;
    match run_recall(&mut conn, &q, k, &ctx, source_prefix) {
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
            let usage = compute_headlines_token_usage(&results);
            json_response(
                StatusCode::OK,
                json!({
                    "count": matches.len(),
                    "matches": matches,
                    "tokenUsage": {
                        "used": usage.spent,
                        "saved": usage.saved
                    },
                    "tokenUsageLine": format!(
                        "Token usage: used {} tokens, saved {} vs full recall excerpts.",
                        usage.spent, usage.saved
                    )
                }),
            )
        }
        Err(e) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
    }
}

