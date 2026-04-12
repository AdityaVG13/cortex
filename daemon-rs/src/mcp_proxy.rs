// SPDX-License-Identifier: MIT
//! MCP thin proxy -- forwards JSON-RPC from stdin to the HTTP daemon.
//!
//! Instead of loading its own RuntimeState (DB, ONNX engine, caches), this
//! proxy forwards all MCP messages to the running daemon via POST /mcp-rpc.
//!
//! Architecture win:
//! - One ONNX MiniLM engine (in the daemon), shared across ALL clients
//! - One set of caches and counters (served_content, co-occurrence)
//! - Zero extra memory per MCP session (~2MB proxy vs ~70MB standalone)
//! - All agents (Claude, Cursor, Gemini, Codex) hit the same state
//!
//! Resilience:
//! - Plugin mode requires daemon connectivity (no standalone fallback)
//! - Team mode uses explicit API key injection from CLI args
//! - Owner-managed sessions can respawn and stop the daemon they created

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::auth::CortexPaths;
use crate::daemon_lifecycle;

const HEALTH_CHECK_ATTEMPTS: u32 = 5;
const REQUEST_ATTEMPTS: u32 = 3;
const MAX_CONSECUTIVE_FAILURES: u32 = 2;
const RESPAWN_COOLDOWN_SECS: u64 = 15;
const SESSION_HEARTBEAT_SECS: u64 = 15;
const SESSION_RESTART_ATTEMPTS: u32 = 4;
const SESSION_RESTART_DELAY_MS: u64 = 250;
const HEARTBEAT_RECOVERY_FAILURES: u32 = 2;

#[derive(Clone, Copy, Debug, Default)]
pub struct ProxyRuntimeOptions {
    pub allow_respawn: bool,
    pub shutdown_on_exit: bool,
}

/// Read the auth token from ~/.cortex/cortex.token.
pub(crate) fn read_auth_token() -> Option<String> {
    let token_path = crate::auth::CortexPaths::resolve().token;
    match std::fs::read_to_string(&token_path) {
        Ok(token) => {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                eprintln!(
                    "[cortex-mcp] Auth token file is empty: {}",
                    token_path.display()
                );
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!(
                "[cortex-mcp] Failed to read auth token {}: {e}",
                token_path.display()
            );
            None
        }
    }
}

/// Detect team mode without a full DB open.
/// Team mode is explicit from CLI options.
fn detect_team_mode(api_key: Option<&str>) -> bool {
    api_key.is_some()
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_agent_identity(agent_arg: Option<&str>) -> (String, Option<String>) {
    let model = env_trimmed("CORTEX_AGENT_MODEL").or_else(|| env_trimmed("CORTEX_MODEL"));

    let mut agent = env_trimmed("CORTEX_AGENT_DISPLAY")
        .or_else(|| {
            agent_arg
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| env_trimmed("CORTEX_AGENT_NAME"))
        .unwrap_or_else(|| "mcp".to_string());

    if !agent.contains('(') {
        if let Some(model_name) = model.as_deref() {
            if agent.eq_ignore_ascii_case("droid") {
                agent = format!("DROID ({model_name})");
            } else {
                agent = format!("{agent} ({model_name})");
            }
        }
    }

    (agent, model)
}

fn build_auth_header(api_key: Option<&str>) -> Option<String> {
    if let Some(key) = api_key {
        return Some(format!("Bearer {key}"));
    }
    read_auth_token().map(|token| format!("Bearer {token}"))
}

fn is_cortex_health_response(status: reqwest::StatusCode, body: &str) -> bool {
    if !status.is_success() {
        return false;
    }

    let Ok(json) = serde_json::from_str::<Value>(body.trim()) else {
        return false;
    };

    let health_status = json.get("status").and_then(|value| value.as_str());
    let runtime = json.get("runtime").and_then(|value| value.as_object());
    let stats = json.get("stats").and_then(|value| value.as_object());

    matches!(health_status, Some("ok" | "degraded")) && runtime.is_some() && stats.is_some()
}

async fn health_check_ready(client: &reqwest::Client, health_url: &str) -> bool {
    let response = match client.get(health_url).send().await {
        Ok(response) => response,
        Err(_) => return false,
    };

    let status = response.status();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return false,
    };

    is_cortex_health_response(status, &body)
}

fn is_auth_recovery_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
}

