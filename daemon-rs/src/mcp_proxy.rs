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
//! - Solo mode: falls back to standalone if daemon unreachable
//! - Team mode: fails closed (auth requires daemon, no standalone fallback)
//! - Write buffer: failed POST requests buffered to write_buffer.jsonl for replay

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const MCP_RPC_URL: &str = "http://127.0.0.1:7437/mcp-rpc";

/// Read the auth token from ~/.cortex/cortex.token.
fn read_auth_token() -> Option<String> {
    let token_path = crate::auth::cortex_dir().join("cortex.token");
    std::fs::read_to_string(token_path)
        .ok()
        .map(|t| t.trim().to_string())
}

/// Detect team mode without a full DB open.
/// Checks: (1) ~/.cortex/mode file, (2) token starts with ctx_ prefix.
fn detect_team_mode() -> bool {
    let cortex_dir = crate::auth::cortex_dir();

    // Check explicit mode file first
    if let Ok(mode) = std::fs::read_to_string(cortex_dir.join("mode")) {
        if mode.trim() == "team" {
            return true;
        }
    }

    // Check if token has team-mode prefix
    if let Some(token) = read_auth_token() {
        if token.starts_with("ctx_") {
            return true;
        }
    }

    false
}

/// Append a failed request body to the write buffer for later replay.
fn buffer_write(body: &str) {
    let buf_path = crate::auth::cortex_dir().join("write_buffer.jsonl");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&buf_path)
    {
        let _ = writeln!(f, "{}", body);
        eprintln!("[cortex-mcp] Daemon unreachable, buffered write to write_buffer.jsonl");
    }
}

/// Drain buffered writes by replaying them to the daemon.
async fn drain_write_buffer(client: &reqwest::Client) {
    let buf_path = crate::auth::cortex_dir().join("write_buffer.jsonl");
    if !buf_path.exists() {
        return;
    }

    let content = match std::fs::read_to_string(&buf_path) {
        Ok(c) if !c.trim().is_empty() => c,
        _ => return,
    };

    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return;
    }

    let mut drained = 0usize;
    let mut remaining = Vec::new();

    for line in &lines {
        let auth_header = read_auth_token()
            .map(|t| format!("Bearer {}", t))
            .unwrap_or_default();

        let mut req = client
            .post(MCP_RPC_URL)
            .header("content-type", "application/json")
            .header("x-cortex-request", "true");
        if !auth_header.is_empty() {
            req = req.header("authorization", &auth_header);
        }

        match req.body(line.to_string()).send().await {
            Ok(resp) if resp.status().is_success() => {
                drained += 1;
            }
            _ => {
                remaining.push(*line);
            }
        }
    }

    // Rewrite buffer with only failed lines (or delete if empty)
    if remaining.is_empty() {
        let _ = std::fs::remove_file(&buf_path);
    } else {
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(&buf_path) {
            for line in &remaining {
                let _ = writeln!(f, "{}", line);
            }
        }
    }

    if drained > 0 {
        eprintln!("[cortex-mcp] Drained {drained} buffered writes");
    }
}

/// Returns true if the JSON-RPC method is a write operation (store, activity, etc.)
fn is_write_method(msg: &Value) -> bool {
    if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
        matches!(
            method,
            "tools/call"
        ) && msg
            .pointer("/params/name")
            .and_then(|n| n.as_str())
            .map(|name| {
                matches!(
                    name,
                    "cortex_store"
                        | "cortex_diary"
                        | "cortex_forget"
                        | "cortex_activity"
                        | "cortex_message"
                        | "cortex_feedback"
                )
            })
            .unwrap_or(false)
    } else {
        false
    }
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

    let team_mode = detect_team_mode();

    // Health check -- is the daemon alive right now?
    let daemon_alive = match client.get("http://127.0.0.1:7437/health").send().await {
        Ok(r) if r.status().is_success() => {
            eprintln!("[cortex-mcp] Proxy mode -- forwarding to daemon on :7437");
            true
        }
        _ => {
            if team_mode {
                eprintln!(
                    "[cortex-mcp] Team mode daemon unreachable -- failing closed (auth requires daemon)"
                );
                // Return true to prevent standalone fallback; exit with error
                std::process::exit(1);
            }
            eprintln!(
                "[cortex-mcp] Proxy mode -- daemon not yet available, will retry on each request"
            );
            false
        }
    };

    // Drain any buffered writes from previous sessions
    if daemon_alive {
        drain_write_buffer(&client).await;
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
                let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                let _ = stdout.flush().await;
                continue;
            }
        };

        let has_id = msg.get("id").is_some();
        let is_write = is_write_method(&msg);

        // Retry with exponential backoff capped at 30s.
        // Re-reads auth token on each attempt since daemon restart generates a new token.
        let mut attempt = 0u32;
        let max_attempts = if team_mode { 5u32 } else { u32::MAX };

        loop {
            let auth_header = read_auth_token()
                .map(|t| format!("Bearer {}", t))
                .unwrap_or_default();

            let mut req = client
                .post(MCP_RPC_URL)
                .header("content-type", "application/json")
                .header("x-cortex-request", "true");
            if !auth_header.is_empty() {
                req = req.header("authorization", &auth_header);
            }

            match req.body(trimmed.to_string()).send().await {
                Ok(resp) if resp.status().as_u16() == 401 => {
                    attempt += 1;
                    if attempt >= max_attempts {
                        if team_mode {
                            eprintln!(
                                "[cortex-mcp] Team mode daemon auth failed after {attempt} retries -- failing closed"
                            );
                            let err = serde_json::json!({
                                "jsonrpc": "2.0",
                                "error": { "code": -32000, "message": "Team mode: daemon auth failed" },
                                "id": msg.get("id")
                            });
                            let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                        break;
                    }
                    let backoff = backoff_duration(attempt);
                    eprintln!(
                        "[cortex-mcp] Auth rejected (new token?), retry in {}s...",
                        backoff.as_secs()
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                Ok(resp) => {
                    if daemon_was_down {
                        eprintln!("[cortex-mcp] Daemon reconnected after {} retries", attempt);
                        daemon_was_down = false;
                        // Drain buffered writes on reconnect
                        drain_write_buffer(&client).await;
                    }
                    if has_id {
                        if let Ok(body) = resp.text().await {
                            let body = body.trim();
                            if !body.is_empty() {
                                let _ = stdout.write_all(format!("{}\n", body).as_bytes()).await;
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

                    // Team mode: fail closed after limited retries
                    if team_mode && attempt >= max_attempts {
                        eprintln!(
                            "[cortex-mcp] Team mode daemon unreachable after {attempt} retries -- failing closed"
                        );
                        if is_write {
                            buffer_write(trimmed);
                        }
                        if has_id {
                            let err = serde_json::json!({
                                "jsonrpc": "2.0",
                                "error": { "code": -32000, "message": "Team mode: daemon unreachable" },
                                "id": msg.get("id")
                            });
                            let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                        break;
                    }

                    // Solo mode: buffer writes, keep retrying for reads
                    if !team_mode && is_write && attempt >= 3 {
                        buffer_write(trimmed);
                        if has_id {
                            let err = serde_json::json!({
                                "jsonrpc": "2.0",
                                "error": { "code": -32000, "message": "Daemon unreachable, write buffered" },
                                "id": msg.get("id")
                            });
                            let _ = stdout.write_all(format!("{}\n", err).as_bytes()).await;
                            let _ = stdout.flush().await;
                        }
                        break;
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
