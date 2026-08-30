use super::configure::{step_configure, summarize_configs};
use super::detect::step_detect;
use super::helpers::{daemon_base_url, daemon_port, daemon_url, print_step, stable_mcp_binary_path};
use super::types::StepResult;
use crate::auth;
use std::fs;
pub async fn run_setup() {
    eprintln!();
    eprintln!("  Cortex Setup -- Universal AI Memory");
    eprintln!("  ====================================");
    eprintln!();
    let init_result = step_init().await;
    print_step(1, "Initialize", &init_result);
    let cortex_exe = stable_mcp_binary_path();
    let detected = step_detect();
    print_step(
        2,
        "Detect AI tools",
        &if detected.is_empty() {
            StepResult::Warn("No AI tools detected. You can configure them manually later.".into())
        } else {
            let names: Vec<&str> = detected.iter().map(|t| t.name).collect();
            StepResult::Ok(format!("Found: {}", names.join(", ")))
        },
    );
    let config_results = step_configure(&detected, &cortex_exe);
    print_step(3, "Configure AI tools", &summarize_configs(&config_results));
    for (tool_name, result) in &config_results {
        eprintln!("       {} {}: {}", result.icon(), tool_name, result.message());
    }
    let daemon_result = step_daemon().await;
    print_step(4, "Daemon availability", &daemon_result);
    let verify_result = step_verify().await;
    print_step(5, "Verify", &verify_result);
    eprintln!();
    let token = auth::read_token().unwrap_or_else(|| "???".into());
    let token_preview = if token.len() > 8 { &token[..8] } else { &token };
    eprintln!("  Your API token: {}... (full token in ~/.cortex/cortex.token)", token_preview);
    eprintln!("  Daemon:         {}", daemon_base_url());
    eprintln!("  Health check:   curl {}", daemon_url("/health"));
    eprintln!("  Readiness:      curl {}", daemon_url("/readiness"));
    eprintln!();
    eprintln!("  Cortex is configured. Start it from Control Center or let your client run `cortex mcp --agent <name>` when you want a live daemon.");
    eprintln!();
}
async fn step_init() -> StepResult {
    let cortex_dir = auth::cortex_dir();
    let mut notes = Vec::new();
    if let Err(e) = fs::create_dir_all(&cortex_dir) {
        return StepResult::Fail(format!("Cannot create {}: {e}", cortex_dir.display()));
    }
    notes.push(format!("Directory: {}", cortex_dir.display()));
    if auth::read_token().is_some() {
        notes.push("Token: exists (reusing)".into());
    } else {
        if let Err(e) = auth::try_generate_token() {
            return StepResult::Fail(format!("Token: {e}"));
        }
        notes.push("Token: generated".into());
    }
    notes.push("Retrieval engine: clock-quorum (model-free)".into());
    StepResult::Ok(notes.join(" | "))
}
async fn step_daemon() -> StepResult {
    let port = daemon_port();
    if is_daemon_healthy().await {
        return StepResult::Ok(format!("Daemon already running on :{port}"));
    }
    StepResult::Warn(format!("No daemon is running on :{port}. Start Cortex from Control Center or let your client launch `cortex mcp --agent <name>`."))
}
async fn is_daemon_healthy() -> bool {
    let paths = auth::CortexPaths::resolve();
    crate::daemon_lifecycle::daemon_healthy(&paths).await
}
async fn step_verify() -> StepResult {
    if !is_daemon_healthy().await {
        return StepResult::Warn(
"Skipped live verification because no daemon is currently running. Start Cortex from Control Center or `cortex mcp --agent <name>`, then rerun setup if you want a round-trip check."
.into(),);
    }
    let token = match auth::read_token() {
        Some(t) => t,
        None => return StepResult::Fail("No auth token found".into()),
    };
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return StepResult::Fail(format!("HTTP client error: {e}")),
    };
    let store_resp = client
        .post(daemon_url("/store"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Cortex-Request", "true")
        .json(&serde_json::json!({"decision"
:"Cortex installed and verified","context":"Automated setup verification","type":"memory","source_agent":"cortex-setup"}))
        .send()
        .await;
    match store_resp {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            return StepResult::Warn(format!("Store returned {}: daemon is running but store failed", r.status()));
        }
        Err(e) => return StepResult::Fail(format!("Cannot reach daemon: {e}")),
    }
    let recall_resp = client
        .get(daemon_url("/recall"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Cortex-Request", "true")
        .query(&[("q", "Cortex installed"), ("k", "1"), ("budget", "100")])
        .send()
        .await;
    match recall_resp {
        Ok(r) if r.status().is_success() => StepResult::Ok("Store + recall round-trip verified".into()),
        Ok(r) => StepResult::Warn(format!("Recall returned {}: store worked but recall did not", r.status())),
        Err(e) => StepResult::Warn(format!("Recall failed: {e}. Store succeeded.")),
    }
}