async fn recover_solo_auth(
    client: &reqwest::Client,
    health_url: &str,
    base_url: &str,
    agent: &str,
    model: Option<&str>,
) -> bool {
    if !health_check_ready(client, health_url).await {
        return false;
    }

    if !session_start_with_retry(
        client,
        base_url,
        None,
        agent,
        model,
        SESSION_RESTART_ATTEMPTS,
        SESSION_RESTART_DELAY_MS,
    )
    .await
    {
        eprintln!("[cortex-mcp] Auth recovered but session re-registration did not succeed yet");
        return false;
    }

    true
}

async fn session_start_with_retry(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    agent: &str,
    model: Option<&str>,
    attempts: u32,
    delay_ms: u64,
) -> bool {
    for attempt in 1..=attempts.max(1) {
        if session_start(client, base_url, api_key, agent, model).await {
            return true;
        }

        if attempt < attempts {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms * attempt as u64)).await;
        }
    }

    false
}

fn persist_write_buffer(
    buffer_path: &std::path::Path,
    remaining: &[String],
) -> Result<(), std::io::Error> {
    use std::io::Write;

    let mut file = std::fs::File::create(buffer_path)?;
    for line in remaining {
        writeln!(file, "{line}")?;
    }
    Ok(())
}

async fn drain_write_buffer(
    client: &reqwest::Client,
    rpc_url: &str,
    api_key: Option<&str>,
    agent: &str,
    model: Option<&str>,
    paths: &CortexPaths,
) {
    let buffer_path = &paths.write_buffer;
    let content = match std::fs::read_to_string(buffer_path) {
        Ok(content) if !content.trim().is_empty() => content,
        _ => return,
    };

    let lines: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect();
    if lines.is_empty() {
        return;
    }

    let mut remaining = Vec::new();
    let mut drained = 0usize;

    for line in lines {
        let mut req = client
            .post(rpc_url)
            .header("content-type", "application/json")
            .header("x-cortex-request", "true")
            .header("x-source-agent", agent);
        if let Some(model_name) = model {
            req = req.header("x-source-model", model_name);
        }
        if let Some(auth) = build_auth_header(api_key) {
            req = req.header("authorization", auth);
        }

        match req.body(line.clone()).send().await {
            Ok(resp) if resp.status().is_success() => {
                drained += 1;
            }
            _ => remaining.push(line),
        }
    }

    if let Err(e) = persist_write_buffer(buffer_path, &remaining) {
        eprintln!(
            "[cortex-mcp] Failed to compact write buffer {}: {e}",
            buffer_path.display()
        );
        return;
    }

    if drained > 0 {
        eprintln!(
            "[cortex-mcp] Drained {drained} buffered writes and compacted {}",
            buffer_path.display()
        );
    }
}

