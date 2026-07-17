use super::agent::{
    build_agent_feedback_stats_payload, normalize_horizon_days, normalize_limit,
    record_agent_feedback_from_value, AgentFeedbackRecordRequest, AgentFeedbackStatsQuery,
};
use crate::handlers::{ensure_auth_with_caller_rated, json_error, json_response};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde_json::json;
pub async fn handle_agent_feedback_record(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<AgentFeedbackRecordRequest>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"error":"Team mode requires a caller-scoped ctx_ API key"}),
        );
    }
    let owner_id = if state.team_mode {
        caller_id.unwrap_or_default()
    } else {
        0
    };
    let args = json!({"agent":body.agent,"task_class":body.task_class,"outcome":body.outcome,"outcome_score"
:body.outcome_score,"quality_score":body.quality_score,"latency_ms":body.latency_ms,"retries":body.retries,"tokens_used":body.
tokens_used,"memory_sources":body.memory_sources.unwrap_or_default(),"notes":body.notes,});
    let conn = state.db.lock().await;
    match record_agent_feedback_from_value(&conn, owner_id, &args, "http") {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_error(StatusCode::BAD_REQUEST, &err),
    }
}
pub async fn handle_agent_feedback_stats(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<AgentFeedbackStatsQuery>,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({"error":"Team mode requires a caller-scoped ctx_ API key"}),
        );
    }
    let owner_id = if state.team_mode {
        caller_id.unwrap_or_default()
    } else {
        0
    };
    let horizon_days = normalize_horizon_days(query.horizon_days);
    let limit = normalize_limit(query.limit);
    let conn = state.db.lock().await;
    match build_agent_feedback_stats_payload(
        &conn,
        owner_id,
        horizon_days,
        limit,
        query.task_class.as_deref(),
        query.agent.as_deref(),
    ) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_error(StatusCode::BAD_REQUEST, &err),
    }
}
