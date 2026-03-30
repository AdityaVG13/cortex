mod auth;
mod co_occurrence;
mod compiler;
mod conflict;
mod db;
mod embeddings;
mod handlers;
mod indexer;
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
            let token_path = auth::cortex_dir().join("cortex.token");
            let pid_path = auth::cortex_dir().join("cortex.pid");
            eprintln!("[cortex] Auth token at {}", token_path.display());
            eprintln!(
                "[cortex] PID {} written to {}",
                std::process::id(),
                pid_path.display()
            );

            // Clone the DB handle before state is moved into the router.
            let db_for_shutdown = state.db.clone();

            let router = server::build_router(state);

            // Combine shutdown sources: HTTP /shutdown endpoint OR Ctrl+C/SIGTERM.
            //
            // On Unix we also listen for SIGTERM; on Windows that signal does not exist
            // so we use a future that never resolves as a no-op third branch.
            #[cfg(unix)]
            async fn sigterm_future() {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = signal(SignalKind::terminate())
                    .expect("Failed to register SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            async fn sigterm_future() {
                std::future::pending::<()>().await;
            }

            let shutdown_future = async {
                tokio::select! {
                    _ = shutdown_rx => {
                        eprintln!("[cortex] Shutdown requested via HTTP");
                    }
                    _ = tokio::signal::ctrl_c() => {
                        eprintln!("[cortex] Received Ctrl+C, shutting down...");
                    }
                    _ = sigterm_future() => {
                        eprintln!("[cortex] Received SIGTERM, shutting down...");
                    }
                }
            };

            server::run(router, 7437, shutdown_future).await;

            // After server stops: WAL checkpoint + DB cleanup.
            eprintln!("[cortex] Flushing database...");
            {
                let conn = db_for_shutdown.lock().await;
                if let Err(e) =
                    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")
                {
                    eprintln!("[cortex] Warning: WAL checkpoint failed: {e}");
                }
            }

            // Clean up PID file.
            let _ = std::fs::remove_file(&pid_path);
            eprintln!("[cortex] Shutdown complete.");
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

            // Run MCP stdio transport — blocks until stdin is closed.
            mcp_stdio::run(state.clone()).await;
            eprintln!("[cortex-mcp] MCP session ended.");

            // After stdin closes: checkpoint WAL so nothing is stranded in the journal.
            {
                let conn = state.db.lock().await;
                if let Err(e) =
                    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")
                {
                    eprintln!("[cortex-mcp] Warning: WAL checkpoint failed: {e}");
                }
            }
        }
        _ => {
            eprintln!("Usage: cortex <serve|mcp>");
            eprintln!("  serve — HTTP daemon only (standalone)");
            eprintln!("  mcp   — MCP stdio + HTTP daemon (for Claude Code)");
            std::process::exit(1);
        }
    }
}
