//! Operator-managed capture adapter. Stdin contains normalized observations
//! or trusted host grants, not credentials or filesystem paths to crawl.
use super::common::{parse_flag_value, parse_flag_values, validate_cli_options};
use crate::{
    auth::CortexPaths,
    runtime::{CortexRuntime, cycle::NeedSpec, observation::MAX_CAPTURE_BYTES},
};
use asupersync::Cx;
use cortex_logic::protocol::nonempty_owned;
use serde_json::{Value, json};
use std::io::{Read, Write};

#[path = "capture/host.rs"]
mod host;
#[path = "capture/learning.rs"]
mod learning;
#[path = "capture/source.rs"]
mod source;

const USAGE: &str = "Usage: cortex capture <register|enable|disable|put|tail|file|get|inventory|bootstrap|reconcile|subscribe|prepare|query|require|retract|rebuild|learn|learn-reset|learn-explain|assess|unassess|assemble|assembly-get|assembly-expand|learn-event|learn-retract|learn-erase|routes-rebuild|routes-reset|why|host-register|host-put|host-tail|host-cycle>";

pub(super) fn input() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::stdin().lock().take(MAX_CAPTURE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err("capture_batch_byte_limit".into());
    }
    Ok(bytes)
}

pub(super) fn json_input<T: serde::de::DeserializeOwned>() -> Result<T, String> {
    serde_json::from_slice(&input()?).map_err(|e| e.to_string())
}

fn command_value_flags(command: &str) -> Result<&'static [&'static str], String> {
    Ok(match command {
        "register" => &["--source", "--scope", "--role", "--max-bytes"],
        "enable" | "disable" => &["--source"],
        "put" => &["--source", "--generation"],
        "tail" => &["--source", "--generation", "--offset"],
        "file" => &["--path"],
        "get" => &["--id"],
        "inventory" | "reconcile" => &["--revision"],
        "require" => &["--parent", "--child"],
        "bootstrap" => &["--revision", "--max-sources", "--max-bytes"],
        "subscribe" | "host-register" | "assemble" | "learn-event" => &[],
        "prepare" => &["--id", "--context", "--present"],
        "retract" => &["--id", "--reason"],
        "learn" | "learn-reset" | "rebuild" | "routes-rebuild" | "routes-reset" => &["--scope"],
        "query" => &["--scope", "--query", "--path"],
        "learn-explain" | "why" => &["--scope", "--query"],
        "assess" => &["--scope", "--event-key", "--id", "--assessment"],
        "unassess" => &["--scope", "--event-key"],
        "assembly-get" | "assembly-expand" => &["--id"],
        "learn-retract" => &["--origin", "--event", "--reason"],
        "learn-erase" => &["--source"],
        "host-put" | "host-tail" | "host-cycle" => &[
            "--grant",
            "--host-version",
            "--session",
            "--generation",
            "--event-key",
            "--origins",
            "--offset",
            "--context",
            "--present",
        ],
        _ => return Err(USAGE.into()),
    })
}

pub async fn run_capture_cli(cx: &Cx, paths: &CortexPaths, args: &[String]) -> Result<(), String> {
    let args = super::common::without_global_value_flags(args);
    let command = args.first().map(String::as_str).ok_or(USAGE)?;
    let flags = &args[1..];
    let values = command_value_flags(command)?;
    validate_cli_options(flags, values, if matches!(command, "prepare" | "host-cycle") { &["--quiet", "--payload"] } else { &["--quiet"] })?;
    let required = |key: &str| -> Result<String, String> {
        parse_flag_value(flags, key).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("Missing value for {key}"))
    };
    let output = if matches!(command, "register" | "enable" | "disable" | "put" | "tail" | "file" | "get") {
        Some(source::run_source_command(cx, paths, command, flags, &required).await?)
    } else {
        let runtime = CortexRuntime::open(paths).map_err(|e| e.to_string())?;
        dispatch_runtime(cx, &runtime, command, flags, &required).await?
    };
    if let Some(output) = output.filter(|_| !flags.iter().any(|s| s == "--quiet")) {
        println!("{output}");
    }
    Ok(())
}

async fn dispatch_runtime(
    cx: &Cx, runtime: &CortexRuntime, command: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<Value>, String> {
    let output = match command {
        "inventory" | "reconcile" | "bootstrap" => source::run_inventory_command(cx, runtime, command, flags, required).await?,
        "learn" | "learn-reset" | "learn-explain" | "assess" | "unassess" => learning::associations(cx, runtime, command, required).await?,
        "assemble" | "assembly-get" | "assembly-expand" | "learn-event" | "learn-retract" | "learn-erase" | "routes-rebuild" | "routes-reset" | "why" => {
            learning::assemblies(cx, runtime, command, required).await?
        }
        "host-register" | "host-put" | "host-tail" | "host-cycle" => return host::run_host_command(cx, runtime, command, flags, required).await,
        "prepare" => return prepare(cx, runtime, flags, required).await,
        _ => run_cycle_command(cx, runtime, command, flags, required).await?,
    };
    Ok(Some(output))
}

async fn run_cycle_command(
    cx: &Cx, runtime: &CortexRuntime, command: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    match command {
        "subscribe" => serde_json::to_value(runtime.subscribe_observations(cx, json_input::<NeedSpec>()?).await?),
        "require" => {
            runtime.require_observation(cx, &required("--parent")?, &required("--child")?).await?;
            Ok(json!({"status":"required"}))
        }
        "retract" => {
            runtime.retract_observation(cx, &required("--id")?, &required("--reason")?).await?;
            Ok(json!({"status":"retracted"}))
        }
        "query" => {
            let query = required("--query")?;
            let paths = parse_flag_values(flags, "--path");
            let extra = parse_flag_value(flags, "--scope").and_then(nonempty_owned);
            serde_json::to_value(if paths.is_empty() {
                runtime.query_observations(cx, &required("--scope")?, &query, 32, 65536, false).await?
            } else {
                runtime.query_observations_for_paths(cx, &query, &paths, extra.as_deref(), 32, 65536, false).await?
            })
        }
        "rebuild" => Ok(json!({"projected":runtime.rebuild_observation_projection(cx, &required("--scope")?).await?})),
        _ => unreachable!(),
    }
    .map_err(|e: serde_json::Error| e.to_string())
}

async fn prepare(cx: &Cx, runtime: &CortexRuntime, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>) -> Result<Option<Value>, String> {
    let view = runtime
        .prepare_observations(cx, &required("--id")?, &required("--context")?, parse_flag_value(flags, "--present").as_deref())
        .await?;
    if !flags.iter().any(|s| s == "--payload") {
        return serde_json::to_value(view).map(Some).map_err(|e| e.to_string());
    }
    if view.status != "ready" {
        return Err(view.status);
    }
    if !flags.iter().any(|s| s == "--quiet") {
        std::io::stdout().lock().write_all(view.payload.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(None)
}
