use super::super::common::parse_flag_value;
use super::{input, json_input};
use crate::runtime::{
    CortexRuntime,
    host_capture::{HostCaptureContext, HostCaptureGrant, HostOrigin, HostOriginBinding},
};
use asupersync::Cx;
use serde_json::{Value, json};

fn origins(raw: &str) -> Result<Vec<HostOriginBinding>, String> {
    let sidecar: std::collections::BTreeMap<String, String> = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    if sidecar.len() > 128 {
        return Err("host_origin_limit".into());
    }
    sidecar
        .into_iter()
        .map(|(event_key, origin)| {
            Ok(HostOriginBinding {
                event_key,
                origin: match origin.as_str() {
                    "external" => HostOrigin::External,
                    "cortex_delivery" => HostOrigin::CortexDelivery,
                    _ => return Err("invalid_host_origin".into()),
                },
            })
        })
        .collect()
}

pub(super) async fn run_host_command(
    cx: &Cx, runtime: &CortexRuntime, command: &str, flags: &[String], required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<Value>, String> {
    if command == "host-register" {
        runtime.register_host_capture(cx, json_input::<HostCaptureGrant>()?).await?;
        return Ok(Some(json!({"status":"registered"})));
    }
    let origins = origins(&required("--origins")?)?;
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
        return run_host_cycle(cx, runtime, &grant, &context, &raw, flags, required).await;
    }
    let value = if command == "host-tail" {
        let offset = required("--offset")?.parse().map_err(|_| "invalid_source_cursor")?;
        serde_json::to_value(runtime.tail_host_transcript(cx, &grant, &context, offset, &raw).await?)
    } else {
        serde_json::to_value(runtime.capture_host_event(cx, &grant, &context, &raw).await?)
    };
    value.map(Some).map_err(|e| e.to_string())
}

async fn run_host_cycle(
    cx: &Cx, runtime: &CortexRuntime, grant: &str, context: &HostCaptureContext, raw: &[u8], flags: &[String],
    required: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<Value>, String> {
    let (receipt, view) = runtime
        .capture_and_prepare_host(cx, grant, context, raw, &required("--context")?, parse_flag_value(flags, "--present").as_deref())
        .await?;
    // Quiet host capture intentionally skips payload rendering and its errors.
    if flags.iter().any(|s| s == "--quiet") {
        return Ok(None);
    }
    if !flags.iter().any(|s| s == "--payload") {
        return Ok(Some(json!({"capture":receipt,"view":view})));
    }
    let Some(view) = view else {
        return Ok(None);
    };
    if view.status != "ready" {
        return Err(view.status);
    }
    if view.payload.is_empty() {
        return Ok(None);
    }
    let host: Value = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    Ok(Some(json!({"hookSpecificOutput":{"hookEventName":host["hook_event_name"],"additionalContext":view.payload}})))
}
