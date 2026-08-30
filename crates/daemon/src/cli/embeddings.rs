use super::common::parse_flag_usize;
use crate::auth;
use crate::state;
use serde_json::json;
pub async fn run_embeddings_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "status" => {
            let json_output = args.iter().any(|arg| arg == "--json");
            run_embeddings_status_cli(paths, json_output).await;
        }
        "drain" | "rebuild" | "re-embed" | "reembed" => {
            eprintln!("Clock-Quorum Recall is model-free. Embedding backfill commands are removed.");
            eprintln!("Use `cortex rebuild-anchors --json` to rebuild clock projections.");
            eprintln!("Existing embedding rows stay inert and unread.");
            std::process::exit(1);
        }
        _ => {
            eprintln!("Usage: cortex embeddings status [--json]");
            eprintln!("Clock-Quorum Recall is model-free. Use `cortex rebuild-anchors` to rebuild clock projections.");
            std::process::exit(1);
        }
    }
}
pub(crate) async fn run_embeddings_status_cli(paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state: {err}");
            std::process::exit(1);
        }
    };
    let conn = state.db.lock().await;
    let anchors: i64 = conn.query_row("SELECT COUNT(*) FROM clock_anchors", [], |row| row.get(0)).unwrap_or(0);
    let links: i64 = conn.query_row("SELECT COUNT(*) FROM clock_links", [], |row| row.get(0)).unwrap_or(0);
    let inert: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0)).unwrap_or(0);
    if json_output {
        println!("{}", json!({"engine":"clock-quorum","modelFree":true,"anchors":anchors,"links":links,"inertEmbeddings":inert}));
    } else {
        println!("Clock-Quorum Recall");
        println!("engine: clock-quorum (model-free)");
        println!("anchors: {anchors}");
        println!("links: {links}");
        println!("inert embedding rows: {inert} (preserved, unread)");
    }
}
pub async fn run_rebuild_anchors_cli(paths: &auth::CortexPaths, args: &[String]) {
    run_clock_rebuild_cli(paths, args).await;
}
async fn run_clock_rebuild_cli(paths: &auth::CortexPaths, args: &[String]) {
    let json_output = args.iter().any(|arg| arg == "--json");
    let batch = match parse_flag_usize(args, "--batch-size") {
        Ok(Some(value)) => value.clamp(16, 10_000),
        Ok(None) => 256,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state: {err}");
            std::process::exit(1);
        }
    };
    let conn = state.db.lock().await;
    match crate::clockwork::rebuild_clock_projections(&conn, batch) {
        Ok(projected) => {
            if json_output {
                println!("{}", json!({"rebuilt":true,"projected":projected,"engine":"clock-quorum"}));
            } else {
                println!("Clock projections rebuilt: {projected} targets");
            }
        }
        Err(err) => {
            eprintln!("Error: clock rebuild failed: {err}");
            std::process::exit(1);
        }
    }
}
