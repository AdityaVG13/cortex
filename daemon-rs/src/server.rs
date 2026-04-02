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
    // SEC-001: restrict CORS to localhost origins only.
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
        // ── Public endpoints (no auth) ─────────────────────────────
        .route("/health", get(handlers::health::handle_health))
        // ── Core endpoints ─────────────────────────────────────────
        // boot and recall already accept HeaderMap and call ensure_auth.
        // digest, savings, peek, budget_recall now have auth added to
        // their handler bodies directly.
        .route("/digest", get(handlers::health::handle_digest))
        .route("/savings", get(handlers::health::handle_savings))
        .route("/dump", get(handlers::health::handle_dump))
        .route("/store", post(handlers::store::handle_store))
        .route("/recall", get(handlers::recall::handle_recall))
        .route("/peek", get(handlers::recall::handle_peek))
        .route("/unfold", get(handlers::recall::handle_unfold))
        .route("/boot", get(handlers::boot::handle_boot))
        .route("/diary", post(handlers::diary::handle_diary))
        .route("/recall/budget", get(handlers::recall::handle_budget_recall))
        .route("/feedback", post(handlers::feedback::handle_feedback))
        .route("/feedback/stats", get(handlers::feedback::handle_feedback_stats))
        .route("/crystals", get(handle_crystals))
        .route("/crystallize", post(handle_crystallize))
        .route("/compact", post(handle_compact))
        .route("/storage", get(handle_storage))
        .route("/forget", post(handlers::mutate::handle_forget))
        .route("/resolve", post(handlers::mutate::handle_resolve))
        .route("/conflicts", get(handlers::mutate::handle_conflicts))
        .route("/archive", post(handlers::mutate::handle_archive))
        .route("/focus/start", post(handle_focus_start))
        .route("/focus/end", post(handle_focus_end))
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

// ─── Compaction handlers ────────────────────────────────────────────────

async fn handle_compact(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let conn = state.db.lock().await;
    let result = crate::compaction::run_compaction(&conn);
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({
            "eventsPruned": result.events_pruned,
            "archivedTextStripped": result.archived_text_stripped,
            "crystalEmbeddingsPruned": result.crystal_embeddings_pruned,
            "feedbackAggregated": result.feedback_aggregated,
            "bytesBefore": result.bytes_before,
            "bytesAfter": result.bytes_after,
            "savedKB": (result.bytes_before - result.bytes_after) / 1024,
        }),
    )
}

async fn handle_storage(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let conn = state.db.lock().await;
    let breakdown = crate::compaction::storage_breakdown(&conn);
    let total_bytes: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        * conn.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0)).unwrap_or(4096);

    let tables: Vec<serde_json::Value> = breakdown
        .iter()
        .map(|(name, count)| serde_json::json!({"table": name, "rows": count}))
        .collect();

    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({
            "totalBytes": total_bytes,
            "totalMB": format!("{:.1}", total_bytes as f64 / 1_048_576.0),
            "tables": tables,
        }),
    )
}

// ─── Crystal handlers ───────────────────────────────────────────────────

async fn handle_crystals(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let conn = state.db.lock().await;
    let crystals = crate::crystallize::list_crystals(&conn);
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({ "crystals": crystals, "count": crystals.len() }),
    )
}

async fn handle_crystallize(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let conn = state.db.lock().await;
    let result = crate::crystallize::run_crystallize_pass(
        &conn,
        state.embedding_engine.as_deref(),
    );
    handlers::json_response(
        axum::http::StatusCode::OK,
        serde_json::json!({
            "clusters": result.clusters_found,
            "created": result.crystals_created,
            "updated": result.crystals_updated,
            "consolidated": result.entries_consolidated,
        }),
    )
}

// ─── Focus handlers (thin wrappers around focus.rs) ──────────────────────

#[derive(serde::Deserialize)]
struct FocusRequest {
    label: Option<String>,
    agent: Option<String>,
}

async fn handle_focus_start(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FocusRequest>,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => return handlers::json_error(axum::http::StatusCode::BAD_REQUEST, "Missing field: label"),
    };
    let agent = body.agent.as_deref().unwrap_or("http");
    let conn = state.db.lock().await;
    match crate::focus::focus_start(&conn, label, agent) {
        Ok(v) => handlers::json_response(axum::http::StatusCode::OK, v),
        Err(e) => handlers::json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn handle_focus_end(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FocusRequest>,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) { return resp; }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => return handlers::json_error(axum::http::StatusCode::BAD_REQUEST, "Missing field: label"),
    };
    let agent = body.agent.as_deref().unwrap_or("http");
    let conn = state.db.lock().await;
    match crate::focus::focus_end(&conn, label, agent) {
        Ok(v) => handlers::json_response(axum::http::StatusCode::OK, v),
        Err(e) => handlers::json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
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
