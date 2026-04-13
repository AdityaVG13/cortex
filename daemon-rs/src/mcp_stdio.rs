// SPDX-License-Identifier: MIT
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::auth;
use crate::handlers::mcp::handle_mcp_message_with_caller;
use crate::state::RuntimeState;

/// Resolve MCP caller identity at startup.
///
/// In team mode, reads an API key from `CORTEX_API_KEY` env var (preferred) or
/// `~/.cortex/cortex.token` (fallback), then matches it against the team's
/// user hashes to resolve a user ID. Returns `None` in solo mode or if no
/// matching key is found (fail-closed via `is_visible`).
fn resolve_mcp_caller(state: &RuntimeState) -> Option<i64> {
    if !state.team_mode {
        return None;
    }

    let key = match std::env::var("CORTEX_API_KEY") {
        Ok(key) => Some(key),
        Err(std::env::VarError::NotPresent) => auth::read_token(),
        Err(e) => {
            eprintln!("[cortex-mcp] team mode: failed to read CORTEX_API_KEY: {e}");
            auth::read_token()
        }
    };

    let key = match key {
        Some(k) if k.starts_with("ctx_") => k,
        _ => {
            eprintln!("[cortex-mcp] team mode: no ctx_ API key found, MCP caller unidentified");
            return None;
        }
    };

    let hashes = match state.team_api_key_hashes.read() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[cortex-mcp] WARNING: lock poisoned reading API key hashes: {e}");
            return None;
        }
    };
    for (user_id, hash) in hashes.iter() {
        if auth::verify_api_key_argon2id(&key, hash) {
            eprintln!("[cortex-mcp] team mode: authenticated as user {user_id}");
            return Some(*user_id);
        }
    }

    eprintln!("[cortex-mcp] team mode: API key did not match any user");
    None
}

/// Run the MCP stdio transport.
///
/// Reads newline-delimited JSON-RPC 2.0 messages from stdin, dispatches each
/// to the MCP handler, and writes responses to stdout.  All logging goes to
/// stderr so the stdout channel is never contaminated with non-JSON-RPC data.
///
/// In team mode, caller identity is resolved once at startup from the API key
/// and passed to every dispatched request.
///
/// This function blocks until stdin is closed (i.e. the Claude Code session ends).
pub async fn run(state: RuntimeState) {
    let caller_id = resolve_mcp_caller(&state);
    if state.team_mode && caller_id.is_none() {
        eprintln!(
            "[cortex-mcp] Team mode requires a valid ctx_ API key (CORTEX_API_KEY or cortex.token). Refusing to start anonymous stdio session."
        );
        return;
    }
    eprintln!(
        "[cortex-mcp] MCP stdio transport started (caller_id: {})",
        caller_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".to_string())
    );

    let stdin = tokio::io::stdin();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    loop {
        match lines.next_line().await {
            Ok(Some(raw_line)) => {
                let line = raw_line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                let msg: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[cortex-mcp] JSON parse error: {e}");
                        write_stdout(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "error": { "code": -32700, "message": "Parse error" },
                            "id": null
                        }));
                        continue;
                    }
                };

                let has_id = msg.get("id").is_some();

                let response = handle_mcp_message_with_caller(&state, &msg, caller_id, None).await;

                if has_id {
                    if let Some(resp) = response {
                        write_stdout(&resp);
                    }
                }
            }
            Ok(None) => {
                eprintln!("[cortex-mcp] stdin closed, exiting");
                break;
            }
            Err(e) => {
                eprintln!("[cortex-mcp] stdin read error: {e}");
                break;
            }
        }
    }
}

/// Write a JSON value followed by a newline to stdout.
///
/// Uses `std::io::Write` directly on the locked stdout handle to avoid any
/// buffering or redirection that could corrupt the JSON-RPC stream.
fn write_stdout(value: &Value) {
    use std::io::Write;

    let json = match serde_json::to_string(value) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[cortex-mcp] Failed to serialise response: {e}");
            return;
        }
    };

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    if let Err(e) = handle.write_all(json.as_bytes()) {
        eprintln!("[cortex-mcp] stdout write error: {e}");
        return;
    }
    if let Err(e) = handle.write_all(b"\n") {
        eprintln!("[cortex-mcp] stdout newline error: {e}");
        return;
    }
    if let Err(e) = handle.flush() {
        eprintln!("[cortex-mcp] stdout flush error: {e}");
    }
}
