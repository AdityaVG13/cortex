mod aging;
mod auth;
mod co_occurrence;
mod compaction;
mod compiler;
mod conflict;
mod crystallize;
mod db;
mod embeddings;
mod focus;
mod handlers;
mod hook_boot;
mod indexer;
mod logging;
mod mcp_proxy;
mod mcp_stdio;
mod prompt_inject;
mod rate_limit;
mod server;
mod service;
mod setup;
mod state;
mod tls;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match mode {
        // ── HTTP daemon (standalone or via service) ─────────────────
        "serve" => {
            #[cfg(unix)]
            async fn sigterm_future() {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm =
                    signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            async fn sigterm_future() {
                std::future::pending::<()>().await;
            }

            run_daemon(async {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        eprintln!("[cortex] Received Ctrl+C, shutting down...");
                    }
                    _ = sigterm_future() => {
                        eprintln!("[cortex] Received SIGTERM, shutting down...");
                    }
                }
            })
            .await;
        }

        // ── MCP stdio transport ─────────────────────────────────────
        // Tries proxy mode first (thin client → daemon on :7437).
        // Falls back to standalone if daemon is unreachable.
        "mcp" => {
            if mcp_proxy::run().await {
                // Proxy mode handled everything -- clean exit
                return;
            }

            // Fallback: standalone MCP (stdio only, no daemon pretending).
            // Arch review fix: do NOT write PID or bind HTTP port.
            // A standalone MCP session is not a daemon and must not conflict with one.
            eprintln!("[cortex-mcp] Running standalone -- start the daemon for shared state");
            let db_path = auth::db_path();
            eprintln!("[cortex-mcp] DB: {}", db_path.display());

            let (mcp_state, _shutdown_rx) =
                state::initialize(&db_path, false).expect("Failed to initialize state");

            mcp_stdio::run(mcp_state.clone()).await;
            eprintln!("[cortex-mcp] MCP session ended.");

            let conn = mcp_state.db.lock().await;
            if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")
            {
                eprintln!("[cortex-mcp] Warning: WAL checkpoint failed: {e}");
            }
        }

        // ── Hook: SessionStart (replaces brain-boot.js) ─────────────
        "hook-boot" => {
            let agent = args
                .get(2)
                .and_then(|a| {
                    if a == "--agent" {
                        args.get(3).map(|s| s.as_str())
                    } else {
                        Some(a.as_str())
                    }
                })
                .unwrap_or("claude-opus");
            hook_boot::run_boot(agent).await;
        }

        // ── Hook: Statusline one-liner ──────────────────────────────
        "hook-status" => {
            hook_boot::run_status().await;
        }

        // ── Windows Service lifecycle ───────────────────────────────
        "service" => {
            let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
            match subcmd {
                "install" => service::install(),
                "uninstall" => service::uninstall(),
                "start" => service::start(),
                "stop" => service::stop(),
                "status" => service::status(),
                _ => {
                    eprintln!("Usage: cortex service <install|uninstall|start|stop|status>");
                }
            }
        }

        // ── Windows Service entry point (called by SCM) ─────────────
        "service-run" => {
            service::dispatch_service();
        }

        // ── System prompt injector CLI ──────────────────────────────
        "prompt-inject" => {
            let remaining: Vec<String> = args[2..].to_vec();
            prompt_inject::run(&remaining).await;
        }

        // ── Setup: detect AI tools, configure, verify ──────────────
        "setup" => {
            setup::run_setup().await;
        }

        _ => {
            eprintln!(
                "Cortex v{} -- Universal AI Memory Daemon",
                env!("CARGO_PKG_VERSION")
            );
            eprintln!();
            eprintln!("Usage: cortex <command>");
            eprintln!();
            eprintln!("Setup:");
            eprintln!("  setup              First-run setup: detect AI tools, configure, verify");
            eprintln!();
            eprintln!("Daemon:");
            eprintln!("  serve              HTTP daemon on :7437");
            eprintln!("  mcp                MCP stdio (proxy to daemon, standalone fallback)");
            eprintln!();
            eprintln!("Hooks:");
            eprintln!("  hook-boot [AGENT]  SessionStart hook (default: claude-opus)");
            eprintln!("  hook-status        Statusline one-liner");
            eprintln!();
            eprintln!("Tools:");
            eprintln!("  prompt-inject      Inject Cortex context into system prompt files");
            eprintln!();
            eprintln!("Service:");
            eprintln!("  service install    Register as Windows Service (auto-start)");
            eprintln!("  service uninstall  Remove Windows Service");
            eprintln!("  service start      Start the service");
            eprintln!("  service stop       Stop the service");
            eprintln!("  service status     Check service status");
            std::process::exit(1);
        }
    }
}

// ── Shared daemon logic (used by `serve` and `service-run`) ─────────────────

