//! MCP stdio edge over one in-process brain. Blocking stdio belongs to this
//! dedicated host process; memory operations run on its asupersync runtime.
use crate::CortexRuntime;
use crate::handlers::{SourceIdentity, mcp::handle_mcp_message_with_caller};
use asupersync::Cx;
use serde_json::Value;
use std::io::{BufRead, Read, Write};

pub async fn answer_line(cx: &Cx, runtime: &CortexRuntime, line: &str, agent: &str) -> Option<Value> {
    let msg = match serde_json::from_str::<Value>(line) {
        Ok(msg) => msg,
        Err(_) => return Some(crate::handlers::mcp::mcp_error(Value::Null, -32700, "Parse error")),
    };
    let source = SourceIdentity { agent: agent.into(), model: None };
    handle_mcp_message_with_caller(cx, runtime.state(), &msg, runtime.state().default_owner_id, Some(&source)).await
}

pub async fn run(cx: &Cx, paths: &crate::auth::CortexPaths, agent: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = CortexRuntime::open(paths)?;
    let input = std::io::stdin();
    let mut input = input.lock();
    let output = std::io::stdout();
    let mut output = output.lock();
    let mut line = String::new();
    loop {
        cx.checkpoint().map_err(|err| err.to_string())?;
        line.clear();
        let mut bounded = input.by_ref().take(2 * 1024 * 1024 + 1);
        if bounded.read_line(&mut line)? == 0 {
            break;
        }
        if line.len() > 2 * 1024 * 1024 {
            return Err("MCP message exceeds 2 MiB".into());
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = answer_line(cx, &runtime, &line, agent.unwrap_or("mcp")).await {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}
