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

/// Read the auth token from ~/.cortex/cortex.token.
fn read_auth_token() -> Option<String> {
    let token_path = crate::auth::CortexPaths::resolve().token;
    std::fs::read_to_string(token_path)
        .ok()
        .map(|t| t.trim().to_string())
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
    let max_health_retries = 5;
    let mut healthy = false;
    for attempt in 1..=max_health_retries {
        match client.get(&health_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                healthy = true;
                break;
            }
            Ok(resp) => {
                eprintln!(
                    "[cortex-mcp] Health check attempt {attempt}/{max_health_retries}: HTTP {}",
                    resp.status()
                );
            }
            Err(e) => {
                eprintln!(
                    "[cortex-mcp] Health check attempt {attempt}/{max_health_retries}: {e}"
                );
            }
        }
        if attempt < max_health_retries {
            tokio::time::sleep(std::time::Duration::from_secs(attempt as u64)).await;
        }
    }
    if !healthy {
        return Err(format!(
            "Cortex daemon unreachable at {health_url} after {max_health_retries} attempts -- start with `cortex plugin ensure-daemon`"
        )
        .into());
    }

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();

    let max_retries: u32 = 3;
    let mut consecutive_failures: u32 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
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
                let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        let has_id = msg.get("id").is_some();

        // Retry loop for daemon requests
        let mut last_err = String::new();
        let mut resp_ok = None;
        for attempt in 0..=max_retries {
            let auth_header = if let Some(key) = api_key {
                Some(format!("Bearer {key}"))
            } else {
                read_auth_token().map(|token| format!("Bearer {token}"))
            };

            let mut req = client
                .post(&rpc_url)
                .header("content-type", "application/json")
                .header("x-cortex-request", "true");
            if let Some(auth) = auth_header {
                req = req.header("authorization", auth);
            }

            match req.body(trimmed.to_string()).send().await {
                Ok(resp) => {
                    resp_ok = Some(resp);
                    consecutive_failures = 0;
                    break;
                }
                Err(e) => {
                    last_err = format!("{e}");
                    if attempt < max_retries {
                        eprintln!(
                            "[cortex-mcp] Request failed (attempt {}/{}): {e}",
                            attempt + 1,
                            max_retries + 1
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(
                            500 * (attempt as u64 + 1),
                        ))
                        .await;
                    }
                }
            }
        }

        let resp = match resp_ok {
            Some(r) => r,
            None => {
                consecutive_failures += 1;
                eprintln!(
                    "[cortex-mcp] All {max_retries} retries failed: {last_err} (consecutive: {consecutive_failures})"
                );
                // Return error to client instead of killing the proxy
                if has_id {
                    let id = msg.get("id").cloned().unwrap_or(Value::Null);
                    let err_resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": { "code": -32603, "message": format!("Daemon unavailable: {last_err}") },
                        "id": id
                    });
                    let _ = stdout
                        .write_all(format!("{}\n", err_resp).as_bytes())
                        .await;
                    let _ = stdout.flush().await;
                }
                // Only die after 10 consecutive total failures (daemon is truly gone)
                if consecutive_failures >= 10 {
                    return Err(format!(
                        "Daemon unreachable after {consecutive_failures} consecutive failures: {last_err}"
                    )
                    .into());
                }
                continue;
            }
        };

        if has_id {
            let body = match resp.text().await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[cortex-mcp] Failed to read response body: {e}");
                    continue;
                }
            };
            let body = body.trim();
            if !body.is_empty() {
                stdout.write_all(format!("{body}\n").as_bytes()).await?;
                stdout.flush().await?;
            }
        }
    }

    eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
    Ok(())
}

