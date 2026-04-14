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
use sysinfo::{ProcessesToUpdate, System};
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
const STARTUP_IDLE_TIMEOUT_SECS: u64 = 60;
const ORPHAN_CHECK_SECS: u64 = 15;
const MAX_AGENT_HEADER_LEN: usize = 160;
const MAX_MODEL_HEADER_LEN: usize = 160;

#[derive(Clone, Copy, Debug, Default)]
pub struct ProxyRuntimeOptions {
    pub allow_respawn: bool,
    pub shutdown_on_exit: bool,
    pub shutdown_on_idle_startup: bool,
    pub respawn_owner: Option<&'static str>,
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

fn startup_idle_timeout() -> std::time::Duration {
    let secs = std::env::var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(STARTUP_IDLE_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs.max(1))
}

fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_header_value(raw: &str, max_len: usize) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > max_len {
        return None;
    }
    if !trimmed.is_ascii() {
        return None;
    }
    if trimmed
        .as_bytes()
        .iter()
        .any(|byte| *byte <= 31 || *byte == 127)
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_api_key(api_key: Option<&str>) -> Option<&str> {
    api_key.map(str::trim).filter(|value| !value.is_empty())
}

fn detect_agent_hint(value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    if value.contains("codex") {
        return Some("codex");
    }
    if value.contains("cursor") {
        return Some("cursor");
    }
    if value.contains("gemini") {
        return Some("gemini");
    }
    if value.contains("claude") {
        return Some("claude-code");
    }
    if value.contains("cline") {
        return Some("cline");
    }
    None
}

fn infer_agent_from_process_tree() -> Option<String> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let current_pid = sysinfo::get_current_pid().ok()?;
    let mut next_pid = Some(current_pid);
    let mut depth = 0usize;

    while let Some(pid) = next_pid {
        let process = system.process(pid)?;
        let candidates = [
            process.name().to_string_lossy().into_owned(),
            process
                .exe()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" "),
        ];

        for candidate in candidates {
            if let Some(agent) = detect_agent_hint(&candidate) {
                return Some(agent.to_string());
            }
        }

        next_pid = process.parent();
        depth += 1;
        if depth >= 6 {
            break;
        }
    }

    None
}

fn current_parent_pid() -> Option<sysinfo::Pid> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let current_pid = sysinfo::get_current_pid().ok()?;
    system.process(current_pid)?.parent()
}

fn process_is_alive(pid: sysinfo::Pid) -> bool {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).is_some()
}

