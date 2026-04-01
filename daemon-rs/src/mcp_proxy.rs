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
//! Falls back to standalone mode if the daemon is unreachable.

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_RPC_URL: &str = "http://127.0.0.1:7437/mcp-rpc";

/// Read the auth token from ~/.cortex/cortex.token.
fn read_auth_token() -> Option<String> {
    let token_path = crate::auth::cortex_dir().join("cortex.token");
    std::fs::read_to_string(token_path).ok().map(|t| t.trim().to_string())
}

/// Try to run in proxy mode. Returns `true` if proxy connected and ran,
/// `false` if daemon is unreachable (caller should fall back to standalone).
pub async fn run() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Health check -- is the daemon alive right now?
    // If not, still start in proxy mode. The daemon may come up later
    // and the retry loop will connect when it does.
    match client
        .get("http://127.0.0.1:7437/health")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            eprintln!("[cortex-mcp] Proxy mode -- forwarding to daemon on :7437");
        }
        _ => {
            eprintln!("[cortex-mcp] Proxy mode -- daemon not yet available, will retry on each request");
        }
    }

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();
    let mut daemon_was_down = false;

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
                let _ = stdout
                    .write_all(format!("{}\n", err).as_bytes())
                    .await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        let has_id = msg.get("id").is_some();

        // Retry indefinitely with exponential backoff capped at 30s.
        // The proxy NEVER gives up -- daemon restarts, rebuilds, and
        // temporary outages are all survivable. Re-reads auth token on
        // each attempt since daemon restart generates a new token.
        let mut attempt = 0u32;
        loop {
            let auth_header = read_auth_token()
                .map(|t| format!("Bearer {}", t))
                .unwrap_or_default();

            let mut req = client
                .post(MCP_RPC_URL)
                .header("content-type", "application/json");
            if !auth_header.is_empty() {
                req = req.header("authorization", &auth_header);
            }

            match req.body(trimmed.to_string()).send().await {
                Ok(resp) if resp.status().as_u16() == 401 => {
                    // Token mismatch -- daemon restarted with new token.
                    // Re-read token on next iteration (already done above).
                    attempt += 1;
                    let backoff = backoff_duration(attempt);
                    eprintln!("[cortex-mcp] Auth rejected (new token?), retry in {}s...", backoff.as_secs());
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Ok(resp) => {
                    if daemon_was_down {
                        eprintln!("[cortex-mcp] Daemon reconnected after {} retries", attempt);
                        daemon_was_down = false;
                    }
                    if has_id {
                        if let Ok(body) = resp.text().await {
                            let body = body.trim();
                            if !body.is_empty() {
                                let _ = stdout
                                    .write_all(format!("{}\n", body).as_bytes())
                                    .await;
                                let _ = stdout.flush().await;
                            }
                        }
                    }
                    break;
                }
                Err(e) => {
                    attempt += 1;

                    if attempt == 1 {
                        eprintln!("[cortex-mcp] Daemon unreachable: {e}");
                        daemon_was_down = true;
                    }

                    // Log periodically, not every retry
                    if attempt % 10 == 0 {
                        eprintln!("[cortex-mcp] Still waiting for daemon... ({attempt} retries, {}s backoff)", backoff_duration(attempt).as_secs());
                    }

                    let backoff = backoff_duration(attempt);
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            }
        }
    }

    eprintln!("[cortex-mcp] Proxy session ended (stdin closed)");
    true
}

/// Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s, 30s, 30s...
fn backoff_duration(attempt: u32) -> std::time::Duration {
    let secs = (1u64 << attempt.min(5)).min(30);
    std::time::Duration::from_secs(secs)
}
