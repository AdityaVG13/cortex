// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
use crate::handlers;
use crate::handlers::ensure_auth;
use crate::handlers::mcp::handle_mcp_message_with_caller;
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde_json::Value;
use std::path::Path;
use tower_http::cors::CorsLayer;

pub fn build_router(state: RuntimeState, port: u16) -> Router {
    // SEC-001: restrict CORS to localhost origins only.
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

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
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
        .route(
            "/recall/budget",
            get(handlers::recall::handle_budget_recall),
        )
        .route("/feedback", post(handlers::feedback::handle_feedback))
        .route(
            "/feedback/stats",
            get(handlers::feedback::handle_feedback_stats),
        )
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
        .route("/messages", get(handlers::conductor::handle_get_messages))
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
        .route("/tasks/next", get(handlers::conductor::handle_next_task))
        .route("/tasks/claim", post(handlers::conductor::handle_claim_task))
        .route(
            "/tasks/complete",
            post(handlers::conductor::handle_complete_task),
        )
        .route(
            "/tasks/abandon",
            post(handlers::conductor::handle_abandon_task),
        )
        .route("/tasks/delete", post(handlers::conductor::handle_delete_task))
        // ── Feed ────────────────────────────────────────────────────
        .route(
            "/feed",
            post(handlers::feed::handle_post_feed).get(handlers::feed::handle_get_feed),
        )
        .route("/feed/ack", post(handlers::feed::handle_feed_ack))
        .route("/feed/{id}", get(handlers::feed::handle_get_feed_by_id))
        // ── Export / Import ────────────────────────────────────────
        .route("/export", get(handlers::export::handle_export))
        .route("/import", post(handlers::export::handle_import))
        // ── Admin (team-mode only, owner/admin role required) ──
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

    let caller_id = match handlers::ensure_auth_with_caller(&headers, &state) {
        Ok(caller_id) => caller_id,
        Err(_) => {
            eprintln!("[cortex] Failed to resolve MCP caller after auth succeeded");
            None
        }
    };
    match handle_mcp_message_with_caller(&state, &msg, caller_id).await {
        Some(resp) => Json(resp),
        None => Json(serde_json::json!({})),
    }
}

// ─── Compaction handlers ────────────────────────────────────────────────

