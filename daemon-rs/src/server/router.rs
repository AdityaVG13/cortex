// SPDX-License-Identifier: MIT
use super::*;
use crate::budgets::BudgetEndpoint;
use crate::handlers;
use crate::handlers::mcp::handle_mcp_message_with_caller;
use crate::state::RuntimeState;
use axum::body::Bytes;
use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use serde_json::Value;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::CorsLayer;
pub fn build_router(state: RuntimeState, port: u16) -> Router {
    let allowed_origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        "http://127.0.0.1:1420".to_string(),
        "http://localhost:1420".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "http://localhost:3000".to_string(),
        "http://127.0.0.1:5173".to_string(),
        "http://localhost:5173".to_string(),
        "tauri://localhost".to_string(),
        "https://tauri.localhost".to_string(),
    ]
    .into_iter()
    .filter_map(|origin| parse_allowed_origin(&origin))
    .collect::<Vec<_>>();
    let cors = CorsLayer::new().allow_origin(allowed_origins).allow_methods(tower_http::cors::Any).allow_headers(tower_http::cors::Any);
    Router::new()
        .route("/health", get(handlers::health::handle_health))
        .route("/readiness", get(handlers::health::handle_readiness))
        .route("/digest", get(handlers::health::handle_digest))
        .route("/savings", get(handlers::health::handle_savings))
        .route("/stats", get(handlers::health::handle_stats))
        .route("/dump", get(handlers::health::handle_dump))
        .route("/store", post(handlers::store::handle_store))
        .route("/recall", get(handlers::recall::handle_recall).post(handlers::recall::handle_recall_post))
        .route("/recall/explain", get(handlers::recall::handle_recall_explain))
        .route("/recall/semantic", get(handlers::recall::handle_semantic_recall))
        .route("/peek", get(handlers::recall::handle_peek))
        .route("/unfold", get(handlers::recall::handle_unfold))
        .route("/boot", get(handlers::boot::handle_boot))
        .route("/boot/audit", get(handlers::boot::handle_boot_audit))
        .route("/diary", post(handlers::diary::handle_diary))
        .route("/recall/budget", get(handlers::recall::handle_budget_recall))
        .route("/feedback", post(handlers::feedback::handle_feedback))
        .route("/feedback/stats", get(handlers::feedback::handle_feedback_stats))
        .route("/agent-feedback", post(handlers::feedback::handle_agent_feedback_record))
        .route("/agent-feedback/stats", get(handlers::feedback::handle_agent_feedback_stats))
        .route("/crystals", get(handle_crystals))
        .route("/crystallize", post(handle_crystallize))
        .route("/compact", post(handle_compact))
        .route("/compact/benchmark", post(handle_compact_benchmark))
        .route("/storage", get(handle_storage))
        .route("/forget", post(handlers::mutate::handle_forget))
        .route("/resolve", post(handlers::mutate::handle_resolve))
        .route("/conflicts/resolve", post(handlers::mutate::handle_resolve))
        .route("/conflicts", get(handlers::mutate::handle_conflicts))
        .route("/permissions", get(handlers::mutate::handle_permissions_list))
        .route("/permissions/grant", post(handlers::mutate::handle_permissions_grant))
        .route("/permissions/revoke", post(handlers::mutate::handle_permissions_revoke))
        .route("/archive", post(handlers::mutate::handle_archive))
        .route("/focus/start", post(handle_focus_start))
        .route("/focus/end", post(handle_focus_end))
        .route("/shutdown", post(handlers::mutate::handle_shutdown))
        .route("/lock", post(handlers::conductor::handle_lock))
        .route("/unlock", post(handlers::conductor::handle_unlock))
        .route("/locks", get(handlers::conductor::handle_locks))
        .route("/activity", post(handlers::conductor::handle_post_activity).get(handlers::conductor::handle_get_activity))
        .route("/message", post(handlers::conductor::handle_post_message))
        .route("/messages", get(handlers::conductor::handle_get_messages))
        .route("/session/start", post(handlers::conductor::handle_session_start))
        .route("/session/heartbeat", post(handlers::conductor::handle_session_heartbeat))
        .route("/session/end", post(handlers::conductor::handle_session_end))
        .route("/sessions", get(handlers::conductor::handle_sessions))
        .route("/tasks", post(handlers::conductor::handle_create_task).get(handlers::conductor::handle_get_tasks))
        .route("/tasks/next", get(handlers::conductor::handle_next_task))
        .route("/tasks/claim", post(handlers::conductor::handle_claim_task))
        .route("/tasks/complete", post(handlers::conductor::handle_complete_task))
        .route("/tasks/abandon", post(handlers::conductor::handle_abandon_task))
        .route("/tasks/delete", post(handlers::conductor::handle_delete_task))
        .route("/feed", post(handlers::feed::handle_post_feed).get(handlers::feed::handle_get_feed))
        .route("/feed/ack", post(handlers::feed::handle_feed_ack))
        .route("/feed/{id}", get(handlers::feed::handle_get_feed_by_id))
        .route("/export", get(handlers::export::handle_export))
        .route("/import", post(handlers::export::handle_import))
        .route("/admin/user/add", post(handlers::admin::handle_user_add))
        .route("/admin/user/rotate-key", post(handlers::admin::handle_user_rotate_key))
        .route("/admin/user/remove", post(handlers::admin::handle_user_remove))
        .route("/admin/users", get(handlers::admin::handle_user_list))
        .route("/admin/team/create", post(handlers::admin::handle_team_create))
        .route("/admin/team/add-member", post(handlers::admin::handle_team_add_member))
        .route("/admin/team/remove-member", post(handlers::admin::handle_team_remove_member))
        .route("/admin/teams", get(handlers::admin::handle_team_list))
        .route("/admin/unowned", get(handlers::admin::handle_unowned))
        .route("/admin/assign-owner", post(handlers::admin::handle_assign_owner))
        .route("/admin/set-visibility", post(handlers::admin::handle_set_visibility))
        .route("/admin/archive", post(handlers::admin::handle_archive))
        .route("/admin/stats", get(handlers::admin::handle_stats))
        .route("/events/stream", get(handlers::events::handle_events_stream))
        .route("/brain/firing", get(handlers::events::handle_brain_firing_stream))
        .route("/mcp-rpc", post(handle_mcp_rpc))
        .layer(axum::middleware::from_fn_with_state(state.clone(), activity_tracking_middleware))
        .layer(CatchPanicLayer::custom(handle_handler_panic))
        .layer(cors)
        .with_state(state)
}
pub(crate) fn handle_handler_panic(err: Box<dyn std::any::Any + Send + 'static>) -> Response {
    let message = if let Some(s) = err.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    };
    eprintln!("[cortex] HTTP handler panic: {message}");
    handlers::json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        serde_json::json!({
            "error": "internal server error",
        }),
    )
}
pub(crate) async fn activity_tracking_middleware(State(state): State<RuntimeState>, mut request: Request, next: Next) -> Response {
    state.mark_activity_now();
    request.headers_mut().remove(handlers::CORTEX_PEER_IP_HEADER);
    let peer_ip = request.extensions().get::<ConnectInfo<std::net::SocketAddr>>().map(|ConnectInfo(addr)| addr.ip());
    if let Some(ip) = peer_ip {
        if let Ok(value) = HeaderValue::from_str(&ip.to_string()) {
            request.headers_mut().insert(handlers::CORTEX_PEER_IP_HEADER, value);
        }
    }
    next.run(request).await
}
pub(crate) async fn handle_mcp_rpc(State(state): State<RuntimeState>, headers: HeaderMap, body: Bytes) -> axum::response::Response {
    let caller_id = match handlers::ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(caller_id) => caller_id,
        Err(resp) => {
            let (message, hint) = match resp.status() {
                StatusCode::FORBIDDEN => ("Missing X-Cortex-Request header", Some("Include header X-Cortex-Request: true")),
                StatusCode::UNAUTHORIZED => ("Unauthorized", None),
                _ => ("Auth failed", None),
            };
            let status = resp.status();
            return handlers::json_response(
                status,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32600,
                        "message": message,
                        "hint": hint
                    },
                    "id": serde_json::Value::Null
                }),
            );
        }
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(msg) => msg,
        Err(_) => {
            return handlers::json_response(
                StatusCode::BAD_REQUEST,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32700,
                        "message": "Parse error"
                    },
                    "id": serde_json::Value::Null
                }),
            );
        }
    };
    let source = handlers::resolve_source_identity(&headers, "mcp");
    let ip = handlers::client_ip(&headers);
    if let Some(decision) = state.rate_limiter.check_budget_for_endpoint(ip, BudgetEndpoint::Mcp).await {
        if !decision.allowed {
            handlers::log_budget_rejection(&state, &decision, &source.agent, &ip).await;
            let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
            return handlers::json_response(
                StatusCode::OK,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32029,
                        "message": "budget_exceeded",
                        "data": decision.http_body_json()
                    },
                    "id": id
                }),
            );
        }
    }
    handlers::register_agent_presence_from_headers(&state, &headers, caller_id).await;
    match handle_mcp_message_with_caller(&state, &msg, caller_id, Some(&source)).await {
        Some(resp) => handlers::json_response(StatusCode::OK, resp),
        None => handlers::json_response(StatusCode::OK, serde_json::json!({})),
    }
}
