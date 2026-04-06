// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
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

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const HEALTH_CHECK_ATTEMPTS: u32 = 5;
const REQUEST_ATTEMPTS: u32 = 3;
const MAX_CONSECUTIVE_FAILURES: u32 = 10;

/// Read the auth token from ~/.cortex/cortex.token.
fn read_auth_token() -> Option<String> {
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

/// Run MCP proxy over stdio -> HTTP.
pub async fn run(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let base_url = base_url.trim_end_matches('/');
    let rpc_url = format!("{base_url}/mcp-rpc");
    let health_url = format!("{base_url}/health");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .connect_timeout(std::time::Duration::from_secs(3))
        .build()?;

    let team_mode = detect_team_mode(api_key);
    if team_mode {
        eprintln!("[cortex-mcp] Team mode proxy -> {base_url}");
    } else {
        eprintln!("[cortex-mcp] Solo mode proxy -> {base_url}");
    }

    // Health check with retry (daemon may still be starting)
    let mut healthy = false;
    for attempt in 1..=HEALTH_CHECK_ATTEMPTS {
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                healthy = true;
                break;
            }
            Ok(resp) => {
                eprintln!(
                    "[cortex-mcp] Health check attempt {attempt}/{HEALTH_CHECK_ATTEMPTS}: HTTP {}",
                    resp.status()
                );
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

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();

    let mut consecutive_failures: u32 = 0;

    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
                return Ok(());
            }
            Err(e) => {
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
        let request_deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(15);

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
                .timeout(remaining.min(std::time::Duration::from_secs(10)));
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
                        tokio::time::sleep(std::time::Duration::from_millis(
                            500 * attempt as u64,
                        ))
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
                eprintln!("[cortex-mcp] Stdout closed while returning daemon error");
                return Ok(());
            }
        }

        if should_count_failure && consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            return Err(format!(
                "Daemon unreachable after {consecutive_failures} consecutive failures: {last_err}"
            )
            .into());
        }

        if let Some(body) = response_body {
            if !write_raw_line(&mut stdout, &body).await? {
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