async fn session_start(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    agent: &str,
    model: Option<&str>,
) -> bool {
    let mut req = client
        .post(format!("{base_url}/session/start"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .json(&serde_json::json!({
            "agent": agent,
            "ttl": 7200,
            "description": model
                .map(|m| format!("MCP session · {m}"))
                .unwrap_or_else(|| "MCP session".to_string())
        }));

    if let Some(auth) = build_auth_header(api_key) {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

enum SessionHeartbeatOutcome {
    Renewed,
    MissingSession,
    Failed,
}

async fn session_heartbeat(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    agent: &str,
    model: Option<&str>,
) -> SessionHeartbeatOutcome {
    let mut req = client
        .post(format!("{base_url}/session/heartbeat"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .json(&serde_json::json!({
            "agent": agent,
            "description": model.map(|m| format!("MCP session · {m}")).unwrap_or_else(|| "MCP session".to_string())
        }));

    if let Some(auth) = build_auth_header(api_key) {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => SessionHeartbeatOutcome::Renewed,
        Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
            SessionHeartbeatOutcome::MissingSession
        }
        Ok(_) => SessionHeartbeatOutcome::Failed,
        Err(_) => SessionHeartbeatOutcome::Failed,
    }
}

async fn session_end(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    agent: &str,
) -> bool {
    let mut req = client
        .post(format!("{base_url}/session/end"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .json(&serde_json::json!({ "agent": agent }));

    if let Some(auth) = build_auth_header(api_key) {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn shutdown_daemon(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> bool {
    let mut req = client
        .post(format!("{base_url}/shutdown"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .body("{}");

    if let Some(auth) = build_auth_header(api_key) {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn finalize_proxy_session(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    agent: &str,
    options: ProxyRuntimeOptions,
) {
    let _ = session_end(client, base_url, api_key, agent).await;
    if options.shutdown_on_exit {
        if shutdown_daemon(client, base_url, api_key).await {
            eprintln!("[cortex-mcp] Stopped owned daemon for '{agent}'");
        } else {
            eprintln!("[cortex-mcp] Warning: failed to stop owned daemon for '{agent}'");
        }
    }
}

/// Run MCP proxy over stdio -> HTTP.
pub async fn run(
    base_url: &str,
    api_key: Option<&str>,
    agent: Option<&str>,
    options: ProxyRuntimeOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let base_url = base_url.trim_end_matches('/');
    let mut rpc_base_url = base_url.to_string();
    let mut rpc_url = format!("{rpc_base_url}/mcp-rpc");
    let health_url = format!("{rpc_base_url}/health");
    let (agent_display, agent_model) = resolve_agent_identity(agent);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()?;

    let team_mode = detect_team_mode(api_key);
    if team_mode {
        eprintln!("[cortex-mcp] Team mode proxy -> {base_url} as '{agent_display}'");
    } else {
        eprintln!("[cortex-mcp] Solo mode proxy -> {base_url} as '{agent_display}'");
    }

    // Health check with retry (daemon may still be starting)
    let mut healthy = false;
    for attempt in 1..=HEALTH_CHECK_ATTEMPTS {
        match client.get(&health_url).send().await {
            Ok(resp) => {
                let status = resp.status();
                match resp.text().await {
                    Ok(body) if is_cortex_health_response(status, &body) => {
                        healthy = true;
                        break;
                    }
                    Ok(_) => {
                        eprintln!(
                            "[cortex-mcp] Health check attempt {attempt}/{HEALTH_CHECK_ATTEMPTS}: HTTP {status} was not a valid Cortex health payload"
                        );
                    }
                    Err(e) => {
                        eprintln!(
                            "[cortex-mcp] Health check attempt {attempt}/{HEALTH_CHECK_ATTEMPTS}: failed reading body: {e}"
                        );
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[cortex-mcp] Health check attempt {attempt}/{HEALTH_CHECK_ATTEMPTS}: {e}"
                );
            }
        }
        if attempt < HEALTH_CHECK_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
        }
    }
    if !healthy {
        eprintln!(
            "[cortex-mcp] Health check failed after {HEALTH_CHECK_ATTEMPTS} attempts; keeping proxy alive and deferring errors to JSON-RPC responses"
        );
    } else {
        let paths = CortexPaths::resolve();
        drain_write_buffer(
            &client,
            &rpc_url,
            api_key,
            &agent_display,
            agent_model.as_deref(),
            &paths,
        )
        .await;
    }

    let _ = session_start_with_retry(
        &client,
        &rpc_base_url,
        api_key,
        &agent_display,
        agent_model.as_deref(),
        SESSION_RESTART_ATTEMPTS,
        SESSION_RESTART_DELAY_MS,
    )
    .await;

    // Spawn background heartbeat to keep sessions visible and recover after daemon restarts.
    {
        let heartbeat_base_url = rpc_base_url.clone();
        let heartbeat_agent = agent_display.clone();
        let heartbeat_model = agent_model.clone();
        let heartbeat_api_key = api_key.map(String::from);
        tokio::spawn(async move {
            let hb_client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
            {
                Ok(client) => client,
                Err(e) => {
                    eprintln!("[cortex-mcp] Heartbeat client init failed: {e}");
                    return;
                }
            };
            let heartbeat_health_url = format!("{heartbeat_base_url}/health");
            let mut consecutive_heartbeat_failures = 0u32;

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(SESSION_HEARTBEAT_SECS)).await;
                match session_heartbeat(
                    &hb_client,
                    &heartbeat_base_url,
                    heartbeat_api_key.as_deref(),
                    &heartbeat_agent,
                    heartbeat_model.as_deref(),
                )
                .await
                {
                    SessionHeartbeatOutcome::Renewed => {
                        consecutive_heartbeat_failures = 0;
                    }
                    SessionHeartbeatOutcome::MissingSession => {
                        consecutive_heartbeat_failures = 0;
                        let restarted = session_start_with_retry(
                            &hb_client,
                            &heartbeat_base_url,
                            heartbeat_api_key.as_deref(),
                            &heartbeat_agent,
                            heartbeat_model.as_deref(),
                            SESSION_RESTART_ATTEMPTS,
                            SESSION_RESTART_DELAY_MS,
                        )
                        .await;
                        if restarted {
                            eprintln!("[cortex-mcp] Re-registered session for {heartbeat_agent}");
                        }
                    }
                    SessionHeartbeatOutcome::Failed => {
                        consecutive_heartbeat_failures += 1;
                        if consecutive_heartbeat_failures < HEARTBEAT_RECOVERY_FAILURES {
                            continue;
                        }

                        consecutive_heartbeat_failures = 0;
                        if !health_check_ready(&hb_client, &heartbeat_health_url).await {
                            continue;
                        }

                        let restarted = session_start_with_retry(
                            &hb_client,
                            &heartbeat_base_url,
                            heartbeat_api_key.as_deref(),
                            &heartbeat_agent,
                            heartbeat_model.as_deref(),
                            SESSION_RESTART_ATTEMPTS,
                            SESSION_RESTART_DELAY_MS,
                        )
                        .await;
                        if restarted {
                            eprintln!("[cortex-mcp] Recovered heartbeat session for {heartbeat_agent}");
                        }
                    }
                }
            }
        });
    }

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();

    let mut consecutive_failures: u32 = 0;
    let mut respawn_attempts: u32 = 0;
    let mut last_respawn_attempt_at: Option<std::time::Instant> = None;

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, options)
                    .await;
                eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                return Ok(());
            }
            Err(e) => {
                finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, options)
                    .await;
                eprintln!("[cortex-mcp] Stdin read error: {e}");
                return Err(e.into());
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[cortex-mcp] Parse error: {e}");
                let err = serde_json::json!({
                    "jsonrpc": "2.0",
                    "error": { "code": -32700, "message": "Parse error" },
                    "id": null
                });
                if !write_value(&mut stdout, &err).await? {
                    finalize_proxy_session(
                        &client,
                        &rpc_base_url,
                        api_key,
                        &agent_display,
                        options,
                    )
                    .await;
                    eprintln!("[cortex-mcp] Stdout closed while returning parse error");
                    return Ok(());
                }
                continue;
            }
        };

        let has_id = msg.get("id").is_some();

        // Retry loop for daemon requests
        let mut last_err = String::new();
        let mut response_body: Option<String> = None;
        let mut should_count_failure = false;
        let request_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut attempted_auth_recovery = false;

        for attempt in 1..=REQUEST_ATTEMPTS {
            let now = tokio::time::Instant::now();
            let remaining = request_deadline.saturating_duration_since(now);
            if remaining.is_zero() {
                last_err = "request deadline exceeded".to_string();
                should_count_failure = true;
                break;
            }

            let auth_header = if let Some(key) = api_key {
                Some(format!("Bearer {key}"))
            } else {
                read_auth_token().map(|token| format!("Bearer {token}"))
            };

            let mut req = client
                .post(&rpc_url)
                .header("content-type", "application/json")
                .header("x-cortex-request", "true")
                .header("x-source-agent", &agent_display)
                .timeout(remaining.min(std::time::Duration::from_secs(10)));
            if let Some(model) = agent_model.as_deref() {
                req = req.header("x-source-model", model);
            }
            if let Some(auth) = auth_header {
                req = req.header("authorization", auth);
            }

            match req.body(trimmed.to_string()).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = match resp.text().await {
                        Ok(body) => body,
                        Err(e) => {
                            last_err = format!("failed to read daemon response body: {e}");
                            should_count_failure = true;
                            if attempt < REQUEST_ATTEMPTS {
                                eprintln!(
                                    "[cortex-mcp] Response read failed (attempt {attempt}/{REQUEST_ATTEMPTS}): {e}"
                                );
                                tokio::time::sleep(std::time::Duration::from_millis(
                                    500 * attempt as u64,
                                ))
                                .await;
                                continue;
                            }
                            break;
                        }
                    };

                    if api_key.is_none() && is_auth_recovery_status(status) {
                        last_err = if body.trim().is_empty() {
                            format!("daemon returned auth HTTP {status}")
                        } else {
                            format!("daemon returned auth HTTP {status}: {}", body.trim())
                        };

                        if attempt < REQUEST_ATTEMPTS {
                            if !attempted_auth_recovery {
                                attempted_auth_recovery = true;
                                let recovered = recover_solo_auth(
                                    &client,
                                    &health_url,
                                    &rpc_base_url,
                                    &agent_display,
                                    agent_model.as_deref(),
                                )
                                .await;
                                if recovered {
                                    eprintln!(
                                        "[cortex-mcp] Auth rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); refreshed token and retrying"
                                    );
                                } else {
                                    eprintln!(
                                        "[cortex-mcp] Auth rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); daemon looks live but auth recovery is still settling"
                                    );
                                }
                            } else {
                                eprintln!(
                                    "[cortex-mcp] Auth still rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); retrying once more before surfacing the error"
                                );
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(
                                150 * attempt as u64,
                            ))
                            .await;
                            continue;
                        }
                    }

                    if is_retryable_status(status) {
                        last_err = if body.trim().is_empty() {
                            format!("daemon returned transient HTTP {status}")
                        } else {
                            format!("daemon returned transient HTTP {status}: {}", body.trim())
                        };
                        should_count_failure = true;
                        if attempt < REQUEST_ATTEMPTS {
                            eprintln!(
                                "[cortex-mcp] Request failed (attempt {attempt}/{REQUEST_ATTEMPTS}): {last_err}"
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(
                                500 * attempt as u64,
                            ))
                            .await;
                            continue;
                        }
                        break;
                    }

                    if status.is_success() && has_id {
                        let body = body.trim();
                        if body.is_empty() {
                            last_err = "daemon returned an empty response body".to_string();
                            break;
                        }
                        if let Err(e) = serde_json::from_str::<Value>(body) {
                            last_err = format!("daemon returned invalid JSON-RPC: {e}");
                            break;
                        }
                        response_body = Some(body.to_string());
                    } else if !status.is_success() && has_id {
                        let body = body.trim();
                        if !body.is_empty() && serde_json::from_str::<Value>(body).is_ok() {
                            response_body = Some(body.to_string());
                        } else {
                            last_err = format!("daemon returned HTTP {status}");
                            if !body.is_empty() {
                                last_err.push_str(": ");
                                last_err.push_str(body);
                            }
                        }
                    } else if !status.is_success() {
                        eprintln!(
                            "[cortex-mcp] Notification request returned HTTP {status}: {}",
                            body.trim()
                        );
                    }

                    if consecutive_failures > 0 && status.is_success() {
                        let paths = CortexPaths::resolve();
                        drain_write_buffer(
                            &client,
                            &rpc_url,
                            api_key,
                            &agent_display,
                            agent_model.as_deref(),
                            &paths,
                        )
                        .await;
                    }
                    consecutive_failures = 0;
                    break;
                }
                Err(e) => {
                    last_err = format!("{e}");
                    should_count_failure = true;
                    if attempt < REQUEST_ATTEMPTS {
                        eprintln!(
                            "[cortex-mcp] Request failed (attempt {attempt}/{REQUEST_ATTEMPTS}): {e}"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64))
                            .await;
                    }
                }
            }
        }

        if response_body.is_none() && should_count_failure {
            consecutive_failures += 1;
            eprintln!(
                "[cortex-mcp] Request exhausted after {REQUEST_ATTEMPTS} attempts: {last_err} (consecutive failures: {consecutive_failures})"
            );
        }

        if response_body.is_none() && !last_err.is_empty() && has_id {
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            let err_resp = serde_json::json!({
                "jsonrpc": "2.0",
                "error": { "code": -32603, "message": format!("Daemon unavailable: {last_err}") },
                "id": id
            });
            if !write_value(&mut stdout, &err_resp).await? {
                finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, options)
                    .await;
                eprintln!("[cortex-mcp] Stdout closed while returning daemon error");
                return Ok(());
            }
        }

        if options.allow_respawn && should_count_failure && consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            let in_cooldown = last_respawn_attempt_at
                .map(|t| t.elapsed() < std::time::Duration::from_secs(RESPAWN_COOLDOWN_SECS))
                .unwrap_or(false);
            if in_cooldown {
                continue;
            }

            respawn_attempts += 1;
            last_respawn_attempt_at = Some(std::time::Instant::now());
            eprintln!(
                "[cortex-mcp] {consecutive_failures} consecutive failures; \
                 respawn attempt {respawn_attempts}"
            );

            let mut paths = CortexPaths::resolve();
            if daemon_lifecycle::try_respawn(&paths).await {
                // Daemon is back -- rebuild URLs using the latest resolved port.
                paths = CortexPaths::resolve();
                rpc_base_url = format!("http://127.0.0.1:{}", paths.port);
                rpc_url = format!("{rpc_base_url}/mcp-rpc");
                let _ = session_start(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    agent_model.as_deref(),
                )
                .await;
                drain_write_buffer(
                    &client,
                    &rpc_url,
                    api_key,
                    &agent_display,
                    agent_model.as_deref(),
                    &paths,
                )
                .await;
                consecutive_failures = 0;
                eprintln!("[cortex-mcp] Daemon recovered; resuming proxy");
            } else {
                eprintln!(
                    "[cortex-mcp] Respawn attempt {respawn_attempts} failed; \
                     will keep retrying while proxy stays online"
                );
            }
        }

        if let Some(body) = response_body {
            if !write_raw_line(&mut stdout, &body).await? {
                finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, options)
                    .await;
                eprintln!("[cortex-mcp] Stdout closed while returning daemon response");
                return Ok(());
            }
        }
    }
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

async fn write_value(
    stdout: &mut tokio::io::Stdout,
    value: &Value,
) -> Result<bool, std::io::Error> {
    write_raw_line(stdout, &value.to_string()).await
}

async fn write_raw_line(
    stdout: &mut tokio::io::Stdout,
    line: &str,
) -> Result<bool, std::io::Error> {
    if let Err(e) = stdout.write_all(format!("{line}\n").as_bytes()).await {
        return if e.kind() == std::io::ErrorKind::BrokenPipe {
            Ok(false)
        } else {
            Err(e)
        };
    }
    if let Err(e) = stdout.flush().await {
        return if e.kind() == std::io::ErrorKind::BrokenPipe {
            Ok(false)
        } else {
            Err(e)
        };
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_mcp_{name}_{unique}"))
    }

    #[test]
    fn persist_write_buffer_truncates_when_no_entries_remain() {
        let home_dir = temp_test_dir("write_buffer");
        fs::create_dir_all(&home_dir).unwrap();
        let buffer_path = home_dir.join("write_buffer.jsonl");
        fs::write(&buffer_path, "{\"old\":true}\n").unwrap();

        persist_write_buffer(&buffer_path, &[]).unwrap();

        assert_eq!(fs::read_to_string(&buffer_path).unwrap(), "");

        let _ = fs::remove_dir_all(&home_dir);
    }
}
