use super::super::common::parse_flag_value;
use super::input;
use crate::auth::CortexPaths;
use crate::runtime::{
    CortexRuntime,
    observation::{MAX_CAPTURE_BYTES, ObservationEvent, ObservationRole, SourceSpec},
};
use asupersync::Cx;
use serde_json::{Value, json};
use std::path::Path;

/// Canonicalize the parent so a not-yet-created file still uses the real volume.
fn canonical_file_source_key(source: &str) -> Result<String, String> {
    let Some(raw) = source.strip_prefix("file:") else {
        return Ok(source.to_string());
    };
    let path = Path::new(raw);
    let (name, parent) = match (path.file_name(), path.parent()) {
        (Some(name), Some(parent)) if path.is_absolute() && !parent.as_os_str().is_empty() => (name, parent),
        _ => return Err("source_not_authorized".into()),
    };
    match parent.canonicalize() {
        Ok(parent) => Ok(format!("file:{}", parent.join(name).display())),
        Err(_) => Ok(source.to_string()),
    }
}

enum SourceAction {
    Register(SourceSpec),
    SetEnabled(bool),
    Put { generation: String, event: ObservationEvent },
    Tail { generation: String, offset: u64, bytes: Vec<u8> },
    File,
    Get,
}

fn registration(source: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>) -> Result<SourceSpec, String> {
    let mut spec = SourceSpec::document(source, required("--scope")?);
    let role = parse_flag_value(flags, "--role");
    let raw = role.as_deref().unwrap_or("document");
    // CLI role names stay exact, unlike the whitespace-tolerant library parser.
    spec.role = ObservationRole::parse(raw).filter(|role| role.as_str() == raw).ok_or("Invalid --role")?;
    if let Some(limit) = parse_flag_value(flags, "--max-bytes") {
        spec.max_bytes = limit.parse().map_err(|_| "Invalid --max-bytes")?;
        if spec.max_bytes == 0 || spec.max_bytes > MAX_CAPTURE_BYTES {
            return Err("Invalid --max-bytes".into());
        }
    }
    if spec.key.len() > 1024 || spec.scope.len() > 1024 {
        return Err("invalid_source_identity".into());
    }
    Ok(spec)
}

impl SourceAction {
    fn parse(command: &str, source: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>) -> Result<Self, String> {
        Ok(match command {
            "register" => Self::Register(registration(source, flags, required)?),
            "enable" | "disable" => Self::SetEnabled(command == "enable"),
            "put" => {
                let generation = required("--generation")?;
                let event = serde_json::from_slice(&input()?).map_err(|e| format!("Invalid observation JSON: {e}"))?;
                Self::Put { generation, event }
            }
            "tail" => {
                let generation = required("--generation")?;
                let offset = required("--offset")?.parse().map_err(|_| "Invalid --offset")?;
                Self::Tail { generation, offset, bytes: input()? }
            }
            "file" => Self::File,
            "get" => Self::Get,
            _ => unreachable!(),
        })
    }
}

pub(super) async fn run_source_command(
    cx: &Cx, paths: &CortexPaths, command: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    let key = match command {
        "get" => "--id",
        "file" => "--path",
        _ => "--source",
    };
    let source = required(key)?;
    let source = if matches!(command, "get" | "file") { source } else { canonical_file_source_key(&source)? };
    // Validate and read input before opening the database. No impossible
    // combinations of optional generation, offset, event, and spec remain.
    let action = SourceAction::parse(command, &source, flags, required)?;
    if let SourceAction::Register(_) = &action {
        std::fs::create_dir_all(&paths.home).map_err(|e| e.to_string())?;
        if let Some(parent) = paths.db.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let runtime = CortexRuntime::open(paths).map_err(|e| e.to_string())?;
    match action {
        SourceAction::Register(spec) => {
            runtime.register_source(cx, spec).await?;
            Ok(json!({"status":"registered","source":source}))
        }
        SourceAction::SetEnabled(enabled) => {
            runtime.set_source_enabled(cx, &source, enabled).await?;
            Ok(json!({"status":if enabled {"enabled"} else {"disabled"},"source":source}))
        }
        SourceAction::Put { generation, event } => serde_json::to_value(runtime.observe(cx, &source, &generation, event).await?),
        SourceAction::Tail { generation, offset, bytes } => serde_json::to_value(runtime.tail_observations(cx, &source, &generation, offset, &bytes).await?),
        SourceAction::File => serde_json::to_value(runtime.observe_file(cx, Path::new(&source)).await?),
        SourceAction::Get => serde_json::to_value(runtime.read_observation(cx, &source).await?),
    }
    .map_err(|e: serde_json::Error| e.to_string())
}

pub(super) async fn run_inventory_command(
    cx: &Cx, runtime: &CortexRuntime, command: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Value, String> {
    match command {
        "inventory" => serde_json::to_value(if let Some(revision) = parse_flag_value(flags, "--revision") {
            runtime.read_inventory(cx, &revision).await?
        } else {
            runtime.inventory_sources(cx).await?
        }),
        "reconcile" => serde_json::to_value(runtime.reconcile_inventory(cx, &required("--revision")?).await?),
        "bootstrap" => {
            let revision = required("--revision")?;
            let sources = required("--max-sources")?.parse().map_err(|_| "invalid_source_budget")?;
            let bytes = required("--max-bytes")?.parse().map_err(|_| "invalid_byte_budget")?;
            serde_json::to_value(runtime.bootstrap_inventory(cx, &revision, sources, bytes).await?)
        }
        _ => unreachable!(),
    }
    .map_err(|e: serde_json::Error| e.to_string())
}
