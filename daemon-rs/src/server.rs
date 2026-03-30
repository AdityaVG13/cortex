use axum::routing::{get, post};
use axum::Router;

use crate::handlers;
use crate::state::RuntimeState;

pub fn build_router(state: RuntimeState) -> Router {
    Router::new()
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