/// Run the full Cortex daemon. The `extra_shutdown` future is an additional
/// shutdown trigger beyond the HTTP /shutdown endpoint:
/// - `serve` passes Ctrl+C / SIGTERM
/// - `service-run` passes the SCM stop signal
pub(crate) async fn run_daemon(
    extra_shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) {
    auth::kill_stale_daemon();
    let db_path = auth::db_path();
    eprintln!(
        "[cortex] Starting Cortex v{} (Rust)...",
        env!("CARGO_PKG_VERSION")
    );
    eprintln!("[cortex] DB: {}", db_path.display());

    let (state, shutdown_rx) =
        state::initialize(&db_path, true).expect("Failed to initialize state");

    auth::write_pid();
    let token_path = auth::cortex_dir().join("cortex.token");
    let pid_path = auth::cortex_dir().join("cortex.pid");
    eprintln!("[cortex] Auth token at {}", token_path.display());
    eprintln!(
        "[cortex] PID {} written to {}",
        std::process::id(),
        pid_path.display()
    );

    // ── Schema migrations (idempotent) ──────────────────────────────
    {
        let conn = state.db.lock().await;
        db::migrate_aging_columns(&conn);
        db::migrate_focus_table(&conn);
        crystallize::migrate_crystal_tables(&conn);
    }

    // ── Knowledge indexing + score decay ────────────────────────────
    {
        let conn = state.db.lock().await;
        let indexed = indexer::index_all(&conn, &state.home);
        let decayed = indexer::decay_pass(&conn);
        eprintln!("[cortex] Indexed {indexed} entries, decayed {decayed} scores");
    }

    // ── Background embedding builder ────────────────────────────────
    if let Some(engine) = state.embedding_engine.clone() {
        let db = state.db.clone();
        tokio::spawn(async move {
            build_embeddings_async(&engine, &db).await;
        });
    } else {
        tokio::spawn(async {
            if let Some(dir) = embeddings::ensure_model_downloaded().await {
                eprintln!(
                    "[embeddings] Model ready at {} -- restart to activate",
                    dir.display()
                );
            }
        });
    }

    // ── Background WAL checkpoint every 60s ───────────────────────────
    {
        let db_wal = state.db.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                let conn = db_wal.lock().await;
                db::checkpoint_wal_best_effort(&conn);
            }
        });
    }

    // ── Background aging pass every 6 hours ──────────────────────────
    {
        let db_aging = state.db.clone();
        tokio::spawn(async move {
            // Run initial aging pass on startup
            {
                let conn = db_aging.lock().await;
                let (compressed, archived) = aging::run_aging_pass(&conn);
                if compressed > 0 || archived > 0 {
                    eprintln!(
                        "[cortex] Initial aging: {compressed} compressed, {archived} archived"
                    );
                }
            }
            // Then run every 6 hours
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                let conn = db_aging.lock().await;
                aging::run_aging_pass(&conn);
                compaction::run_compaction(&conn);
            }
        });
    }

    // ── Background crystallization pass every 2 hours ─────────────
    {
        let db_crystal = state.db.clone();
        let engine_crystal = state.embedding_engine.clone();
        tokio::spawn(async move {
            // Initial pass on startup (after embeddings are built)
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            {
                let conn = db_crystal.lock().await;
                let result = crystallize::run_crystallize_pass(&conn, engine_crystal.as_deref());
                if result.crystals_created > 0 || result.crystals_updated > 0 {
                    eprintln!(
                        "[cortex] Initial crystallization: {} created, {} updated",
                        result.crystals_created, result.crystals_updated
                    );
                }
            }
            // Then run every 2 hours
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                let conn = db_crystal.lock().await;
                crystallize::run_crystallize_pass(&conn, engine_crystal.as_deref());
            }
        });
    }

    // ── Background rate limiter cleanup every 5 minutes ────────────
    {
        let rl = state.rate_limiter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await;
            loop {
                interval.tick().await;
                rl.cleanup().await;
            }
        });
    }

    let db_for_shutdown = state.db.clone();
    let router = server::build_router(state);

    // Combine shutdown sources: HTTP /shutdown, extra (Ctrl+C or SCM stop)
    let shutdown_future = async {
        tokio::select! {
            _ = shutdown_rx => {
                eprintln!("[cortex] Shutdown requested via HTTP");
            }
            _ = extra_shutdown => {}
        }
    };

    server::run(router, 7437, shutdown_future).await;

    // WAL checkpoint + cleanup
    eprintln!("[cortex] Flushing database...");
    {
        let conn = db_for_shutdown.lock().await;
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;") {
            eprintln!("[cortex] Warning: WAL checkpoint failed: {e}");
        }
    }

    let _ = std::fs::remove_file(&pid_path);
    eprintln!("[cortex] Shutdown complete.");
}

/// Build embeddings for all un-embedded memories and decisions.
/// IMPORTANT: Does NOT hold the DB lock during ONNX inference.
/// Reads IDs/text in a short lock, embeds in memory (no lock), then writes in batches.
async fn build_embeddings_async(
    engine: &embeddings::EmbeddingEngine,
    db: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
) {
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
    };

    let total = unembedded_mem.len() + unembedded_dec.len();
    if total == 0 {
        return;
    }

    eprintln!("[embeddings] Building embeddings for {total} entries...");
    let mut computed = 0;

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