fn resolve_agent_identity(agent_arg: Option<&str>) -> (String, Option<String>) {
    let model = env_trimmed("CORTEX_AGENT_MODEL")
        .or_else(|| env_trimmed("CORTEX_MODEL"))
        .and_then(|value| normalize_header_value(&value, MAX_MODEL_HEADER_LEN));

    let mut agent = env_trimmed("CORTEX_AGENT_DISPLAY")
        .or_else(|| {
            agent_arg
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| env_trimmed("CORTEX_AGENT_NAME"))
        .or_else(infer_agent_from_process_tree)
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

    let agent = match normalize_header_value(&agent, MAX_AGENT_HEADER_LEN) {
        Some(agent) => agent,
        None => {
            eprintln!(
                "[cortex-mcp] Invalid source agent label after normalization; falling back to 'mcp'"
            );
            "mcp".to_string()
        }
    };

    (agent, model)
}

fn normalized_host(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

fn local_daemon_base_from_paths(paths: &CortexPaths) -> String {
    let bind = paths.bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{}", paths.port)
}

fn is_local_daemon_base(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let paths = CortexPaths::resolve();
    let port_ok = url.port_or_known_default() == Some(paths.port);
    let host_norm = normalized_host(host);
    let bind_norm = normalized_host(&paths.bind);
    let host_ok = matches!(host_norm.as_str(), "127.0.0.1" | "localhost" | "::1")
        || (!bind_norm.is_empty()
            && !matches!(bind_norm.as_str(), "0.0.0.0" | "::")
            && host_norm == bind_norm);
    host_ok && port_ok
}

fn local_token_fallback_required(base_url: &str, api_key: Option<&str>) -> bool {
    api_key.is_none() && is_local_daemon_base(base_url)
}

fn build_auth_header(
    base_url: &str,
    api_key: Option<&str>,
    allow_local_token_fallback: bool,
) -> Option<String> {
    if let Some(key) = api_key {
        return Some(format!("Bearer {key}"));
    }
    if allow_local_token_fallback && local_token_fallback_required(base_url, api_key) {
        return read_auth_token().map(|token| format!("Bearer {token}"));
    }
    None
}

fn requires_explicit_api_key(base_url: &str, api_key: Option<&str>) -> bool {
    api_key.is_none() && !is_local_daemon_base(base_url)
}

fn validate_target_base_url(base_url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(base_url).map_err(|_| {
        format!(
            "Invalid Cortex target URL '{base_url}'. Use an absolute http:// or https:// base URL."
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported Cortex target URL scheme '{}' in '{base_url}'. Use http or https.",
            parsed.scheme()
        ));
    }
    if parsed.host_str().is_none() {
        return Err(format!(
            "Invalid Cortex target URL '{base_url}': missing host."
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "Cortex target URL must not include embedded credentials; pass --api-key instead."
                .to_string(),
        );
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(
            "Cortex target URL must not include query parameters or fragments.".to_string(),
        );
    }
    Ok(())
}

fn expected_port_from_url(url: &str) -> Option<u16> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.port_or_known_default())
}

fn is_cortex_health_response(status: reqwest::StatusCode, body: &str, health_url: &str) -> bool {
    let local_paths = if is_local_daemon_base(health_url) {
        Some(CortexPaths::resolve())
    } else {
        None
    };
    daemon_lifecycle::is_cortex_health_payload(
        status.as_u16(),
        body,
        expected_port_from_url(health_url),
        local_paths.as_ref(),
    )
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

    is_cortex_health_response(status, &body, health_url)
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
    allow_local_token_fallback: &mut bool,
) -> bool {
    if !health_check_ready(client, health_url).await {
        *allow_local_token_fallback = false;
        return false;
    }
    *allow_local_token_fallback = true;

    if !session_start_with_retry(
        client,
        base_url,
        None,
        agent,
        model,
        *allow_local_token_fallback,
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
    allow_local_token_fallback: bool,
) -> bool {
    for attempt in 1..=SESSION_RESTART_ATTEMPTS.max(1) {
        if session_start(
            client,
            base_url,
            api_key,
            agent,
            model,
            allow_local_token_fallback,
        )
        .await
        {
            return true;
        }

        if attempt < SESSION_RESTART_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(
                SESSION_RESTART_DELAY_MS * attempt as u64,
            ))
            .await;
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
    allow_local_token_fallback: bool,
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
        if let Some(auth) = build_auth_header(rpc_url, api_key, allow_local_token_fallback) {
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
    allow_local_token_fallback: bool,
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

    if let Some(auth) = build_auth_header(base_url, api_key, allow_local_token_fallback) {
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
    allow_local_token_fallback: bool,
) -> SessionHeartbeatOutcome {
    let mut req = client
        .post(format!("{base_url}/session/heartbeat"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .json(&serde_json::json!({
            "agent": agent,
            "description": model.map(|m| format!("MCP session · {m}")).unwrap_or_else(|| "MCP session".to_string())
        }));

    if let Some(auth) = build_auth_header(base_url, api_key, allow_local_token_fallback) {
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
    allow_local_token_fallback: bool,
) -> bool {
    let mut req = client
        .post(format!("{base_url}/session/end"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .json(&serde_json::json!({ "agent": agent }));

    if let Some(auth) = build_auth_header(base_url, api_key, allow_local_token_fallback) {
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
    allow_local_token_fallback: bool,
) -> bool {
    let mut req = client
        .post(format!("{base_url}/shutdown"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .body("{}");

    if let Some(auth) = build_auth_header(base_url, api_key, allow_local_token_fallback) {
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
    saw_client_message: bool,
    allow_local_token_fallback: bool,
    options: ProxyRuntimeOptions,
) {
    let _ = session_end(client, base_url, api_key, agent, allow_local_token_fallback).await;
    if options.shutdown_on_exit || (options.shutdown_on_idle_startup && !saw_client_message) {
        if shutdown_daemon(client, base_url, api_key, allow_local_token_fallback).await {
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
    let api_key = normalize_api_key(api_key);
    let base_url = base_url.trim_end_matches('/');
    validate_target_base_url(base_url)?;
    if requires_explicit_api_key(base_url, api_key) {
        return Err(format!(
            "Remote Cortex target '{base_url}' requires an API key. Pass --api-key <key> or set CORTEX_API_KEY."
        )
        .into());
    }
    let mut rpc_base_url = base_url.to_string();
    let mut rpc_url = format!("{rpc_base_url}/mcp-rpc");
    let mut health_url = format!("{rpc_base_url}/health");
    let (rpc_base_tx, mut rpc_base_rx) = tokio::sync::watch::channel(rpc_base_url.clone());
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
                    Ok(body) if is_cortex_health_response(status, &body, &health_url) => {
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
    }

    let mut allow_local_token_fallback =
        !local_token_fallback_required(&rpc_base_url, api_key) || healthy;
    if local_token_fallback_required(&rpc_base_url, api_key) && !allow_local_token_fallback {
        eprintln!(
            "[cortex-mcp] Local target is not identity-verified yet; withholding local token auth until health is valid"
        );
    } else if healthy {
        let paths = CortexPaths::resolve();
        drain_write_buffer(
            &client,
            &rpc_url,
            api_key,
            &agent_display,
            agent_model.as_deref(),
            &paths,
            allow_local_token_fallback,
        )
        .await;
    }

    if allow_local_token_fallback || !local_token_fallback_required(&rpc_base_url, api_key) {
        let _ = session_start_with_retry(
            &client,
            &rpc_base_url,
            api_key,
            &agent_display,
            agent_model.as_deref(),
            allow_local_token_fallback,
        )
        .await;
    }

    // Spawn background heartbeat to keep sessions visible and recover after daemon restarts.
    {
        let heartbeat_base_url = rpc_base_url.clone();
        let heartbeat_base_tx = rpc_base_tx.clone();
        let heartbeat_agent = agent_display.clone();
        let heartbeat_model = agent_model.clone();
        let heartbeat_api_key = api_key.map(String::from);
        let mut heartbeat_allow_local_token_fallback = allow_local_token_fallback;
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
            let mut heartbeat_base_url = heartbeat_base_url;
            let mut heartbeat_health_url = format!("{heartbeat_base_url}/health");
            let resolved_local_base = local_daemon_base_from_paths(&CortexPaths::resolve());
            let heartbeat_can_refresh_local = heartbeat_base_url == resolved_local_base;
            let mut consecutive_heartbeat_failures = 0u32;

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(SESSION_HEARTBEAT_SECS)).await;
                match session_heartbeat(
                    &hb_client,
                    &heartbeat_base_url,
                    heartbeat_api_key.as_deref(),
                    &heartbeat_agent,
                    heartbeat_model.as_deref(),
                    heartbeat_allow_local_token_fallback,
                )
                .await
                {
                    SessionHeartbeatOutcome::Renewed => {
                        consecutive_heartbeat_failures = 0;
                    }
                    SessionHeartbeatOutcome::MissingSession => {
                        consecutive_heartbeat_failures = 0;
                        let mut restarted = session_start_with_retry(
                            &hb_client,
                            &heartbeat_base_url,
                            heartbeat_api_key.as_deref(),
                            &heartbeat_agent,
                            heartbeat_model.as_deref(),
                            heartbeat_allow_local_token_fallback,
                        )
                        .await;
                        if !restarted
                            && local_token_fallback_required(
                                &heartbeat_base_url,
                                heartbeat_api_key.as_deref(),
                            )
                            && !heartbeat_allow_local_token_fallback
                            && health_check_ready(&hb_client, &heartbeat_health_url).await
                        {
                            heartbeat_allow_local_token_fallback = true;
                            restarted = session_start_with_retry(
                                &hb_client,
                                &heartbeat_base_url,
                                heartbeat_api_key.as_deref(),
                                &heartbeat_agent,
                                heartbeat_model.as_deref(),
                                heartbeat_allow_local_token_fallback,
                            )
                            .await;
                        }
                        if !restarted && heartbeat_can_refresh_local {
                            let refreshed_base =
                                local_daemon_base_from_paths(&CortexPaths::resolve());
                            if refreshed_base != heartbeat_base_url {
                                heartbeat_base_url = refreshed_base;
                                heartbeat_health_url = format!("{heartbeat_base_url}/health");
                                let _ = heartbeat_base_tx.send(heartbeat_base_url.clone());
                                heartbeat_allow_local_token_fallback =
                                    !local_token_fallback_required(
                                        &heartbeat_base_url,
                                        heartbeat_api_key.as_deref(),
                                    );
                                restarted = session_start_with_retry(
                                    &hb_client,
                                    &heartbeat_base_url,
                                    heartbeat_api_key.as_deref(),
                                    &heartbeat_agent,
                                    heartbeat_model.as_deref(),
                                    heartbeat_allow_local_token_fallback,
                                )
                                .await;
                            }
                        }
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
                            if heartbeat_can_refresh_local {
                                let refreshed_base =
                                    local_daemon_base_from_paths(&CortexPaths::resolve());
                                if refreshed_base != heartbeat_base_url {
                                    heartbeat_base_url = refreshed_base;
                                    heartbeat_health_url = format!("{heartbeat_base_url}/health");
                                    let _ = heartbeat_base_tx.send(heartbeat_base_url.clone());
                                }
                            }
                            if !health_check_ready(&hb_client, &heartbeat_health_url).await {
                                continue;
                            }
                        }
                        heartbeat_allow_local_token_fallback = true;

                        let restarted = session_start_with_retry(
                            &hb_client,
                            &heartbeat_base_url,
                            heartbeat_api_key.as_deref(),
                            &heartbeat_agent,
                            heartbeat_model.as_deref(),
                            heartbeat_allow_local_token_fallback,
                        )
                        .await;
                        if restarted {
                            eprintln!(
                                "[cortex-mcp] Recovered heartbeat session for {heartbeat_agent}"
                            );
                        }
                    }
                }
            }
        });
    }

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let (stdin_tx, mut stdin_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Option<String>, String>>();
    tokio::spawn(async move {
        let mut lines = reader.lines();
        loop {
            let next = match lines.next_line().await {
                Ok(Some(line)) => Ok(Some(line)),
                Ok(None) => Ok(None),
                Err(err) => Err(err.to_string()),
            };
            let should_stop = matches!(next, Ok(None) | Err(_));
            if stdin_tx.send(next).is_err() || should_stop {
                break;
            }
        }
    });

    let mut consecutive_failures: u32 = 0;
    let mut respawn_attempts: u32 = 0;
    let mut last_respawn_attempt_at: Option<std::time::Instant> = None;
    let startup_timeout = startup_idle_timeout();
    let parent_pid = current_parent_pid();
    let mut saw_client_message = false;
    let mut orphan_check = tokio::time::interval(std::time::Duration::from_secs(ORPHAN_CHECK_SECS));
    orphan_check.tick().await;

    loop {
        let line = if !saw_client_message {
            let startup_sleep = tokio::time::sleep(startup_timeout);
            tokio::pin!(startup_sleep);
            tokio::select! {
                _ = orphan_check.tick() => {
                    if let Some(parent_pid) = parent_pid {
                        if !process_is_alive(parent_pid) {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (parent process exited before handshake)");
                            return Ok(());
                        }
                    }
                    continue;
                }
                _ = &mut startup_sleep => {
                    finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                        .await;
                    eprintln!(
                        "[cortex-mcp] Proxy session ended (no client handshake within {}s)",
                        startup_timeout.as_secs()
                    );
                    return Ok(());
                }
                result = stdin_rx.recv() => {
                    match result {
                        Some(Ok(Some(line))) => line,
                        Some(Ok(None)) | None => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Stdin read error: {e}");
                            return Err(std::io::Error::other(e).into());
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                _ = orphan_check.tick() => {
                    if let Some(parent_pid) = parent_pid {
                        if !process_is_alive(parent_pid) {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (parent process exited)");
                            return Ok(());
                        }
                    }
                    continue;
                }
                result = stdin_rx.recv() => {
                    match result {
                        Some(Ok(Some(line))) => line,
                        Some(Ok(None)) | None => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, allow_local_token_fallback, options)
                                .await;
                            eprintln!("[cortex-mcp] Stdin read error: {e}");
                            return Err(std::io::Error::other(e).into());
                        }
                    }
                }
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        saw_client_message = true;

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
                        saw_client_message,
                        allow_local_token_fallback,
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
        if rpc_base_rx.has_changed().unwrap_or(false) {
            let refreshed_base = rpc_base_rx.borrow_and_update().clone();
            if refreshed_base != rpc_base_url {
                rpc_base_url = refreshed_base;
                rpc_url = format!("{rpc_base_url}/mcp-rpc");
                health_url = format!("{rpc_base_url}/health");
                allow_local_token_fallback = !local_token_fallback_required(&rpc_base_url, api_key);
            }
        }

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

            let auth_header = build_auth_header(&rpc_base_url, api_key, allow_local_token_fallback);

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
                                    &mut allow_local_token_fallback,
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
                            allow_local_token_fallback,
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
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    saw_client_message,
                    allow_local_token_fallback,
                    options,
                )
                .await;
                eprintln!("[cortex-mcp] Stdout closed while returning daemon error");
                return Ok(());
            }
        }

        if options.allow_respawn
            && should_count_failure
            && consecutive_failures >= MAX_CONSECUTIVE_FAILURES
        {
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
            if daemon_lifecycle::try_respawn(&paths, options.respawn_owner).await {
                // Daemon is back -- rebuild URLs using the latest resolved port.
                paths = CortexPaths::resolve();
                rpc_base_url = local_daemon_base_from_paths(&paths);
                rpc_url = format!("{rpc_base_url}/mcp-rpc");
                health_url = format!("{rpc_base_url}/health");
                allow_local_token_fallback = !local_token_fallback_required(&rpc_base_url, api_key)
                    || health_check_ready(&client, &health_url).await;
                let _ = rpc_base_tx.send(rpc_base_url.clone());
                let _ = session_start(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    agent_model.as_deref(),
                    allow_local_token_fallback,
                )
                .await;
                drain_write_buffer(
                    &client,
                    &rpc_url,
                    api_key,
                    &agent_display,
                    agent_model.as_deref(),
                    &paths,
                    allow_local_token_fallback,
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
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    saw_client_message,
                    allow_local_token_fallback,
                    options,
                )
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

    #[test]
    fn detect_agent_hint_matches_known_clients() {
        assert_eq!(detect_agent_hint("Codex.exe"), Some("codex"));
        assert_eq!(detect_agent_hint("cursor-agent"), Some("cursor"));
        assert_eq!(detect_agent_hint("Gemini CLI"), Some("gemini"));
        assert_eq!(detect_agent_hint("Claude Code"), Some("claude-code"));
    }

    #[test]
    fn startup_idle_timeout_respects_env_override_and_floor() {
        std::env::remove_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS");
        assert_eq!(startup_idle_timeout().as_secs(), STARTUP_IDLE_TIMEOUT_SECS);

        std::env::set_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS", "0");
        assert_eq!(startup_idle_timeout().as_secs(), 1);

        std::env::set_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS", "75");
        assert_eq!(startup_idle_timeout().as_secs(), 75);

        std::env::remove_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS");
    }

    #[test]
    fn is_cortex_health_response_validates_expected_port() {
        let body =
            r#"{"status":"ok","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#;
        assert!(is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "https://example.com:7437/health"
        ));
        assert!(!is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "https://example.com:9000/health"
        ));
        assert!(is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "invalid-url"
        ));
    }

    #[test]
    fn normalize_api_key_treats_blank_values_as_missing() {
        assert_eq!(normalize_api_key(None), None);
        assert_eq!(normalize_api_key(Some("")), None);
        assert_eq!(normalize_api_key(Some("   ")), None);
        assert_eq!(normalize_api_key(Some(" ctx_abc ")), Some("ctx_abc"));
    }

    #[test]
    fn normalize_header_value_rejects_invalid_characters() {
        assert_eq!(
            normalize_header_value("codex-cli", MAX_AGENT_HEADER_LEN),
            Some("codex-cli".to_string())
        );
        assert_eq!(
            normalize_header_value("bad\nvalue", MAX_AGENT_HEADER_LEN),
            None
        );
        assert_eq!(normalize_header_value("módèl", MAX_MODEL_HEADER_LEN), None);
    }

    #[test]
    fn custom_url_without_api_key_does_not_use_local_token_fallback() {
        let custom_base = "https://example.com";
        assert!(!is_local_daemon_base(custom_base));
        assert_eq!(build_auth_header(custom_base, None, true), None);
    }

    #[test]
    fn remote_target_requires_explicit_api_key() {
        let remote_base = "https://example.com";
        assert!(requires_explicit_api_key(remote_base, None));
        assert!(!requires_explicit_api_key(remote_base, Some("ctx_remote")));
    }

    #[test]
    fn validate_target_base_url_rejects_invalid_or_unsafe_values() {
        assert!(validate_target_base_url("https://example.com").is_ok());
        assert!(validate_target_base_url("ftp://example.com").is_err());
        assert!(validate_target_base_url("https://user:pass@example.com").is_err());
        assert!(validate_target_base_url("https://example.com?x=1").is_err());
        assert!(validate_target_base_url("not-a-url").is_err());
    }

    #[test]
    fn configured_bind_host_is_treated_as_local_for_token_fallback() {
        let home_dir = temp_test_dir("configured_bind_local");
        fs::create_dir_all(&home_dir).unwrap();
        fs::write(home_dir.join("cortex.token"), "ctx_local").unwrap();

        std::env::set_var("CORTEX_HOME", &home_dir);
        std::env::set_var("CORTEX_PORT", "7437");
        std::env::set_var("CORTEX_BIND", "100.64.0.12");

        let local_base = "http://100.64.0.12:7437";
        assert!(is_local_daemon_base(local_base));
        assert_eq!(
            build_auth_header(local_base, None, true),
            Some("Bearer ctx_local".to_string())
        );
        assert_eq!(build_auth_header(local_base, None, false), None);

        std::env::remove_var("CORTEX_HOME");
        std::env::remove_var("CORTEX_PORT");
        std::env::remove_var("CORTEX_BIND");
        let _ = fs::remove_dir_all(&home_dir);
    }
}
