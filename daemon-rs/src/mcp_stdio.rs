use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::handlers::mcp::handle_mcp_message;
use crate::state::RuntimeState;

/// Run the MCP stdio transport.
///
/// Reads newline-delimited JSON-RPC 2.0 messages from stdin, dispatches each
/// to the MCP handler, and writes responses to stdout.  All logging goes to
/// stderr so the stdout channel is never contaminated with non-JSON-RPC data.
///
/// This function blocks until stdin is closed (i.e. the Claude Code session ends).
pub async fn run(state: RuntimeState) {
    eprintln!("[cortex-mcp] MCP stdio transport started");

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

                // Only respond if the message has an id (requests, not notifications)
                let has_id = msg.get("id").is_some();

                let response = handle_mcp_message(&state, &msg).await;

                if has_id {
                    if let Some(resp) = response {
                        write_stdout(&resp);
                    }
                }
                // Notifications (no id) get no response — we still dispatch them
                // for side-effects (e.g. notifications/initialized) but send nothing.
            }
            Ok(None) => {
                // stdin closed — Claude Code session ended
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
    let _ = handle.flush();
}
