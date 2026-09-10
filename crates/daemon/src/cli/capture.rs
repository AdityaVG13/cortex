//! Operator-managed V5 capture adapter. Stdin contains normalized observations,
//! not grants, credentials or filesystem paths to crawl.
use super::common::{parse_flag_value, validate_cli_options};
use crate::{
    auth::CortexPaths,
    runtime::{
        CortexRuntime,
        observation::{MAX_CAPTURE_BYTES, ObservationEvent, ObservationRole, SourceSpec},
    },
};
use asupersync::Cx;
use serde_json::json;
use std::io::Read;

pub async fn run_capture_cli(cx: &Cx, paths: &CortexPaths, args: &[String]) -> Result<(), String> {
    let command = args.first().map(String::as_str).ok_or("Usage: cortex capture <register|enable|disable|put|tail|file|get>")?;
    if matches!(
        command,
        "assess"
            | "unassess"
            | "inventory"
            | "bootstrap"
            | "subscribe"
            | "prepare"
            | "query"
            | "rebuild"
            | "retract"
            | "learn"
            | "learn-reset"
            | "learn-explain"
            | "host-register"
            | "host-put"
            | "host-tail"
            | "host-cycle"
    ) {
        return super::cycle::run_cycle_cli(cx, paths, args).await;
    }
    let flags = &args[1..];
    let required = |key: &str| -> Result<String, String> {
        parse_flag_value(flags, key).filter(|v| !v.trim().is_empty()).ok_or_else(|| format!("Missing value for {key}"))
    };
    let values: &[&str] = match command {
        "register" => &["--source", "--scope", "--role", "--max-bytes"],
        "enable" | "disable" => &["--source"],
        "put" => &["--source", "--generation"],
        "tail" => &["--source", "--generation", "--offset"],
        "file" => &["--path"],
        "get" => &["--id"],
        _ => return Err("Usage: cortex capture <register|enable|disable|put|tail|file|get>".into()),
    };
    validate_cli_options(flags, values, &["--quiet"])?;
    let source = match command {
        "get" => required("--id")?,
        "file" => required("--path")?,
        _ => required("--source")?,
    };
    let generation = if matches!(command, "put" | "tail") { Some(required("--generation")?) } else { None };
    let offset = if command == "tail" { Some(required("--offset")?.parse::<u64>().map_err(|_| "Invalid --offset")?) } else { None };
    let spec = if command == "register" {
        let mut spec = SourceSpec::document(&source, required("--scope")?);
        spec.role = match parse_flag_value(flags, "--role").as_deref().unwrap_or("document") {
            "document" => ObservationRole::Document,
            "user_statement" => ObservationRole::UserStatement,
            "agent_assertion" => ObservationRole::AgentAssertion,
            "tool_report" => ObservationRole::ToolReport,
            "delivery_only" => ObservationRole::DeliveryOnly,
            _ => return Err("Invalid --role".into()),
        };
        if let Some(limit) = parse_flag_value(flags, "--max-bytes") {
            spec.max_bytes = limit.parse().map_err(|_| "Invalid --max-bytes")?;
            if spec.max_bytes == 0 || spec.max_bytes > MAX_CAPTURE_BYTES {
                return Err("Invalid --max-bytes".into());
            }
        }
        Some(spec)
    } else {
        None
    };
    let input = if matches!(command, "put" | "tail") {
        let mut input = Vec::new();
        std::io::stdin().lock().take(MAX_CAPTURE_BYTES as u64 + 1).read_to_end(&mut input).map_err(|err| err.to_string())?;
        if input.len() > MAX_CAPTURE_BYTES {
            return Err("capture_batch_byte_limit".into());
        }
        input
    } else {
        Vec::new()
    };
    let event = if command == "put" {
        Some(serde_json::from_slice::<ObservationEvent>(&input).map_err(|err| format!("Invalid observation JSON: {err}"))?)
    } else {
        None
    };
    if let Some(spec) = &spec {
        if spec.key.len() > 1024 || spec.scope.len() > 1024 {
            return Err("invalid_source_identity".into());
        }
        std::fs::create_dir_all(&paths.home).map_err(|err| err.to_string())?;
        if let Some(parent) = paths.db.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
        }
    }
    let runtime = CortexRuntime::open(paths).map_err(|err| err.to_string())?;
    let output = match command {
        "register" => {
            runtime.register_source(cx, spec.expect("validated source registration")).await?;
            json!({"status":"registered","source":source})
        }
        "enable" | "disable" => {
            runtime.set_source_enabled(cx, &source, command == "enable").await?;
            json!({"status":if command=="enable" {"enabled"} else {"disabled"},"source":source})
        }
        "put" => serde_json::to_value(
            runtime
                .observe(cx, &source, generation.as_deref().expect("validated generation"), event.expect("validated observation"))
                .await?,
        )
        .map_err(|err| err.to_string())?,
        "tail" => serde_json::to_value(
            runtime
                .tail_observations(cx, &source, generation.as_deref().expect("validated generation"), offset.expect("validated offset"), &input)
                .await?,
        )
        .map_err(|err| err.to_string())?,
        "file" => serde_json::to_value(runtime.observe_file(cx, std::path::Path::new(&source)).await?).map_err(|err| err.to_string())?,
        "get" => serde_json::to_value(runtime.read_observation(cx, &source).await?).map_err(|err| err.to_string())?,
        _ => unreachable!(),
    };
    if !flags.iter().any(|arg| arg == "--quiet") {
        println!("{output}");
    }
    Ok(())
}
