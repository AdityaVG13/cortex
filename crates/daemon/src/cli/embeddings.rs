use super::common::{parse_flag_usize, validate_cli_options_or_exit};
use crate::auth;
use crate::state;
use serde_json::json;
pub async fn run_embeddings_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, args: &[String]) {
    let args = super::common::without_global_value_flags(args);
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "status" => {
            validate_cli_options_or_exit(&args[1..], &[], &["--json"]);
            let json_output = args.iter().any(|arg| arg == "--json");
            run_embeddings_status_cli(cx, paths, json_output).await;
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
pub(crate) async fn run_embeddings_status_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state: {err}");
            std::process::exit(1);
        }
    };
    let conn = match state.db.lock(cx).await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("[cortex] {err}");
            std::process::exit(1);
        }
    };
    let count = |sql: &str, table: &str| -> i64 {
        match conn.query_row(sql, [], |row| row.get(0)) {
            Ok(n) => n,
            Err(err) => {
                eprintln!("Error: failed to count {table}: {err}");
                std::process::exit(1);
            }
        }
    };
    let anchors = count("SELECT COUNT(*) FROM clock_anchors", "clock_anchors");
    let links = count("SELECT COUNT(*) FROM clock_links", "clock_links");
    let inert = count("SELECT COUNT(*) FROM embeddings", "embeddings");
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
pub async fn run_rebuild_anchors_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, args: &[String]) {
    run_clock_rebuild_cli(cx, paths, args).await;
}
async fn run_clock_rebuild_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--batch-size"], &["--json"]);
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
    let conn = match state.db.lock(cx).await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("[cortex] {err}");
            std::process::exit(1);
        }
    };
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
