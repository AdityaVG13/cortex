mod auth;
mod co_occurrence;
mod compiler;
mod conflict;
mod db;
mod handlers;
mod logging;
mod mcp_stdio;
mod server;
mod state;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match mode {
        "serve" => {
            let db_path = auth::db_path();
            eprintln!("[cortex] Starting Cortex v2.1.0 (Rust)...");
            eprintln!("[cortex] DB: {}", db_path.display());

            let (state, shutdown_rx) =
                state::initialize(&db_path).expect("Failed to initialize state");

            auth::write_pid();
            eprintln!(
                "[cortex] Auth token at {}",
                auth::cortex_dir().join("cortex.token").display()
            );
            eprintln!("[cortex] PID {} written", std::process::id());

            let router = server::build_router(state);
            server::run(router, 7437, async {
                shutdown_rx.await.ok();
            })
            .await;

            eprintln!("[cortex] Shut down cleanly.");
        }
        "mcp" => {
            let db_path = auth::db_path();
            // All log output goes to stderr in MCP mode — stdout is reserved for JSON-RPC
            eprintln!("[cortex] Starting Cortex v2.1.0 (Rust, MCP mode)...");
            eprintln!("[cortex] DB: {}", db_path.display());

            let (state, _shutdown_rx) =
                state::initialize(&db_path).expect("Failed to initialize state");

            auth::write_pid();
            eprintln!(
                "[cortex] Auth token at {}",
                auth::cortex_dir().join("cortex.token").display()
            );
            eprintln!("[cortex] PID {} written", std::process::id());

            // Start HTTP server in background (non-blocking)
            // If the port is already in use (serve mode running), skip silently
            let http_state = state.clone();
            let http_router = server::build_router(http_state);
            tokio::spawn(async move {
                match tokio::net::TcpListener::bind(("127.0.0.1", 7437)).await {
                    Ok(listener) => {
                        eprintln!("[cortex-mcp] HTTP server also listening on http://127.0.0.1:7437");
                        let _ = axum::serve(listener, http_router).await;
                    }
                    Err(_) => {
                        eprintln!("[cortex-mcp] Port 7437 already in use — HTTP serve mode likely running. MCP uses stdio only.");
                    }
                }
            });

            // Run MCP stdio transport — blocks until stdin is closed
            mcp_stdio::run(state).await;
            eprintln!("[cortex-mcp] MCP session ended.");
        }
        _ => {
            eprintln!("Usage: cortex <serve|mcp>");
            eprintln!("  serve — HTTP daemon only (standalone)");
            eprintln!("  mcp   — MCP stdio + HTTP daemon (for Claude Code)");
            std::process::exit(1);
        }
    }
}
