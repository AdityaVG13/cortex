use super::*;
use crate::api_types::StoreRequest;
use crate::budgets::BudgetEndpoint;
use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, json_response, require_team_caller, resolve_source_identity};
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde_json::json;
pub async fn handle_store(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<StoreRequest>) -> Response {
    let caller_id = match ensure_auth_with_caller_rated_for_class(&headers, &state, RequestClass::Store).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if let Err(resp) = require_team_caller(&state, caller_id) {
        return resp;
    }
    let extra_anchors = store_request_anchors(&body);
    let decision = body.decision.unwrap_or_default();
    if decision.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":"Missing field: decision"}));
    }
    let source_identity = resolve_source_identity(&headers, body.source_agent.as_deref().unwrap_or("http"));
    let source_agent = source_identity.agent.clone();
    if let Err(resp) = ensure_endpoint_budget(&headers, &state, BudgetEndpoint::Store, &source_agent).await {
        return resp;
    }
    let benchmark_store = body.entry_type.as_deref().map(is_benchmark_entry_type).unwrap_or(false) || is_benchmark_source_agent(&source_agent);
    let provenance =
        DecisionProvenance::from_fields(&source_agent, body.source_model.as_deref().or(source_identity.model.as_deref()), body.reasoning_depth.as_deref());
    if let Err(StoreError::BadRequest(message)) = validate_explicit_ttl_seconds(body.ttl_seconds) {
        return json_response(StatusCode::BAD_REQUEST, json!({"error":message}));
    }
    let decision_text = crate::handlers::redact_secrets(&decision.trim().to_string());
    let redacted_context = body.context.map(|c| crate::handlers::redact_secrets(&c));
    let mut conn = state.db.lock().await;
    let result = store_decision_with_input_embedding_and_provenance_retention(
        &mut conn,
        &decision_text,
        redacted_context.clone(),
        body.entry_type,
        source_agent.clone(),
        provenance,
        body.confidence,
        body.ttl_seconds,
        body.retention_class,
        None,
        caller_id,
    );
    match result {
        Ok((mut entry, new_id)) => {
            if !benchmark_store {
                crate::focus::focus_append(&conn, &source_agent, &decision_text);
            }
            let target_id = new_id.or_else(|| entry.get("id").and_then(|v| v.as_i64()));
            let action = entry.get("action").and_then(|v| v.as_str()).unwrap_or("stored").to_string();
            if let Some(version_id) = crate::traces::record_store_write(&conn, &source_agent, &decision_text, &action, "decision", target_id, caller_id) {
                entry["versionId"] = json!(version_id);
            }
            crate::graph::ingest_for_target(&conn, &decision_text, "decision", target_id, None, caller_id);
            if let Some(id) = target_id {
                let origin =
                    if extra_anchors.is_empty() { crate::clockwork::ClockOrigin::DeterministicExtract } else { crate::clockwork::ClockOrigin::Explicit };
                if let Err(err) = crate::clockwork::project_target(&conn, &decision_text, &extra_anchors, "decision", id, origin, None) {
                    eprintln!("[store] Warning: failed to project clock anchors for {id}: {err}");
                }
            }
            json_response(
                StatusCode::OK,
                json!({"stored":true,
"entry":entry}),
            )
        }
        Err(StoreError::BadRequest(message)) => json_response(StatusCode::BAD_REQUEST, json!({"error":message})),
        Err(StoreError::Validation { message, quality, factors }) => json_response(
            StatusCode::BAD_REQUEST,
            json!({"error":message,"quality":quality,
"factors":factors.as_json(),}),
        ),
        Err(StoreError::Internal(err)) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error":
format!("Store failed: {err}")}),
        ),
    }
}

fn store_request_anchors(body: &StoreRequest) -> Vec<crate::clockwork::QueryAnchor> {
    use crate::clockwork::{normalize_anchor_value, AnchorKind, QueryAnchor, MAX_ANCHORS_PER_TRACE};
    let mut extra = Vec::new();
    for path in body.paths.iter().flatten() {
        extra.push(QueryAnchor { kind: AnchorKind::Path, value: normalize_anchor_value(AnchorKind::Path, path), specificity: 3 });
    }
    for symbol in body.symbols.iter().flatten() {
        extra.push(QueryAnchor { kind: AnchorKind::Symbol, value: normalize_anchor_value(AnchorKind::Symbol, symbol), specificity: 3 });
    }
    if let Some(goal) = body.goal_id {
        extra.push(QueryAnchor { kind: AnchorKind::Goal, value: goal.to_string(), specificity: 3 });
    }
    for raw in body.anchors.iter().flatten() {
        extra.push(QueryAnchor { kind: AnchorKind::Term, value: normalize_anchor_value(AnchorKind::Term, raw), specificity: 2 });
    }
    extra.truncate(MAX_ANCHORS_PER_TRACE);
    extra
}
