
mod types;
pub(crate) use types::*;

use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

pub(crate) fn bounded_ttl_seconds(raw: Option<i64>, default_seconds: i64) -> i64 {
    raw.unwrap_or(default_seconds).clamp(1, MAX_REQUEST_TTL_SECONDS)
}

fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}

fn bad_request(error: &'static str) -> Response {
    json_response(StatusCode::BAD_REQUEST, json!({"error":error}))
}

async fn auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    ensure_auth_rated(headers, state).await.map(|_| ())
}

fn expires_in(ttl: i64) -> String {
    (Utc::now() + Duration::seconds(ttl)).to_rfc3339()
}

fn ok(body: Value) -> Response {
    json_response(StatusCode::OK, body)
}

pub async fn handle_lock(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<LockRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.path).is_none() || trimmed_non_empty(body.agent).is_none() {
        return bad_request("Missing required fields: path, agent");
    }
    ok(json!({"locked":true,"lockId":Uuid::new_v4().to_string(),"expiresAt":expires_in(bounded_ttl_seconds(body.ttl,300))}))
}

pub async fn handle_unlock(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<LockRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.path).is_none() || trimmed_non_empty(body.agent).is_none() {
        return bad_request("Missing required fields: path, agent");
    }
    ok(json!({"unlocked":true}))
}

pub async fn handle_locks(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"locks":[]}))
}

pub async fn handle_post_activity(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<ActivityRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.agent).is_none() || trimmed_non_empty(body.description).is_none() {
        return bad_request("Missing required fields: agent, description");
    }
    ok(json!({"recorded":true,"activityId":Uuid::new_v4().to_string()}))
}

pub async fn handle_get_activity(State(state): State<RuntimeState>, headers: HeaderMap, Query(_query): Query<SinceQuery>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"activities":[]}))
}

pub async fn handle_post_message(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<MessageRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.from).is_none() || trimmed_non_empty(body.to).is_none() || trimmed_non_empty(body.message).is_none() {
        return bad_request("Missing required fields: from, to, message");
    }
    ok(json!({"sent":true,"messageId":Uuid::new_v4().to_string()}))
}

pub async fn handle_get_messages(State(state): State<RuntimeState>, headers: HeaderMap, Query(_query): Query<MessagesQuery>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"messages":[]}))
}

pub async fn handle_session_start(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionStartRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.agent).is_none() {
        return bad_request("Missing required field: agent");
    }
    ok(json!({"sessionId":Uuid::new_v4().to_string(),"heartbeatInterval":60,"freshened":false}))
}

pub async fn handle_session_heartbeat(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionHeartbeatRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.agent).is_none() {
        return bad_request("Missing or invalid required field: agent");
    }
    ok(json!({"renewed":true,"expiresAt":expires_in(SESSION_TTL_SECONDS)}))
}

pub async fn handle_session_end(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<SessionEndRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.agent).is_none() {
        return bad_request("Missing required field: agent");
    }
    ok(json!({"ended":true}))
}

pub async fn handle_sessions(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"sessions":[]}))
}

pub async fn handle_create_task(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TaskCreateRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    if trimmed_non_empty(body.title).is_none() {
        return bad_request("Missing required field: title");
    }
    json_response(StatusCode::CREATED, json!({"taskId":Uuid::new_v4().to_string(),"status":"pending"}))
}

pub async fn handle_get_tasks(State(state): State<RuntimeState>, headers: HeaderMap, Query(_query): Query<TaskQuery>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"tasks":[]}))
}

pub async fn handle_next_task(State(state): State<RuntimeState>, headers: HeaderMap, Query(_query): Query<NextTaskQuery>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    ok(json!({"task":null}))
}

pub async fn handle_claim_task(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TaskClaimRequest>) -> Response {
    task_ack(state, headers, body.task_id, body.agent, "claimed").await
}

pub async fn handle_complete_task(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TaskCompleteRequest>) -> Response {
    task_ack(state, headers, body.task_id, body.agent, "completed").await
}

pub async fn handle_abandon_task(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TaskAbandonRequest>) -> Response {
    task_ack(state, headers, body.task_id, body.agent, "abandoned").await
}

async fn task_ack(state: RuntimeState, headers: HeaderMap, task_id: Option<String>, agent: Option<String>, field: &'static str) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let Some(task_id) = trimmed_non_empty(task_id) else {
        return bad_request("Missing required fields: taskId, agent");
    };
    if trimmed_non_empty(agent).is_none() {
        return bad_request("Missing required fields: taskId, agent");
    }
    let mut payload = json!({"taskId":task_id});
    if let Value::Object(map) = &mut payload {
        map.insert(field.to_string(), Value::Bool(true));
    }
    ok(payload)
}

pub async fn handle_delete_task(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<TaskDeleteRequest>) -> Response {
    if let Err(resp) = auth(&headers, &state).await {
        return resp;
    }
    let Some(task_id) = trimmed_non_empty(body.task_id) else {
        return bad_request("Missing required field: taskId");
    };
    ok(json!({"deleted":true,"taskId":task_id}))
}
