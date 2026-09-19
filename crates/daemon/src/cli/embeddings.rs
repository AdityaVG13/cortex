use super::common::{die, or_die, parse_flag_usize, validate_cli_options_or_exit};
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
        "drain" | "rebuild" | "re-embed" | "reembed" => die(
            "Clock-Quorum Recall is model-free. Embedding backfill commands are removed.\nUse `cortex rebuild-anchors --json` to rebuild clock projections.\nExisting embedding rows stay inert and unread.",
        ),
        _ => die("Usage: cortex embeddings status [--json]\nClock-Quorum Recall is model-free. Use `cortex rebuild-anchors` to rebuild clock projections."),
    }
}
pub(crate) async fn run_embeddings_status_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = or_die(state::initialize(paths, false), "Error: failed to initialize state: ");
    let conn = or_die(state.db.lock(cx).await, "[cortex] ");
    let count = |sql: &str, table: &str| -> i64 { or_die(conn.query_row(sql, [], |row| row.get(0)), &format!("Error: failed to count {table}: ")) };
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
        Err(err) => die(format!("Error: {err}")),
    };
    let (state, _shutdown_rx) = or_die(state::initialize(paths, false), "Error: failed to initialize state: ");
    let conn = or_die(state.db.lock(cx).await, "[cortex] ");
    let projected = or_die(crate::clockwork::rebuild_clock_projections(&conn, batch), "Error: clock rebuild failed: ");
    if json_output {
        println!("{}", json!({"rebuilt":true,"projected":projected,"engine":"clock-quorum"}));
    } else {
        println!("Clock projections rebuilt: {projected} targets");
    }
}
