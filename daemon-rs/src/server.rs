use axum::routing::{get, post};
use axum::Router;

use crate::handlers;
use crate::state::RuntimeState;

pub fn build_router(state: RuntimeState) -> Router {
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
        .with_state(state)
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
