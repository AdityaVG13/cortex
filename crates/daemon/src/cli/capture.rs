//! Operator-managed capture adapter. Stdin contains normalized observations
//! or trusted host grants, not credentials or filesystem paths to crawl.
use super::common::{parse_flag_value, parse_flag_values, validate_cli_options};
use crate::{
    auth::CortexPaths,
    runtime::{
        CortexRuntime,
        assembly::AssemblySpec,
        associations::AssociationAssessment,
        cycle::NeedSpec,
        host_capture::{HostCaptureContext, HostCaptureGrant, HostOrigin, HostOriginBinding},
        observation::{MAX_CAPTURE_BYTES, ObservationEvent, ObservationRole, SourceSpec},
    },
};
use asupersync::Cx;
use serde_json::json;
use std::io::{Read, Write};

const USAGE: &str = "Usage: cortex capture <register|enable|disable|put|tail|file|get|inventory|bootstrap|reconcile|subscribe|prepare|query|require|retract|rebuild|learn|learn-reset|learn-explain|assess|unassess|assemble|assembly-get|assembly-expand|learn-event|learn-retract|learn-erase|routes-rebuild|routes-reset|why|host-register|host-put|host-tail|host-cycle>";

fn input() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_CAPTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err("capture_batch_byte_limit".into());
    }
    Ok(bytes)
}

