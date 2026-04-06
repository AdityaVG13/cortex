// SPDX-License-Identifier: MIT
//! Hook subcommands -- replaces brain-boot.js and statusline JS.
//!
//! `cortex hook-boot [--agent NAME]`  -- Claude Code SessionStart hook
//! `cortex hook-status`               -- Statusline one-liner output
//!
//! Both are short-lived CLI invocations that HTTP-call the running daemon.
//! No RuntimeState, no DB, no ONNX -- just a thin HTTP client.

use serde_json::json;
use std::path::PathBuf;

const DEFAULT_BUDGET: u32 = 600;

// ---- Internal types ---------------------------------------------------------

struct BootResult {
    boot_prompt: String,
    token_estimate: Option<i64>,
    savings: Option<serde_json::Value>,
}

struct HealthResult {
    memories: i64,
    decisions: i64,
    embeddings: i64,
}

// ---- HTTP helpers -----------------------------------------------------------

/// Read the auth token from ~/.cortex/cortex.token for authenticated requests.
fn read_auth_token() -> Option<String> {
    let path = crate::auth::CortexPaths::resolve().token;
    match std::fs::read_to_string(path) {
        Ok(token) => {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(_) => None,
    }
}

fn daemon_port() -> u16 {
    crate::auth::CortexPaths::resolve().port
}

async fn fetch_boot(agent: &str, budget: u32, port: u16) -> Option<BootResult> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(7))
        .build()
        .ok()?;

    let url = format!(
        "http://127.0.0.1:{port}/boot?agent={}&budget={}",
        agent, budget
    );
    let mut req = client.get(&url).header("x-cortex-request", "true");
    if let Some(token) = read_auth_token() {
        req = req.header("Authorization", format!("Bearer {}", token));
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    Some(BootResult {
        boot_prompt: data.get("bootPrompt")?.as_str()?.to_string(),
        token_estimate: data.get("tokenEstimate").and_then(|v| v.as_i64()),
        savings: data.get("savings").cloned(),
    })
}

async fn fetch_health(port: u16) -> Option<HealthResult> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let resp = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }

    let data: serde_json::Value = resp.json().await.ok()?;
    let stats = data.get("stats")?;
    Some(HealthResult {
        memories: stats.get("memories")?.as_i64()?,
        decisions: stats.get("decisions")?.as_i64()?,
        embeddings: stats
            .get("embeddings")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    })
}

fn status_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("brain-status.json")
}

// ---- Public entry points ----------------------------------------------------

/// SessionStart hook -- outputs JSON for Claude Code hook system.
pub async fn run_boot(agent: &str) {
    let port = daemon_port();
    let (boot, health) = tokio::join!(
        fetch_boot(agent, DEFAULT_BUDGET, port),
        fetch_health(port)
    );

    let (total, memories, decisions) = health
        .as_ref()
        .map(|h| (h.memories + h.decisions, h.memories, h.decisions))
        .unwrap_or((0, 0, 0));

    let cortex_connected = boot.is_some() || health.is_some();
    let cortex_booted = boot.is_some();
    let overall = if cortex_connected {
        "ONLINE"
    } else {
        "DEGRADED"
    };

    let cortex_part = if cortex_booted {
        format!(
            "Cortex \u{2713} ({} nodes: {} mem + {} dec)",
            total, memories, decisions
        )
    } else if cortex_connected {
        "Cortex \u{2713} (boot failed)".to_string()
    } else {
        "Cortex \u{2717} \u{2014} run cortex-start or cortex service start".to_string()
    };

    let oneliner = format!("Brain: {} | {}", overall, cortex_part);

    // Write brain-status.json for statusline
    let status = json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "overall": overall,
        "cortex": {
            "connected": cortex_connected,
            "booted": cortex_booted,
            "total": total,
            "memories": memories,
            "decisions": decisions,
            "tokenEstimate": boot.as_ref().and_then(|b| b.token_estimate),
            "savings": boot.as_ref().and_then(|b| b.savings.clone()),
        },
        "oneliner": oneliner,
    });

    let _ = std::fs::write(
        status_path(),
        serde_json::to_string_pretty(&status).unwrap_or_default(),
    );

    // Build additionalContext for Claude Code
    let mut parts = vec![
        "BRAIN BOOT STATUS \u{2014} You MUST include this line at the very start of your first reply:".to_string(),
        oneliner.clone(),
        "Do not ask the user about brain status. Do not explain this line. Just print it.".to_string(),
    ];

    if let Some(b) = &boot {
        parts.push(String::new());
        parts.push("CORTEX BOOT PROMPT (auto-injected by SessionStart hook):".to_string());
        parts.push(b.boot_prompt.clone());
        parts.push(String::new());
        parts.push(
            "cortex_boot() was already called mechanically by the hook. Do NOT call it again."
                .to_string(),
        );
        parts.push(
            "You still use cortex_recall, cortex_store, and cortex_diary as normal MCP tools."
                .to_string(),
        );
    } else if cortex_connected {
        parts.push(String::new());
        parts.push(
            "WARNING: Cortex is running but boot failed. Call cortex_boot() manually as fallback."
                .to_string(),
        );
    } else {
        parts.push(String::new());
        parts.push(
            "WARNING: Cortex daemon is not running. Advise user to run cortex-start.".to_string(),
        );
        parts.push("cortex_boot() will fail until the daemon is started.".to_string());
    }

    // Output hook JSON to stdout
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "SessionStart",
            "additionalContext": parts.join("\n"),
        }
    });

    println!("{}", serde_json::to_string(&output).unwrap_or_default());
}

/// Statusline output -- prints a one-liner to stdout.
pub async fn run_status() {
    match fetch_health(daemon_port()).await {
        Some(h) => {
            println!(
                "ONLINE | {} mem | {} dec | {} emb",
                h.memories, h.decisions, h.embeddings
            );
        }
        None => {
            println!("OFFLINE");
        }
    }
}