async fn handle_compact(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> axum::response::Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let conn = state.db.lock().await;
    let breakdown = crate::compaction::storage_breakdown(&conn);
    let total_bytes: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        * conn
            .query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
            .unwrap_or(4096);

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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let conn = state.db.lock().await;
    let result = crate::crystallize::run_crystallize_pass(&conn, state.embedding_engine.as_deref(), state.default_owner_id);
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => {
            return handlers::json_error(
                axum::http::StatusCode::BAD_REQUEST,
                "Missing field: label",
            )
        }
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
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let label = match &body.label {
        Some(l) if !l.is_empty() => l.as_str(),
        _ => {
            return handlers::json_error(
                axum::http::StatusCode::BAD_REQUEST,
                "Missing field: label",
            )
        }
    };
    let agent = body.agent.as_deref().unwrap_or("http");
    let conn = state.db.lock().await;
    match crate::focus::focus_end(&conn, label, agent, state.default_owner_id) {
        Ok(v) => handlers::json_response(axum::http::StatusCode::OK, v),
        Err(e) => handlers::json_error(axum::http::StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

pub async fn run(
    router: Router,
    port: u16,
    db_path: &Path,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let bind_addr = std::env::var("CORTEX_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());

    match crate::tls::try_load_tls() {
        Ok(Some(acceptor)) => {
            run_tls(router, &bind_addr, port, acceptor, shutdown).await;
        }
        Ok(None) => {
            run_plain(router, &bind_addr, port, shutdown).await;
        }
        Err(e) => {
            // Team mode: refuse to start with broken TLS (auth integrity requires it)
            // Solo mode: warn and fall back to plain HTTP
            let team_mode = detect_team_mode_for_tls(db_path);
            if team_mode {
                eprintln!("[cortex] TLS configuration error: {e}");
                eprintln!("[cortex] Team mode requires valid TLS -- fix certs at ~/.cortex/tls/ or set CORTEX_TLS_CERT/CORTEX_TLS_KEY");
                std::process::exit(1);
            } else {
                eprintln!("[cortex] TLS certificate error: {e}");
                eprintln!("[cortex] Starting without TLS (solo mode -- localhost only)");
                run_plain(router, &bind_addr, port, shutdown).await;
            }
        }
    }
}

/// Lightweight team-mode detection for TLS decisions (before full state init).
/// Opens the DB briefly to read the config table.
fn detect_team_mode_for_tls(db_path: &Path) -> bool {
    if let Ok(conn) = crate::db::open(db_path) {
        crate::db::is_team_mode(&conn)
    } else {
        false
    }
}

async fn run_plain(
    router: Router,
    bind_addr: &str,
    port: u16,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let listener = match tokio::net::TcpListener::bind((bind_addr, port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[cortex] FATAL: Cannot bind to {bind_addr}:{port} -- {e}");
            eprintln!("[cortex] Is another Cortex instance running? Try: cortex paths --json");
            std::process::exit(1);
        }
    };
    eprintln!("[cortex] Listening on http://{bind_addr}:{port}");
    if let Err(e) = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
    {
        eprintln!("[cortex] HTTP server exited with error: {e}");
    }
}

async fn run_tls(
    router: Router,
    bind_addr: &str,
    port: u16,
    acceptor: tokio_rustls::TlsAcceptor,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    use tokio::net::TcpListener;

    let listener = match TcpListener::bind((bind_addr, port)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[cortex] FATAL: Cannot bind to {bind_addr}:{port} -- {e}");
            eprintln!("[cortex] Is another Cortex instance running? Try: cortex paths --json");
            std::process::exit(1);
        }
    };
    eprintln!("[cortex] Listening on https://{bind_addr}:{port} (TLS via rustls)");

    let mut make_svc = router.into_make_service();

    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                eprintln!("[cortex] TLS server shutting down");
                break;
            }
            accept = listener.accept() => {
                match accept {
                    Ok((stream, _addr)) => {
                        let acceptor = acceptor.clone();
                        let svc = tower::MakeService::<&std::net::SocketAddr, axum::http::Request<axum::body::Body>>::make_service(&mut make_svc, &_addr);
                        tokio::spawn(async move {
                            match acceptor.accept(stream).await {
                                Ok(tls_stream) => {
                                    let tower_svc = match svc.await {
                                        Ok(tower_svc) => tower_svc,
                                        Err(e) => {
                                            eprintln!("[cortex] Failed to build TLS service for {_addr}: {e}");
                                            return;
                                        }
                                    };
                                    let hyper_svc = hyper_util::service::TowerToHyperService::new(tower_svc);
                                    let io = hyper_util::rt::TokioIo::new(tls_stream);
                                    if let Err(e) = hyper_util::server::conn::auto::Builder::new(
                                        hyper_util::rt::TokioExecutor::new(),
                                    )
                                    .serve_connection(io, hyper_svc)
                                    .await
                                    {
                                        eprintln!("[cortex] TLS connection error for {_addr}: {e}");
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[cortex] TLS handshake failed: {e}");
                                }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("[cortex] TCP accept error: {e}");
                    }
                }
            }
        }
    }
}

fn parse_allowed_origin(origin: &str) -> Option<HeaderValue> {
    match origin.parse::<HeaderValue>() {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("[cortex] Invalid CORS origin '{origin}': {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    async fn build_state(team_mode: bool) -> RuntimeState {
        let mut db_path = std::env::temp_dir();
        let suffix = if team_mode { "team" } else { "solo" };
        db_path.push(format!(
            "cortex-api-parity-{suffix}-{}.db",
            uuid::Uuid::new_v4()
        ));

        let conn = crate::db::open(&db_path).unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::migrate_focus_table(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);
        if team_mode {
            crate::db::create_team_mode_tables(&conn).unwrap();
            let owner_id =
                crate::db::upsert_owner_user(&conn, "owner", Some("Owner"), "argon2id-placeholder")
                    .unwrap();
            crate::db::migrate_to_team_mode(&conn, owner_id).unwrap();
        }
        drop(conn);

        let (state, _shutdown_rx) = crate::state::initialize(&db_path, false).unwrap();
        let _ = std::fs::remove_file(db_path);
        state
    }

    async fn route_status(
        router: &Router,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> StatusCode {
        let mut req = Request::builder().method(method).uri(path);
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let req = req
            .body(Body::from(body.unwrap_or_default().to_string()))
            .unwrap();
        router.clone().oneshot(req).await.unwrap().status()
    }

    #[tokio::test]
    async fn test_non_admin_routes_preserved_across_team_migration() {
        let solo_router = build_router(build_state(false).await, 7437);
        let team_router = build_router(build_state(true).await, 7437);

        let cases: Vec<(Method, &str, Option<&str>)> = vec![
            (Method::GET, "/health", None),
            (Method::GET, "/digest", None),
            (Method::GET, "/savings", None),
            (Method::GET, "/dump", None),
            (Method::POST, "/store", Some("{}")),
            (Method::GET, "/recall", None),
            (Method::GET, "/peek", None),
            (Method::GET, "/unfold", None),
            (Method::GET, "/boot", None),
            (Method::POST, "/diary", Some("{}")),
            (Method::GET, "/recall/budget", None),
            (Method::POST, "/feedback", Some("{}")),
            (Method::GET, "/feedback/stats", None),
            (Method::GET, "/crystals", None),
            (Method::POST, "/crystallize", Some("{}")),
            (Method::POST, "/compact", Some("{}")),
            (Method::GET, "/storage", None),
            (Method::POST, "/forget", Some("{}")),
            (Method::POST, "/resolve", Some("{}")),
            (Method::GET, "/conflicts", None),
            (Method::POST, "/archive", Some("{}")),
            (Method::POST, "/focus/start", Some("{}")),
            (Method::POST, "/focus/end", Some("{}")),
            (Method::POST, "/shutdown", Some("{}")),
            (Method::POST, "/lock", Some("{}")),
            (Method::POST, "/unlock", Some("{}")),
            (Method::GET, "/locks", None),
            (Method::POST, "/activity", Some("{}")),
            (Method::GET, "/activity", None),
            (Method::POST, "/message", Some("{}")),
            (Method::GET, "/messages", None),
            (Method::POST, "/session/start", Some("{}")),
            (Method::POST, "/session/heartbeat", Some("{}")),
            (Method::POST, "/session/end", Some("{}")),
            (Method::GET, "/sessions", None),
            (Method::POST, "/tasks", Some("{}")),
            (Method::GET, "/tasks", None),
            (Method::GET, "/tasks/next", None),
            (Method::POST, "/tasks/claim", Some("{}")),
            (Method::POST, "/tasks/complete", Some("{}")),
            (Method::POST, "/tasks/abandon", Some("{}")),
            (Method::POST, "/tasks/delete", Some("{}")),
            (Method::POST, "/feed", Some("{}")),
            (Method::GET, "/feed", None),
            (Method::POST, "/feed/ack", Some("{}")),
            (Method::GET, "/feed/demo", None),
            (Method::GET, "/export", None),
            (Method::POST, "/import", Some("{}")),
            (Method::GET, "/events/stream", None),
            (Method::POST, "/mcp-rpc", Some("{}")),
        ];

        for (method, path, body) in cases {
            let solo_status = route_status(&solo_router, method.clone(), path, body).await;
            let team_status = route_status(&team_router, method, path, body).await;

            assert_ne!(solo_status, StatusCode::NOT_FOUND, "solo missing {path}");
            assert_ne!(team_status, StatusCode::NOT_FOUND, "team missing {path}");
            assert_eq!(
                solo_status, team_status,
                "status drift for route {path}: solo={solo_status} team={team_status}"
            );
        }
    }
}

