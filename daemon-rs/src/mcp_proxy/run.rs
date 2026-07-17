use super::*;
use crate::auth::CortexPaths;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
pub async fn run(
    base_url: &str,
    api_key: Option<&str>,
    agent: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let api_key = normalize_api_key(api_key);
    let base_url = base_url.trim_end_matches('/');
    validate_target_base_url(base_url)?;
    if requires_explicit_api_key(base_url, api_key) {
        return Err(format!("Remote Cortex target '{base_url}' requires an API key. Pass --api-key <key> or set CORTEX_API_KEY.").
into());
    }
    let mut rpc_base_url = base_url.to_string();
    let mut health_url = format!("{rpc_base_url}/readiness");
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
    let mut healthy = false;
    let health_probe_headers = internal_health_probe_headers();
    for attempt in 1..=HEALTH_CHECK_ATTEMPTS {
        match transport_request_for_url(
            &client,
            "GET",
            &health_url,
            &health_probe_headers,
            None,
            std::time::Duration::from_secs(5),
        )
        .await
        {
            Ok((status, body)) if is_cortex_health_response(status, &body, &health_url) => {
                healthy = true;
                break;
            }
            Ok((status, _)) => {
                eprintln!(
"[cortex-mcp] Health check attempt {attempt}/{HEALTH_CHECK_ATTEMPTS}: HTTP {status} was not a valid Cortex health payload");
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
"[cortex-mcp] Local target is not identity-verified yet; withholding local token auth until health is valid");
    } else if healthy {
        let paths = CortexPaths::resolve();
        drain_write_buffer(
            &client,
            &rpc_base_url,
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
            let mut heartbeat_health_url = format!("{heartbeat_base_url}/readiness");
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
                                heartbeat_health_url = format!("{heartbeat_base_url}/readiness");
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
                                    heartbeat_health_url =
                                        format!("{heartbeat_base_url}/readiness");
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
        tokio::sync::mpsc::channel::<Result<Option<String>, String>>(STDIN_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let mut lines = reader.lines();
        loop {
            let next = match lines.next_line().await {
                Ok(Some(line)) => Ok(Some(line)),
                Ok(None) => Ok(None),
                Err(err) => Err(err.to_string()),
            };
            let should_stop = matches!(next, Ok(None) | Err(_));
            if stdin_tx.send(next).await.is_err() || should_stop {
                break;
            }
        }
    });
    let mut consecutive_failures: u32 = 0;
    let startup_timeout = startup_idle_timeout();
    let parent_process = current_parent_process();
    let mut saw_client_message = false;
    let mut orphan_check = tokio::time::interval(std::time::Duration::from_secs(ORPHAN_CHECK_SECS));
    orphan_check.tick().await;
    loop {
        let line = if !saw_client_message {
            let startup_sleep = tokio::time::sleep(startup_timeout);
            tokio::pin!(startup_sleep);
            tokio::select! {_=orphan_check.tick()=>{if let
            Some(parent_process)=parent_process{if!process_is_alive(parent_process){finalize_proxy_session(&client,&rpc_base_url,api_key,&
            agent_display,allow_local_token_fallback).await;eprintln!(
            "[cortex-mcp] Proxy session ended (parent process exited before handshake)");return Ok(());}}continue;}_=&mut startup_sleep=>{
            finalize_proxy_session(&client,&rpc_base_url,api_key,&agent_display,allow_local_token_fallback).await;eprintln!(
            "[cortex-mcp] Proxy session ended (no client handshake within {}s)",startup_timeout.as_secs());return Ok(());}result=stdin_rx.recv
            ()=>{match result{Some(Ok(Some(line)))=>line,Some(Ok(None))|None=>{finalize_proxy_session(&client,&rpc_base_url,api_key,&
            agent_display,allow_local_token_fallback).await;eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");return Ok(());}Some(
            Err(e))=>{finalize_proxy_session(&client,&rpc_base_url,api_key,&agent_display,allow_local_token_fallback).await;eprintln!(
            "[cortex-mcp] Stdin read error: {e}");return Err(std::io::Error::other(e).into());}}}}
        } else {
            tokio::select! {_=orphan_check.tick()=>
            {if let Some(parent_process)=parent_process{if!process_is_alive(parent_process){finalize_proxy_session(&client,&rpc_base_url,
            api_key,&agent_display,allow_local_token_fallback).await;eprintln!("[cortex-mcp] Proxy session ended (parent process exited)");
            return Ok(());}}continue;}result=stdin_rx.recv()=>{match result{Some(Ok(Some(line)))=>line,Some(Ok(None))|None=>{
            finalize_proxy_session(&client,&rpc_base_url,api_key,&agent_display,allow_local_token_fallback).await;eprintln!(
            "[cortex-mcp] Proxy session ended (stdin closed)");return Ok(());}Some(Err(e))=>{finalize_proxy_session(&client,&rpc_base_url,
            api_key,&agent_display,allow_local_token_fallback).await;eprintln!("[cortex-mcp] Stdin read error: {e}");return Err(std::io::Error
            ::other(e).into());}}}}
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
                let err = serde_json::json!({"jsonrpc":
"2.0","error":{"code":-32700,"message":"Parse error"},"id":null});
                if !write_value(&mut stdout, &err).await? {
                    finalize_proxy_session(
                        &client,
                        &rpc_base_url,
                        api_key,
                        &agent_display,
                        allow_local_token_fallback,
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
                health_url = format!("{rpc_base_url}/readiness");
                allow_local_token_fallback = !local_token_fallback_required(&rpc_base_url, api_key);
            }
        }
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
            let mut headers = vec![
                ("content-type".to_string(), "application/json".to_string()),
                ("x-cortex-request".to_string(), "true".to_string()),
                ("x-source-agent".to_string(), agent_display.clone()),
            ];
            if let Some(model) = agent_model.as_deref() {
                headers.push(("x-source-model".to_string(), model.to_string()));
            }
            match transport_request(
                &client,
                "POST",
                &rpc_base_url,
                "/mcp-rpc",
                api_key,
                allow_local_token_fallback,
                &headers,
                Some(trimmed),
                remaining.min(std::time::Duration::from_secs(10)),
            )
            .await
            {
                Ok((status, body)) => {
                    if api_key.is_none() && is_auth_recovery_status(status) {
                        last_err = if body.trim().is_empty() {
                            format!("daemon returned auth HTTP {status}")
                        } else {
                            format!("daemon returned auth HTTP {status}: {}", body.trim())
                        };
                        if attempt < REQUEST_ATTEMPTS {
                            invalidate_auth_token_cache();
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
                                    eprintln!("[cortex-mcp] Auth rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); refreshed token and retrying");
                                } else {
                                    eprintln!(
"[cortex-mcp] Auth rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); daemon looks live but auth recovery is still settling"
);
                                }
                            } else {
                                eprintln!(
"[cortex-mcp] Auth still rejected request (attempt {attempt}/{REQUEST_ATTEMPTS}); retrying once more before surfacing the error");
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
"[cortex-mcp] Request failed (attempt {attempt}/{REQUEST_ATTEMPTS}): {last_err}");
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
                            &rpc_base_url,
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
                    last_err = e.to_string();
                    should_count_failure = true;
                    if attempt < REQUEST_ATTEMPTS {
                        eprintln!(
"[cortex-mcp] Request failed (attempt {attempt}/{REQUEST_ATTEMPTS}): {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64))
                            .await;
                    }
                }
            }
        }
        if response_body.is_none() && should_count_failure {
            consecutive_failures += 1;
            eprintln!(
"[cortex-mcp] Request exhausted after {REQUEST_ATTEMPTS} attempts: {last_err} (consecutive failures: {consecutive_failures})");
        }
        if response_body.is_none() && !last_err.is_empty() && has_id {
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            let err_resp = serde_json
::json!({"jsonrpc":"2.0","error":{"code":-32603,"message":format!("Daemon unavailable: {last_err}")},"id":id});
            if !write_value(&mut stdout, &err_resp).await? {
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    allow_local_token_fallback,
                )
                .await;
                eprintln!("[cortex-mcp] Stdout closed while returning daemon error");
                return Ok(());
            }
        }
        if let Some(body) = response_body {
            if !write_raw_line(&mut stdout, &body).await? {
                finalize_proxy_session(
                    &client,
                    &rpc_base_url,
                    api_key,
                    &agent_display,
                    allow_local_token_fallback,
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
