use crate::protocol::arg_str;
use crate::runtime::CortexRuntime;
use serde_json::{Value, json};

const MAX_CAPTURE_SIDECAR_BYTES: u64 = 256 * 1024;

/// Env `CORTEX_CAPTURE` wins. Otherwise the operator sidecar at
/// [`crate::auth::CortexPaths::capture_sidecar`]. Missing both is silent, not CQR.
pub fn load_capture_sidecar(paths: &crate::auth::CortexPaths) -> Option<String> {
    match std::env::var("CORTEX_CAPTURE") {
        Ok(value) if !value.trim().is_empty() => {
            if value.len() as u64 > MAX_CAPTURE_SIDECAR_BYTES {
                return None;
            }
            Some(value)
        }
        _ => read_capture_sidecar_file(&paths.capture_sidecar()),
    }
}

fn read_capture_sidecar_file(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let file = crate::auth::open_nofollow(path).ok()?;
    let mut raw = String::new();
    file.take(MAX_CAPTURE_SIDECAR_BYTES + 1)
        .read_to_string(&mut raw)
        .ok()?;
    if raw.len() as u64 > MAX_CAPTURE_SIDECAR_BYTES || raw.trim().is_empty() {
        return None;
    }
    Some(raw)
}

/// Write the installed sidecar once. Existing operator files are left alone.
pub fn write_installed_capture_sidecar(
    paths: &crate::auth::CortexPaths,
    sidecar: &Value,
) -> Result<bool, String> {
    let path = paths.capture_sidecar();
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Cannot create {}: {err}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(sidecar).map_err(|err| err.to_string())?;
    crate::auth::write_secret_file(&path, &bytes)
        .map_err(|err| format!("Cannot write {}: {err}", path.display()))?;
    Ok(true)
}

fn unavailable_envelope(host_event: &str, reason: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":host_event,"additionalContext":"Cortex: memory unavailable for this event. Do not assume the brain is empty."},"cortex":{"event":null,"decision":"UNAVAILABLE","reason":reason,"overflow":false,"presence":"unknown","automatic_capture":false,"counted":false}})
}

pub async fn run(cx: &asupersync::Cx, kind: &str, _agent: &str) -> Result<(), String> {
    use std::io::Read;
    let limit = crate::runtime::observation::MAX_CAPTURE_BYTES;
    let mut raw = Vec::new();
    std::io::stdin()
        .lock()
        .take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|err| err.to_string())?;
    if let Some(value) =
        run_with_paths(cx, kind, &raw, &crate::auth::CortexPaths::resolve()).await?
    {
        println!("{value}");
    }
    Ok(())
}

/// Observation-only live hook. `None` means silent: no sidecar, or this
/// event is not opted in. CQR stays on [`process`].
pub async fn run_with_paths(
    cx: &asupersync::Cx,
    kind: &str,
    raw: &[u8],
    paths: &crate::auth::CortexPaths,
) -> Result<Option<Value>, String> {
    let limit = crate::runtime::observation::MAX_CAPTURE_BYTES;
    if raw.len() > limit {
        return Err("hook_input_byte_limit".into());
    }
    let payload: Value =
        serde_json::from_slice(raw).map_err(|err| format!("invalid_hook_json: {err}"))?;
    let host_event = arg_str(&payload, &["hook_event_name"]).unwrap_or(kind);
    let Some(sidecar_raw) = load_capture_sidecar(paths) else {
        return Ok(None);
    };
    let sidecar: Value = match serde_json::from_str(&sidecar_raw) {
        Ok(value) => value,
        Err(err) => {
            return Ok(Some(unavailable_envelope(
                host_event,
                &format!("invalid capture invocation sidecar: {err}"),
            )));
        }
    };
    match crate::runtime::host_capture::resolve_host_invocation(kind, &sidecar, &payload) {
        Ok(Some(invocation)) => {
            let runtime =
                CortexRuntime::open(paths).map_err(|err| format!("brain unavailable: {err}"))?;
            let (_receipt, view) = runtime
                .capture_and_prepare_host(
                    cx,
                    &invocation.grant,
                    &invocation.context,
                    raw,
                    &invocation.invocation_context,
                    invocation.present.as_deref(),
                )
                .await?;
            if let Some(view) = view {
                let mut context = String::new();
                if view.status == "ready" && !view.payload.is_empty() {
                    context.push_str(&view.payload);
                }
                if !view.assembly_brief.is_empty() {
                    if !context.is_empty() {
                        context.push('\n');
                    }
                    context.push_str(&view.assembly_brief);
                }
                if !context.is_empty() {
                    return Ok(Some(
                        json!({"hookSpecificOutput":{"hookEventName":host_event,"additionalContext":context}}),
                    ));
                }
            }
            Ok(None)
        }
        Ok(None) => Ok(None),
        Err(err) => Ok(Some(unavailable_envelope(
            host_event,
            &format!("invalid capture invocation sidecar: {err}"),
        ))),
    }
}
