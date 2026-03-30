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
            auth::kill_stale_daemon();
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

            // ── Knowledge indexing + score decay ────────────────────────
            {
                let conn = state.db.lock().await;
                let indexed = indexer::index_all(&conn, &state.home);
                let decayed = indexer::decay_pass(&conn);
                eprintln!("[cortex] Indexed {indexed} entries, decayed {decayed} scores");
            }

            // ── Background embedding builder ─────────────────────────
            // IMPORTANT: Don't hold DB lock during ONNX inference.
            // Read unembedded IDs + text, drop lock, embed in memory,
            // then re-lock briefly to write each batch.
            if let Some(engine) = state.embedding_engine.clone() {
                let db = state.db.clone();
                tokio::spawn(async move {
                    build_embeddings_async(&engine, &db).await;
                });
            } else {
                // Try downloading model in background for next restart.
                tokio::spawn(async {
                    if let Some(dir) = embeddings::ensure_model_downloaded().await {
                        eprintln!("[embeddings] Model ready at {} -- restart to activate", dir.display());
                    }
                });
            }

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

/// Build embeddings for all un-embedded memories and decisions.
/// IMPORTANT: Does NOT hold the DB lock during ONNX inference.
/// Reads IDs/text in a short lock, embeds in memory (no lock), then writes in batches.
async fn build_embeddings_async(
    engine: &embeddings::EmbeddingEngine,
    db: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
) {
    // Step 1: Read un-embedded entries (short lock).
    let (unembedded_mem, unembedded_dec) = {
        let conn = db.lock().await;

        let mem: Vec<(i64, String)> = conn
            .prepare(
                "SELECT m.id, m.text FROM memories m \
                 WHERE m.status = 'active' \
                   AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'memory' AND e.target_id = m.id)",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        let dec: Vec<(i64, String)> = conn
            .prepare(
                "SELECT d.id, d.decision FROM decisions d \
                 WHERE d.status = 'active' \
                   AND NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.target_type = 'decision' AND e.target_id = d.id)",
            )
            .and_then(|mut stmt| {
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
            })
            .unwrap_or_default();

        (mem, dec)
    }; // DB lock released here.

    let total = unembedded_mem.len() + unembedded_dec.len();
    if total == 0 {
        return;
    }

    eprintln!("[embeddings] Building embeddings for {total} entries...");
    let mut computed = 0;

    // Step 2: Embed in memory (no lock held).
    let mut mem_results: Vec<(i64, Vec<u8>)> = Vec::new();
    for (id, text) in &unembedded_mem {
        if let Some(vec) = engine.embed(text) {
            mem_results.push((*id, embeddings::vector_to_blob(&vec)));
            computed += 1;
        }
    }

    let mut dec_results: Vec<(i64, Vec<u8>)> = Vec::new();
    for (id, text) in &unembedded_dec {
        if let Some(vec) = engine.embed(text) {
            dec_results.push((*id, embeddings::vector_to_blob(&vec)));
            computed += 1;
        }
    }

    // Step 3: Write results in a single short lock.
    {
        let conn = db.lock().await;
        for (id, blob) in &mem_results {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                 VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
                rusqlite::params![id, blob],
            );
        }
        for (id, blob) in &dec_results {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                 VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
                rusqlite::params![id, blob],
            );
        }
    }

    eprintln!("[embeddings] Built {computed}/{total} embeddings");
}
