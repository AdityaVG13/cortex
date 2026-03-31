use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde_json::Value;
use tower_http::cors::CorsLayer;

use crate::handlers;
use crate::handlers::mcp::handle_mcp_message;
use crate::handlers::ensure_auth;
use crate::state::RuntimeState;

pub fn build_router(state: RuntimeState) -> Router {
    // SEC-001 fix: restrict CORS to localhost origins only.
    // Blocks drive-by attacks from arbitrary websites.
    let cors = CorsLayer::new()
        .allow_origin([
            "http://127.0.0.1:7437".parse::<HeaderValue>().unwrap(),
            "http://localhost:7437".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:3000".parse::<HeaderValue>().unwrap(),
            "http://localhost:3000".parse::<HeaderValue>().unwrap(),
            "http://127.0.0.1:5173".parse::<HeaderValue>().unwrap(),
            "http://localhost:5173".parse::<HeaderValue>().unwrap(),
        ])
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    Router::new()
        // ── Core endpoints ──────────────────────────────────────────
        .route("/health", get(handlers::health::handle_health))
        .route("/digest", get(handlers::health::handle_digest))
        .route("/savings", get(handlers::health::handle_savings))
        .route("/dump", get(handlers::health::handle_dump))
        .route("/store", post(handlers::store::handle_store))
        .route("/recall", get(handlers::recall::handle_recall))
        .route("/peek", get(handlers::recall::handle_peek))
        .route("/boot", get(handlers::boot::handle_boot))
        .route("/diary", post(handlers::diary::handle_diary))
        .route("/recall/budget", get(handlers::recall::handle_budget_recall))
        .route("/forget", post(handlers::mutate::handle_forget))
        .route("/resolve", post(handlers::mutate::handle_resolve))
        .route("/archive", post(handlers::mutate::handle_archive))
        .route("/shutdown", post(handlers::mutate::handle_shutdown))
        // ── Conductor (locks, activity, messages, sessions, tasks) ──
        .route("/lock", post(handlers::conductor::handle_lock))
        .route("/unlock", post(handlers::conductor::handle_unlock))
        .route("/locks", get(handlers::conductor::handle_locks))
        .route(
            "/activity",
            post(handlers::conductor::handle_post_activity)
                .get(handlers::conductor::handle_get_activity),
        )
        .route("/message", post(handlers::conductor::handle_post_message))
        .route(
            "/messages",
            get(handlers::conductor::handle_get_messages),
        )
        .route(
            "/session/start",
            post(handlers::conductor::handle_session_start),
        )
        .route(
            "/session/heartbeat",
            post(handlers::conductor::handle_session_heartbeat),
        )
        .route(
            "/session/end",
            post(handlers::conductor::handle_session_end),
        )
        .route("/sessions", get(handlers::conductor::handle_sessions))
        .route(
            "/tasks",
            post(handlers::conductor::handle_create_task)
                .get(handlers::conductor::handle_get_tasks),
        )
        .route(
            "/tasks/next",
            get(handlers::conductor::handle_next_task),
        )
        .route(
            "/tasks/claim",
            post(handlers::conductor::handle_claim_task),
        )
        .route(
            "/tasks/complete",
            post(handlers::conductor::handle_complete_task),
        )
        .route(
            "/tasks/abandon",
            post(handlers::conductor::handle_abandon_task),
        )
        // ── Feed ────────────────────────────────────────────────────
        .route(
            "/feed",
            post(handlers::feed::handle_post_feed)
                .get(handlers::feed::handle_get_feed),
        )
        .route("/feed/ack", post(handlers::feed::handle_feed_ack))
        .route(
            "/feed/{id}",
            get(handlers::feed::handle_get_feed_by_id),
        )
        // ── SSE events ──────────────────────────────────────────────
        .route(
            "/events/stream",
            get(handlers::events::handle_events_stream),
        )
        // ── MCP-RPC proxy endpoint ─────────────────────────────────
        // Accepts raw JSON-RPC messages (same format as MCP stdio).
        // This lets `cortex mcp` run as a thin proxy -- no separate
        // ONNX engine, no separate caches, zero duplication.
        .route("/mcp-rpc", post(handle_mcp_rpc))
        .layer(cors)
        .with_state(state)
}

/// HTTP endpoint for MCP proxy -- accepts JSON-RPC, returns JSON-RPC.
/// SEC-001 fix: requires Bearer auth like all other POST mutation endpoints.
async fn handle_mcp_rpc(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(msg): Json<Value>,
) -> Json<Value> {
    if let Err(_resp) = ensure_auth(&headers, &state) {
        return Json(serde_json::json!({
            "jsonrpc": "2.0",
            "error": { "code": -32600, "message": "Unauthorized" },
            "id": msg.get("id")
        }));
    }
    match handle_mcp_message(&state, &msg).await {
        Some(resp) => Json(resp),
        None => Json(serde_json::json!({})),
    }
}

pub async fn run(
    router: Router,
    port: u16,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    eprintln!("[cortex] Listening on http://127.0.0.1:{port}");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
        .ok();
}
