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

    // Quick health check -- is the daemon alive?
    match client
        .get("http://127.0.0.1:7437/health")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            eprintln!("[cortex-mcp] Proxy mode -- forwarding to daemon on :7437");
            eprintln!("[cortex-mcp] Shared ONNX engine, shared caches, zero duplication");
        }
        _ => {
            eprintln!("[cortex-mcp] Daemon unreachable -- falling back to standalone mode");
            return false;
        }
    }

    // Auth token is re-read on each request cycle because daemon restarts
    // generate a new token.  The file is tiny so this is negligible overhead.
    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();
    let mut consecutive_errors: u32 = 0;

    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Parse to check if it has an id (request vs notification)
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

        // Re-read auth token on every request (daemon restart = new token).
        let auth_header = read_auth_token()
            .map(|t| format!("Bearer {}", t))
            .unwrap_or_default();

        // Forward to daemon's /mcp-rpc endpoint with auth token.
        // Retry with backoff if daemon is temporarily unreachable (restart).
        let mut attempt = 0u32;
        let max_retries = 3;
        loop {
            let mut req = client
                .post(MCP_RPC_URL)
                .header("content-type", "application/json");
            if !auth_header.is_empty() {
                req = req.header("authorization", &auth_header);
            }

            match req.body(trimmed.to_string()).send().await {
                Ok(resp) if resp.status().as_u16() == 401 && attempt < max_retries => {
                    // Token mismatch -- daemon likely restarted. Wait and retry
                    // with freshly read token on next loop iteration.
                    attempt += 1;
                    eprintln!("[cortex-mcp] Auth rejected (daemon restarted?), retry {attempt}/{max_retries}...");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
                Ok(resp) => {
                    consecutive_errors = 0;
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
                Err(e) if attempt < max_retries => {
                    attempt += 1;
                    let backoff = std::time::Duration::from_secs(attempt as u64 * 2);
                    eprintln!("[cortex-mcp] Daemon unreachable, retry {attempt}/{max_retries} in {}s...", backoff.as_secs());
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    eprintln!("[cortex-mcp] Proxy HTTP error: {e}");
                    if consecutive_errors >= 5 {
                        eprintln!("[cortex-mcp] ERROR: Daemon appears to have died ({consecutive_errors} consecutive failures)");
                        eprintln!("[cortex-mcp] Restart daemon: cortex serve  OR  cortex service start");
                    }
                    if has_id {
                        let err_resp = serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": {
                                "code": -32603,
                                "message": format!("Daemon proxy error: {e}")
                            },
                            "id": msg.get("id")
                        });
                        let _ = stdout
                            .write_all(format!("{}\n", err_resp).as_bytes())
                            .await;
                        let _ = stdout.flush().await;
                    }
                    break;
                }
            }
        }
    }

    eprintln!("[cortex-mcp] Proxy session ended");
    consecutive_errors < 5
}
