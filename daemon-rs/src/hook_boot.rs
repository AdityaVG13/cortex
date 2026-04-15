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

async fn fetch_boot(
    agent: &str,
    budget: u32,
    paths: &crate::auth::CortexPaths,
) -> Option<BootResult> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(7))
        .build()
        .ok()?;

    let base_url = crate::transport::local_http_base_url(paths);
    let mut url = reqwest::Url::parse(&format!("{}/boot", base_url.trim_end_matches('/'))).ok()?;
    url.query_pairs_mut()
        .append_pair("agent", agent)
        .append_pair("budget", &budget.to_string());

    let mut headers = vec![("x-cortex-request".to_string(), "true".to_string())];
    if let Some(token) = read_auth_token() {
        headers.push(("authorization".to_string(), format!("Bearer {token}")));
    }

    let (status, body) = crate::transport::request_url_with_local_ipc_fallback(
        &client,
        "GET",
        url.as_ref(),
        paths,
        &headers,
        None,
        std::time::Duration::from_secs(7),
    )
    .await
    .ok()?;
    if !status.is_success() {
        return None;
    }

    let data: serde_json::Value = serde_json::from_str(&body).ok()?;
    Some(BootResult {
        boot_prompt: data.get("bootPrompt")?.as_str()?.to_string(),
        token_estimate: data.get("tokenEstimate").and_then(|v| v.as_i64()),
        savings: data.get("savings").cloned(),
    })
}

async fn fetch_health(paths: &crate::auth::CortexPaths) -> Option<HealthResult> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok()?;

    let base_url = crate::transport::local_http_base_url(paths);
    let readiness_url = format!("{base_url}/readiness");
    let (readiness_status, readiness_body) = crate::transport::request_url_with_local_ipc_fallback(
        &client,
        "GET",
        &readiness_url,
        paths,
        &[],
        None,
        std::time::Duration::from_secs(2),
    )
    .await
    .ok()?;
    match crate::daemon_lifecycle::readiness_state_from_payload(
        readiness_status.as_u16(),
        &readiness_body,
        Some(paths.port),
        None,
    ) {
        Some(true) => {}
        Some(false) | None => return None,
    }

    let health_url = format!("{base_url}/health");
    let (status, body) = crate::transport::request_url_with_local_ipc_fallback(
        &client,
        "GET",
        &health_url,
        paths,
        &[],
        None,
        std::time::Duration::from_secs(2),
    )
    .await
    .ok()?;
    if !status.is_success() {
        return None;
    }
    let data: serde_json::Value = serde_json::from_str(&body).ok()?;
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

/// Register an active session with the daemon so the Agents panel shows it.
async fn register_session(agent: &str, paths: &crate::auth::CortexPaths) {
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    let base_url = crate::transport::local_http_base_url(paths);
    let body = json!({
        "agent": agent,
        "ttl": 7200,
        "description": "Active coding session"
    })
    .to_string();
    let mut headers = vec![
        ("content-type".to_string(), "application/json".to_string()),
        ("x-cortex-request".to_string(), "true".to_string()),
    ];
    if let Some(token) = read_auth_token() {
        headers.push(("authorization".to_string(), format!("Bearer {token}")));
    }
    let _ = crate::transport::request_with_local_ipc_fallback(
        &client,
        "POST",
        &base_url,
        "/session/start",
        paths,
        &headers,
        Some(&body),
        std::time::Duration::from_secs(3),
    )
    .await;
}

// ---- Public entry points ----------------------------------------------------

/// SessionStart hook -- outputs JSON for Claude Code hook system.
pub async fn run_boot(agent: &str) {
    let paths = crate::auth::CortexPaths::resolve();
    let (boot, health) = tokio::join!(
        fetch_boot(agent, DEFAULT_BUDGET, &paths),
        fetch_health(&paths)
    );

    let (total, memories, decisions) = health
        .as_ref()
        .map(|h| (h.memories + h.decisions, h.memories, h.decisions))
        .unwrap_or((0, 0, 0));

    let cortex_connected = boot.is_some() || health.is_some();
    let cortex_booted = boot.is_some();

    // Register session so the Agents panel shows this agent as online
    if cortex_booted {
        register_session(agent, &paths).await;
    }
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
    let paths = crate::auth::CortexPaths::resolve();
    match fetch_health(&paths).await {
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
