//! Operator and trusted-adapter surface for the automatic observation cycle.
use super::common::{parse_flag_value, validate_cli_options};
use crate::{
    auth::CortexPaths,
    runtime::{
        CortexRuntime,
        cycle::NeedSpec,
        host_capture::{HostCaptureContext, HostCaptureGrant, HostOrigin, HostOriginBinding},
        observation::MAX_CAPTURE_BYTES,
    },
};
use asupersync::Cx;
use serde_json::json;
use std::io::{Read, Write};

fn input() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::io::stdin().lock().take(MAX_CAPTURE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err("capture_batch_byte_limit".into());
    }
    Ok(bytes)
}

pub async fn run_cycle_cli(cx: &Cx, paths: &CortexPaths, args: &[String]) -> Result<(), String> {
    let command = args.first().map(String::as_str).ok_or("missing_capture_command")?;
    let flags = &args[1..];
    let values: &[&str] = match command {
        "inventory" => &["--revision"],
        "bootstrap" => &["--revision", "--max-sources", "--max-bytes"],
        "subscribe" | "host-register" => &[],
        "prepare" => &["--id", "--context", "--present"],
        "retract" => &["--id", "--reason"],
        "learn" | "learn-reset" | "rebuild" => &["--scope"],
        "query" => &["--scope", "--query"],
        "learn-explain" => &["--scope", "--query"],
        "assess" => &["--scope", "--event-key", "--id", "--assessment"],
        "unassess" => &["--scope", "--event-key"],
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
        _ => return Err("unknown_capture_command".into()),
    };
    validate_cli_options(flags, values, if matches!(command, "prepare" | "host-cycle") { &["--quiet", "--payload"] } else { &["--quiet"] })?;
    let required = |key: &str| parse_flag_value(flags, key).filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("Missing value for {key}"));
    let runtime = CortexRuntime::open(paths).map_err(|e| e.to_string())?;
    let output = match command {
        "inventory" => serde_json::to_value(if let Some(revision) = parse_flag_value(flags, "--revision") {
            runtime.read_inventory(cx, &revision).await?
        } else {
            runtime.inventory_sources(cx).await?
        }),
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
                    std::io::stdout().lock().write_all(view.payload.as_bytes()).map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
            serde_json::to_value(view)
        }
        "retract" => {
            runtime.retract_observation(cx, &required("--id")?, &required("--reason")?).await?;
            Ok(json!({"status":"retracted"}))
        }
        "query" => serde_json::to_value(runtime.query_observations(cx, &required("--scope")?, &required("--query")?, 32, 65536, false).await?),
        "rebuild" => Ok(json!({"projected":runtime.rebuild_observation_projection(cx,&required("--scope")?).await?})),
        "learn" => Ok(json!({"sources":runtime.rebuild_associations(cx,&required("--scope")?).await?})),
        "learn-reset" => {
            runtime.reset_associations(cx, &required("--scope")?).await?;
            Ok(json!({"status":"disabled"}))
        }
        "learn-explain" => serde_json::to_value(runtime.explain_associations(cx, &required("--scope")?, &[required("--query")?], 32).await?),
        "assess" => {
            use crate::runtime::associations::AssociationAssessment;
            let assessment = match required("--assessment")?.as_str() {
                "useful" => AssociationAssessment::Useful,
                "harmful" => AssociationAssessment::Harmful,
                "neutral" => AssociationAssessment::Neutral,
                _ => return Err("invalid_assessment".into()),
            };
            Ok(json!({"recorded":runtime.record_association_feedback(cx,&required("--scope")?,&required("--event-key")?,&required("--id")?,assessment).await?}))
        }
        "unassess" => {
            runtime.retract_association_feedback(cx, &required("--scope")?, &required("--event-key")?).await?;
            Ok(json!({"status":"retracted"}))
        }
        "host-register" => {
            let grant: HostCaptureGrant = serde_json::from_slice(&input()?).map_err(|e| e.to_string())?;
            runtime.register_host_capture(cx, grant).await?;
            Ok(json!({"status":"registered"}))
        }
        "host-put" | "host-tail" | "host-cycle" => {
            // Origin sidecars are explicit trusted CLI arguments, never parsed from hook stdin.
            let sidecar: std::collections::BTreeMap<String, String> = serde_json::from_str(&required("--origins")?).map_err(|e| e.to_string())?;
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
                                println!("{}", json!({"hookSpecificOutput":{"hookEventName":host["hook_event_name"],"additionalContext":view.payload}}));
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
        println!("{}", output);
    }
    Ok(())
}
