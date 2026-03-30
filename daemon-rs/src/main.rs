mod auth;
mod co_occurrence;
mod db;
mod handlers;
mod logging;
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
            eprintln!("[cortex] MCP mode not yet implemented");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Usage: cortex <serve|mcp>");
            eprintln!("  serve — HTTP daemon only (standalone)");
            eprintln!("  mcp   — MCP stdio + HTTP daemon (for Claude Code)");
            std::process::exit(1);
        }
    }
}
