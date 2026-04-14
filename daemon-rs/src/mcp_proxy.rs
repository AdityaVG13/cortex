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

fn is_local_daemon_base(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let host_ok = matches!(url.host_str(), Some("127.0.0.1" | "localhost"));
    let port_ok = url.port_or_known_default() == Some(CortexPaths::resolve().port);
    host_ok && port_ok
}

fn build_auth_header(base_url: &str, api_key: Option<&str>) -> Option<String> {
    if let Some(key) = api_key {
        return Some(format!("Bearer {key}"));
    }
    if is_local_daemon_base(base_url) {
        return read_auth_token().map(|token| format!("Bearer {token}"));
    }
    None
}

fn expected_port_from_url(url: &str) -> Option<u16> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.port_or_known_default())
}

fn is_cortex_health_response(
    status: reqwest::StatusCode,
    body: &str,
    expected_port: Option<u16>,
) -> bool {
    if !status.is_success() {
        return false;
    }

    let Ok(json) = serde_json::from_str::<Value>(body.trim()) else {
        return false;
    };

    let health_status = json.get("status").and_then(|value| value.as_str());
    let runtime = json.get("runtime").and_then(|value| value.as_object());
    let stats = json.get("stats").and_then(|value| value.as_object());
    let runtime_port = runtime
        .and_then(|runtime| runtime.get("port"))
        .and_then(|value| value.as_u64())
        .and_then(|value| u16::try_from(value).ok());

    if let Some(expected_port) = expected_port {
        if runtime_port != Some(expected_port) {
            return false;
        }
    }

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

    is_cortex_health_response(status, &body, expected_port_from_url(health_url))
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
        if let Some(auth) = build_auth_header(rpc_url, api_key) {
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

    if let Some(auth) = build_auth_header(base_url, api_key) {
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

    if let Some(auth) = build_auth_header(base_url, api_key) {
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

    if let Some(auth) = build_auth_header(base_url, api_key) {
        req = req.header("authorization", auth);
    }

    match req.send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn shutdown_daemon(client: &reqwest::Client, base_url: &str, api_key: Option<&str>) -> bool {
    let mut req = client
        .post(format!("{base_url}/shutdown"))
        .header("content-type", "application/json")
        .header("x-cortex-request", "true")
        .body("{}");

    if let Some(auth) = build_auth_header(base_url, api_key) {
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
    options: ProxyRuntimeOptions,
) {
    let _ = session_end(client, base_url, api_key, agent).await;
    if options.shutdown_on_exit || (options.shutdown_on_idle_startup && !saw_client_message) {
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
    let api_key = normalize_api_key(api_key);
    let base_url = base_url.trim_end_matches('/');
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
                    Ok(body)
                        if is_cortex_health_response(
                            status,
                            &body,
                            expected_port_from_url(&health_url),
                        ) =>
                    {
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
        let heartbeat_base_tx = rpc_base_tx.clone();
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
            let mut heartbeat_base_url = heartbeat_base_url;
            let mut heartbeat_health_url = format!("{heartbeat_base_url}/health");
            let resolved_local_base = format!("http://127.0.0.1:{}", CortexPaths::resolve().port);
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
                            SESSION_RESTART_ATTEMPTS,
                            SESSION_RESTART_DELAY_MS,
                        )
                        .await;
                        if !restarted && heartbeat_can_refresh_local {
                            let refreshed_base =
                                format!("http://127.0.0.1:{}", CortexPaths::resolve().port);
                            if refreshed_base != heartbeat_base_url {
                                heartbeat_base_url = refreshed_base;
                                heartbeat_health_url = format!("{heartbeat_base_url}/health");
                                let _ = heartbeat_base_tx.send(heartbeat_base_url.clone());
                                restarted = session_start_with_retry(
                                    &hb_client,
                                    &heartbeat_base_url,
                                    heartbeat_api_key.as_deref(),
                                    &heartbeat_agent,
                                    heartbeat_model.as_deref(),
                                    SESSION_RESTART_ATTEMPTS,
                                    SESSION_RESTART_DELAY_MS,
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
                                    format!("http://127.0.0.1:{}", CortexPaths::resolve().port);
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
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (parent process exited before handshake)");
                            return Ok(());
                        }
                    }
                    continue;
                }
                _ = &mut startup_sleep => {
                    finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
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
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
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
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
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
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
                                .await;
                            eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                            return Ok(());
                        }
                        Some(Err(e)) => {
                            finalize_proxy_session(&client, &rpc_base_url, api_key, &agent_display, saw_client_message, options)
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

            let auth_header = build_auth_header(&rpc_base_url, api_key);

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
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    saw_client_message,
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
            if daemon_lifecycle::try_respawn(&paths).await {
                // Daemon is back -- rebuild URLs using the latest resolved port.
                paths = CortexPaths::resolve();
                rpc_base_url = format!("http://127.0.0.1:{}", paths.port);
                rpc_url = format!("{rpc_base_url}/mcp-rpc");
                health_url = format!("{rpc_base_url}/health");
                let _ = rpc_base_tx.send(rpc_base_url.clone());
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
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    saw_client_message,
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
            Some(7437)
        ));
        assert!(!is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            Some(9000)
        ));
        assert!(is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            None
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
        assert_eq!(build_auth_header(custom_base, None), None);
    }
}
