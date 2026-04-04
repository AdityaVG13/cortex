mod aging;
mod auth;
mod co_occurrence;
mod compaction;
mod compiler;
mod conflict;
mod crystallize;
mod db;
mod embeddings;
mod export_data;
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
            let remaining: Vec<String> = args[2..].to_vec();
            if remaining.iter().any(|a| a == "--team") {
                setup::run_setup_team(&remaining).await;
            } else {
                setup::run_setup().await;
            }
        }

        // ── Data export/import CLI ──────────────────────────────────
        "export" => {
            let remaining: Vec<String> = args[2..].to_vec();
            run_export_cli(&remaining);
        }
        "import" => {
            let remaining: Vec<String> = args[2..].to_vec();
            run_import_cli(&remaining);
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
            eprintln!("  setup --team       Team-mode setup + schema migration + owner API key");
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
            eprintln!("  export             Export data (--format json|sql, --out <file>)");
            eprintln!(
                "  import             Import JSON data (--file <path>, optional --user <username>)"
            );
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

fn run_export_cli(args: &[String]) {
    let mut format = "json".to_string();
    let mut out_path: Option<String> = None;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if let Some(v) = args.get(i + 1) {
                    format = v.to_string();
                    i += 1;
                }
            }
            "--out" => {
                if let Some(v) = args.get(i + 1) {
                    out_path = Some(v.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(export_format) = export_data::ExportFormat::parse(&format) else {
        eprintln!("Usage: cortex export --format json|sql [--out <path>]");
        std::process::exit(1);
    };

    let db_path = auth::db_path();
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to open database at {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("Failed to configure database: {e}");
        std::process::exit(1);
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("Failed to initialize schema: {e}");
        std::process::exit(1);
    }
    crystallize::migrate_crystal_tables(&conn);

    let output = match export_format {
        export_data::ExportFormat::Json => {
            let value = export_data::export_json_value(&conn);
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
        }
        export_data::ExportFormat::Sql => export_data::export_sql_text(&conn),
    };

    if let Some(path) = out_path {
        if let Err(e) = std::fs::write(&path, output) {
            eprintln!("Failed to write export file {path}: {e}");
            std::process::exit(1);
        }
        eprintln!("Exported to {path}");
    } else {
        println!("{output}");
    }
}

fn run_import_cli(args: &[String]) {
    let mut file_path: Option<String> = None;
    let mut username: Option<String> = None;
    let mut visibility = "private".to_string();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                if let Some(v) = args.get(i + 1) {
                    file_path = Some(v.to_string());
                    i += 1;
                }
            }
            "--user" => {
                if let Some(v) = args.get(i + 1) {
                    username = Some(v.to_string());
                    i += 1;
                }
            }
            "--visibility" => {
                if let Some(v) = args.get(i + 1) {
                    visibility = v.to_string();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(file_path) = file_path else {
        eprintln!("Usage: cortex import --file <path> [--user <username>] [--visibility private|team|shared]");
        std::process::exit(1);
    };
    if !matches!(visibility.as_str(), "private" | "team" | "shared") {
        eprintln!("Invalid --visibility value '{visibility}'. Use private|team|shared.");
        std::process::exit(1);
    }

    let raw = match std::fs::read_to_string(&file_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Cannot read import file {file_path}: {e}");
            std::process::exit(1);
        }
    };
    let payload: export_data::ImportPayload = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Import file is not valid JSON: {e}");
            std::process::exit(1);
        }
    };

    let db_path = auth::db_path();
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Failed to open database at {}: {e}", db_path.display());
            std::process::exit(1);
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("Failed to configure database: {e}");
        std::process::exit(1);
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("Failed to initialize schema: {e}");
        std::process::exit(1);
    }
    crystallize::migrate_crystal_tables(&conn);

    let team_mode = db::current_mode(&conn) == "team";
    if username.is_some() && !team_mode {
        eprintln!("--user import requires team mode. Run: cortex setup --team");
        std::process::exit(1);
    }
    let owner_id = if team_mode {
        if let Some(user) = username {
            match conn.query_row(
                "SELECT id FROM users WHERE username = ?1",
                rusqlite::params![user.clone()],
                |row| row.get::<_, i64>(0),
            ) {
                Ok(id) => Some(id),
                Err(_) => {
                    eprintln!("Unknown user '{user}'. Create the user before import.");
                    std::process::exit(1);
                }
            }
        } else {
            conn.query_row(
                "SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .or_else(|| {
                conn.query_row(
                    "SELECT id FROM users ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, id ASC LIMIT 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .ok()
            })
        }
    } else {
        None
    };
    if team_mode && owner_id.is_none() {
        eprintln!("Team mode import requires a target owner. Run `cortex setup --team` first.");
        std::process::exit(1);
    }

    let options = export_data::ImportOptions {
        owner_id,
        visibility: if team_mode { Some(visibility) } else { None },
        source_agent_fallback: "import-cli".to_string(),
    };
    let counts = export_data::import_payload(&conn, &payload, &options);
    println!(
        "{{\"imported\":{{\"memories\":{},\"decisions\":{}}}}}",
        counts.memories, counts.decisions
    );
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
