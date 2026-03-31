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

/// Try to run in proxy mode. Returns `true` if proxy connected and ran,
/// `false` if daemon is unreachable (caller should fall back to standalone).
pub async fn run() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
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

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();
    let mut stdout = tokio::io::stdout();

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

        // Forward to daemon's /mcp-rpc endpoint
        match client
            .post(MCP_RPC_URL)
            .header("content-type", "application/json")
            .body(trimmed.to_string())
            .send()
            .await
        {
            Ok(resp) => {
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
                // Notifications (no id) -- fire and forget, no response needed
            }
            Err(e) => {
                eprintln!("[cortex-mcp] Proxy HTTP error: {e}");
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
            }
        }
    }

    eprintln!("[cortex-mcp] Proxy session ended");
    true
}
