use std::env;

mod co_occurrence;
mod db;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");

    match mode {
        "serve" => {
            eprintln!("[cortex] Starting Cortex v2.1.0 (Rust daemon)...");
            eprintln!("[cortex] HTTP serve mode — not yet implemented");
        }
        "mcp" => {
            eprintln!("[cortex] Starting Cortex v2.1.0 (Rust daemon)...");
            eprintln!("[cortex] MCP + HTTP mode — not yet implemented");
        }
        _ => {
            eprintln!("Usage: cortex <serve|mcp>");
            eprintln!("  serve — HTTP daemon only (standalone)");
            eprintln!("  mcp   — MCP stdio + HTTP daemon (for Claude Code)");
            std::process::exit(1);
        }
    }
}
