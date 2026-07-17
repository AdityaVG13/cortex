use super::common::{parse_env_usize, parse_flag_usize};
use super::daemon::{
    background_db_lock_max_wait, build_embeddings_async, count_unembedded_targets_for_model, DEFAULT_EMBED_BACKFILL_BATCH_SIZE,
    DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS,
};
use crate::auth;
use crate::state;
use serde_json::json;
use std::time::Duration;
pub(crate) async fn run_embeddings_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.first().map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "status" => {
            let json_output = args.iter().any(|arg| arg == "--json");
            run_embeddings_status_cli(paths, json_output).await;
        }
        "drain" => {
            run_embeddings_drain_cli(paths, &args[1..]).await;
        }
        _ => {
            eprintln!(
"Usage: cortex embeddings <status|drain> [--json] [--batch-size <n>] [--max-batches <n>] [--lock-wait-ms <n>] [--until-exhausted] [--max-iterations <n>]"
);
            std::process::exit(1);
        }
    }
}
pub(crate) async fn run_embeddings_status_cli(paths: &auth::CortexPaths, json_output: bool) {
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state for embeddings status: {err}");
            std::process::exit(1);
        }
    };
    let Some(engine) = state.embedding_engine.clone() else {
        eprintln!("[embeddings] No embedding model is currently loaded. Run `cortex serve` once to trigger model download, then retry.");
        std::process::exit(1);
    };
    let model_key = engine.model_key().to_string();
    let (backlog_memories, backlog_decisions) = {
        let conn = state.db.lock().await;
        count_unembedded_targets_for_model(&conn, &model_key)
    };
    let backlog_total = backlog_memories + backlog_decisions;
    if json_output {
        println!(
            "{}",
            json!({"model":model_key,"backlog":{"memories":backlog_memories,"decisions":backlog_decisions,"total":backlog_total}
            })
        );
    } else {
        println!("Embeddings status");
        println!("model: {model_key}");
        println!("backlog: memories={}, decisions={}, total={}", backlog_memories, backlog_decisions, backlog_total);
    }
}
pub(crate) async fn run_embeddings_drain_cli(paths: &auth::CortexPaths, args: &[String]) {
    let batch_size = match parse_flag_usize(args, "--batch-size") {
        Ok(Some(value)) => value.clamp(1, 10_000),
        Ok(None) => parse_env_usize("CORTEX_EMBED_BACKFILL_BATCH_SIZE", DEFAULT_EMBED_BACKFILL_BATCH_SIZE).clamp(1, 10_000),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let max_batches_per_pass = match parse_flag_usize(args, "--max-batches") {
        Ok(Some(value)) => value.clamp(1, 10_000),
        Ok(None) => parse_env_usize("CORTEX_EMBED_BACKFILL_MAX_BATCHES_PER_PASS", DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS).clamp(1, 10_000),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let lock_wait_ms = match parse_flag_usize(args, "--lock-wait-ms") {
        Ok(Some(value)) => value.clamp(100, 60_000),
        Ok(None) => (background_db_lock_max_wait().as_millis() as usize).clamp(100, 60_000),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let max_iterations = match parse_flag_usize(args, "--max-iterations") {
        Ok(Some(value)) => value.clamp(1, 1024),
        Ok(None) => 32,
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    let until_exhausted = args.iter().any(|arg| arg == "--until-exhausted");
    let json_output = args.iter().any(|arg| arg == "--json");
    let lock_wait = Duration::from_millis(lock_wait_ms as u64);
    let (state, _shutdown_rx) = match state::initialize(paths, false) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("Error: failed to initialize state for embeddings drain: {err}");
            std::process::exit(1);
        }
    };
    let Some(engine) = state.embedding_engine.clone() else {
        eprintln!("[embeddings] No embedding model is currently loaded. Run `cortex serve` once to trigger model download, then retry.");
        std::process::exit(1);
    };
    let model_key = engine.model_key().to_string();
    let mut iterations_ran = 0usize;
    let mut queued_total = 0usize;
    let mut computed_total = 0usize;
    let mut passes_ran = 0usize;
    let mut exhausted = false;
    while iterations_ran < max_iterations {
        iterations_ran += 1;
        let pass = build_embeddings_async(engine.clone(), &state.db, batch_size, max_batches_per_pass, lock_wait).await;
        queued_total += pass.queued_total;
        computed_total += pass.computed_total;
        passes_ran += pass.passes_ran;
        exhausted = pass.exhausted;
        if pass.exhausted || pass.queued_total == 0 || !until_exhausted {
            break;
        }
    }
    let (remaining_memories, remaining_decisions) = {
        let conn = state.db.lock().await;
        count_unembedded_targets_for_model(&conn, &model_key)
    };
    let remaining_total = remaining_memories + remaining_decisions;
    exhausted = exhausted || remaining_total == 0;
    if json_output {
        println!(
            "{}",
            json!({"model":model_key,"batch_size":batch_size,"max_batches_per_pass"
:max_batches_per_pass,"lock_wait_ms":lock_wait_ms,"until_exhausted":until_exhausted,"max_iterations":max_iterations,
"iterations_ran":iterations_ran,"queued_total":queued_total,"computed_total":computed_total,"passes_ran":passes_ran,"remaining":{
"memories":remaining_memories,"decisions":remaining_decisions,"total":remaining_total},"exhausted":exhausted})
        );
    } else {
        println!("Embeddings drain");
        println!("model: {model_key}");
        println!("drain: queued={}, built={}, passes={}, iterations={}", queued_total, computed_total, passes_ran, iterations_ran);
        println!("remaining: memories={}, decisions={}, total={}", remaining_memories, remaining_decisions, remaining_total);
        println!("exhausted: {exhausted}");
    }
    if until_exhausted && !exhausted {
        eprintln!("[embeddings] backlog still pending after {} iteration(s); rerun with higher --max-iterations or --max-batches", iterations_ran);
        std::process::exit(2);
    }
}
