//! `cortex op <operation> [--args '<json>'] [--agent name]`: run one of the
//! eight operations against the local brain in-process. No daemon, HTTP or
//! token is required; the process's own identity is the principal.

use super::common::{parse_flag_usize, validate_cli_options_or_exit};
use crate::auth;
use cortex_kernel::handlers::operations::{Caller, Operation, dispatch};
use crate::runtime::CortexRuntime;

pub async fn run_op_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, args: &[String]) {
    let Some(operation) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("Usage: cortex op <capabilities|orient|query|expand|commit|checkpoint|resolve|feedback> [--args '<json>'] [--agent <name>]");
        std::process::exit(2);
    };
    validate_cli_options_or_exit(&args[1..], &["--args", "--agent", "--home", "--db"], &[]);
    let Some(op) = Operation::from_tool_name(operation) else {
        eprintln!("[cortex] unknown operation `{operation}`");
        std::process::exit(2);
    };
    let raw_args = args.iter().position(|a| a == "--args").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "{}".into());
    let agent = args.iter().position(|a| a == "--agent").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "cli".into());
    let parsed: serde_json::Value = match serde_json::from_str(&raw_args) {
        Ok(v) => v,
        Err(err) => {
            println!("{}", serde_json::json!({"status": "invalid_request", "error": format!("--args is not JSON: {err}")}));
            std::process::exit(2);
        }
    };
    let runtime = match CortexRuntime::open(paths) {
        Ok(r) => r,
        Err(err) => {
            println!("{}", serde_json::json!({"status": "unavailable", "error": err.to_string()}));
            std::process::exit(1);
        }
    };
    let state = runtime.state();
    let caller = Caller {
        owner_id: state.default_owner_id,
        agent: &agent,
        principal: state.default_owner_id.map(|id| format!("user:{id}")).unwrap_or_else(|| "solo".into()),
    };
    match dispatch(cx, state, caller, op, &parsed).await {
        Ok(payload) => println!("{}", serde_json::to_string(&payload).unwrap_or_default()),
        Err(err) => {
            println!("{}", serde_json::json!({"status": "unavailable", "error": err.to_string()}));
            std::process::exit(1);
        }
    }
}

/// `cortex maintain [--jobs N]`: drain a bounded slice of durable
/// maintenance debt in-process and print the debt afterwards.
pub async fn run_maintain_cli(cx: &asupersync::Cx, paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--jobs", "--home", "--db"], &["--json"]);
    let jobs = match parse_flag_usize(args, "--jobs") {
        Ok(Some(value)) => value.clamp(1, 4096),
        Ok(None) => 32,
        Err(err) => {
            eprintln!("[cortex] {err}");
            std::process::exit(2);
        }
    };
    let runtime = match CortexRuntime::open(paths) {
        Ok(r) => r,
        Err(err) => {
            println!("{}", serde_json::json!({"status": "unavailable", "error": err.to_string()}));
            std::process::exit(1);
        }
    };
    let conn = match runtime.state().db.lock(cx).await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("[cortex] {err}");
            std::process::exit(1);
        }
    };
    let _ = crate::db::records::ensure_authoritative_schema(&conn);
    let result = crate::db::outbox::maintain_slice(&conn, "cli", jobs);
    match result {
        Ok(slice) => println!("{}", serde_json::json!({"status": "ok", "slice": slice, "debt": crate::db::outbox::debt(&conn).to_json()})),
        Err(err) => {
            println!("{}", serde_json::json!({"status": "unavailable", "error": err.to_string()}));
            std::process::exit(1);
        }
    }
}