pub async fn run_capture_cli(cx: &Cx, paths: &CortexPaths, args: &[String]) -> Result<(), String> {
    let command = args.first().map(String::as_str).ok_or(USAGE)?;
    let flags = &args[1..];
    let values: &[&str] = match command {
        "register" => &["--source", "--scope", "--role", "--max-bytes"],
        "enable" | "disable" => &["--source"],
        "put" => &["--source", "--generation"],
        "tail" => &["--source", "--generation", "--offset"],
        "file" => &["--path"],
        "get" => &["--id"],
        "inventory" => &["--revision"],
        "reconcile" => &["--revision"],
        "require" => &["--parent", "--child"],
        "bootstrap" => &["--revision", "--max-sources", "--max-bytes"],
        "subscribe" | "host-register" => &[],
        "prepare" => &["--id", "--context", "--present"],
        "retract" => &["--id", "--reason"],
        "learn" | "learn-reset" | "rebuild" => &["--scope"],
        "query" => &["--scope", "--query", "--path"],
        "learn-explain" => &["--scope", "--query"],
        "assess" => &["--scope", "--event-key", "--id", "--assessment"],
        "unassess" => &["--scope", "--event-key"],
        "assemble" | "learn-event" => &[],
        "assembly-get" | "assembly-expand" => &["--id"],
        "learn-retract" => &["--origin", "--event", "--reason"],
        "learn-erase" => &["--source"],
        "routes-rebuild" | "routes-reset" => &["--scope"],
        "why" => &["--scope", "--query"],
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
    };
    validate_cli_options(
        flags,
        values,
        if matches!(command, "prepare" | "host-cycle") {
            &["--quiet", "--payload"]
        } else {
            &["--quiet"]
        },
    )?;
    let required = |key: &str| -> Result<String, String> {
        parse_flag_value(flags, key)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| format!("Missing value for {key}"))
    };
    if matches!(
        command,
        "register" | "enable" | "disable" | "put" | "tail" | "file" | "get"
    ) {
        return run_source_command(cx, paths, command, flags, &required).await;
    }
    let runtime = CortexRuntime::open(paths).map_err(|e| e.to_string())?;
    let output = match command {
        "inventory" => serde_json::to_value(if let Some(revision) = parse_flag_value(flags, "--revision") {
            runtime.read_inventory(cx, &revision).await?
        } else {
            runtime.inventory_sources(cx).await?
        }),
        "reconcile" => serde_json::to_value(runtime.reconcile_inventory(cx, &required("--revision")?).await?),
        "require" => {
            runtime
                .require_observation(cx, &required("--parent")?, &required("--child")?)
                .await?;
            Ok(json!({"status":"required"}))
        }
        "bootstrap" => {
            let revision = required("--revision")?;
            let sources = required("--max-sources")?.parse().map_err(|_| "invalid_source_budget")?;
            let bytes = required("--max-bytes")?.parse().map_err(|_| "invalid_byte_budget")?;
            serde_json::to_value(runtime.bootstrap_inventory(cx, &revision, sources, bytes).await?)
        }
        "subscribe" => {
            let spec: NeedSpec = serde_json::from_slice(&input()?).map_err(|e| e.to_string())?;
            serde_json::to_value(runtime.subscribe_observations(cx, spec).await?)
        }
        "prepare" => {
            let view = runtime
                .prepare_observations(cx, &required("--id")?, &required("--context")?, parse_flag_value(flags, "--present").as_deref())
                .await?;
            if flags.iter().any(|s| s == "--payload") {
                if view.status != "ready" {
                    return Err(view.status);
                }
                if !flags.iter().any(|s| s == "--quiet") {
                    std::io::stdout()
                        .lock()
                        .write_all(view.payload.as_bytes())
                        .map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            serde_json::to_value(view)
        }
        "retract" => {
            runtime
                .retract_observation(cx, &required("--id")?, &required("--reason")?)
                .await?;
            Ok(json!({"status":"retracted"}))
        }
        "query" => {
            let query = required("--query")?;
            let paths = parse_flag_values(flags, "--path");
            let extra = parse_flag_value(flags, "--scope").filter(|s| !s.trim().is_empty());
            serde_json::to_value(if paths.is_empty() {
                runtime
                    .query_observations(cx, &required("--scope")?, &query, 32, 65536, false)
                    .await?
            } else {
                runtime
                    .query_observations_for_paths(
                        cx,
                        &query,
                        &paths,
                        extra.as_deref(),
                        32,
                        65536,
                        false,
                    )
                    .await?
            })
        }
        "rebuild" => Ok(json!({"projected":runtime.rebuild_observation_projection(cx,&required("--scope")?).await?})),
        "learn" => Ok(json!({"sources":runtime.rebuild_associations(cx,&required("--scope")?).await?})),
        "learn-reset" => {
            runtime.reset_associations(cx, &required("--scope")?).await?;
            Ok(json!({"status":"disabled"}))
        }
        "learn-explain" => serde_json::to_value(runtime.explain_associations(cx, &required("--scope")?, &[required("--query")?], 32).await?),
        "assess" => {
            let assessment = match required("--assessment")?.as_str() {
                "useful" => AssociationAssessment::Useful,
                "harmful" => AssociationAssessment::Harmful,
                "neutral" => AssociationAssessment::Neutral,
                _ => return Err("invalid_assessment".into()),
            };
            Ok(json!({"recorded":runtime.record_association_feedback(cx,&required("--scope")?,&required("--event-key")?,&required("--id")?,assessment).await?}))
        }
        "unassess" => {
            runtime
                .retract_association_feedback(cx, &required("--scope")?, &required("--event-key")?)
                .await?;
            Ok(json!({"status":"retracted"}))
        }
        "assemble" => {
            let spec: AssemblySpec = serde_json::from_slice(&input()?).map_err(|e| e.to_string())?;
            serde_json::to_value(runtime.put_assembly(cx, spec).await?)
        }
        "assembly-get" => serde_json::to_value(runtime.get_assembly(cx, &required("--id")?).await?),
        "assembly-expand" => serde_json::to_value(runtime.expand_assembly(cx, &required("--id")?).await?),
        "learn-event" => {
            let event: cortex_kernel::assembly::LearningEvent =
                serde_json::from_slice(&input()?).map_err(|e| e.to_string())?;
            Ok(json!({"recorded": runtime.record_learning_event(cx, event).await?}))
        }
        "learn-retract" => {
            runtime
                .retract_learning_event(cx, &required("--origin")?, &required("--event")?, &required("--reason")?)
                .await?;
            Ok(json!({"status":"retracted"}))
        }
        "learn-erase" => Ok(json!({"retracted": runtime.erase_learning_source(cx, &required("--source")?).await?})),
        "routes-rebuild" => Ok(json!({"edges": runtime.rebuild_assembly_routes(cx, &required("--scope")?).await?})),
        "routes-reset" => {
            runtime.reset_assembly_routes(cx, &required("--scope")?).await?;
            Ok(json!({"status":"disabled"}))
        }
        "why" => serde_json::to_value(
            runtime
                .explain_assembly_routes(cx, &required("--scope")?, &[required("--query")?], 8)
                .await?,
        ),
        "host-register" => {
            let grant: HostCaptureGrant = serde_json::from_slice(&input()?).map_err(|e| e.to_string())?;
            runtime.register_host_capture(cx, grant).await?;
            Ok(json!({"status":"registered"}))
        }
        "host-put" | "host-tail" | "host-cycle" => {
            let sidecar: std::collections::BTreeMap<String, String> =
                serde_json::from_str(&required("--origins")?).map_err(|e| e.to_string())?;
            if sidecar.len() > 128 {
                return Err("host_origin_limit".into());
            }
            let origins = sidecar
                .into_iter()
                .map(|(event_key, origin)| {
                    Ok(HostOriginBinding {
                        event_key,
                        origin: match origin.as_str() {
                            "external" => HostOrigin::External,
                            "cortex_delivery" => HostOrigin::CortexDelivery,
                            _ => return Err("invalid_host_origin"),
                        },
                    })
                })
                .collect::<Result<Vec<_>, &str>>()?;
            let context = HostCaptureContext {
                host_version: required("--host-version")?,
                session_id: required("--session")?,
                generation: required("--generation")?,
                original_event_key: parse_flag_value(flags, "--event-key"),
                origins,
            };
            let grant = required("--grant")?;
            let raw = input()?;
            if command == "host-cycle" {
                let (receipt, view) = runtime
                    .capture_and_prepare_host(cx, &grant, &context, &raw, &required("--context")?, parse_flag_value(flags, "--present").as_deref())
                    .await?;
                if !flags.iter().any(|s| s == "--quiet") {
                    if flags.iter().any(|s| s == "--payload") {
                        if let Some(view) = view {
                            if view.status != "ready" {
                                return Err(view.status);
                            }
                            if !view.payload.is_empty() {
                                let host: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
                                println!(
                                    "{}",
                                    json!({"hookSpecificOutput":{"hookEventName":host["hook_event_name"],"additionalContext":view.payload}})
                                );
                            }
                        }
                    } else {
                        println!("{}", json!({"capture":receipt,"view":view}));
                    }
                }
                return Ok(());
            }
            serde_json::to_value(if command == "host-tail" {
                runtime
                    .tail_host_transcript(cx, &grant, &context, required("--offset")?.parse().map_err(|_| "invalid_source_cursor")?, &raw)
                    .await?
            } else {
                runtime.capture_host_event(cx, &grant, &context, &raw).await?
            })
        }
        _ => unreachable!(),
    }
    .map_err(|e: serde_json::Error| e.to_string())?;
    if !flags.iter().any(|s| s == "--quiet") {
        println!("{output}");
    }
    Ok(())
}

async fn run_source_command(
    cx: &Cx,
    paths: &CortexPaths,
    command: &str,
    flags: &[String],
    required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<(), String> {
    let source = match command {
        "get" => required("--id")?,
        "file" => required("--path")?,
        _ => required("--source")?,
    };
    let generation = if matches!(command, "put" | "tail") {
        Some(required("--generation")?)
    } else {
        None
    };
    let offset = if command == "tail" {
        Some(required("--offset")?.parse::<u64>().map_err(|_| "Invalid --offset")?)
    } else {
        None
    };
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
        input()?
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
            runtime
                .register_source(cx, spec.expect("validated source registration"))
                .await?;
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
