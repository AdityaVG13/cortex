use super::*;
use serde_json::{Value, json};

const NATIVE_FLAGS: &[&str] = &[
    "native_user_prompts",
    "native_bash_results",
    "native_file_results",
    "native_hooks",
    "native_compactions",
    "native_finals",
];

pub(super) fn native_flags_set(sidecar: &Value) -> bool {
    NATIVE_FLAGS.iter().any(|flag| sidecar_flag(sidecar, flag))
}

fn sidecar_flag_or_hooks(sidecar: &Value, flag: &str) -> bool {
    sidecar_flag(sidecar, flag) || sidecar_flag(sidecar, "native_hooks")
}

#[derive(Clone, Copy)]
enum NativeKind {
    User,
    Bash,
    File,
    Pre,
    Compact,
    Stop,
}

impl NativeKind {
    fn detect(kind: &str, sidecar: &Value, tool_name: &str) -> Option<Self> {
        match kind {
            "UserPromptSubmit" if sidecar_flag(sidecar, "native_user_prompts") => Some(Self::User),
            "PostToolUse"
                if sidecar_flag(sidecar, "native_bash_results") && tool_name == "Bash" =>
            {
                Some(Self::Bash)
            }
            "PostToolUse"
                if sidecar_flag_or_hooks(sidecar, "native_file_results")
                    && file_tool_name(tool_name) =>
            {
                Some(Self::File)
            }
            "PreToolUse" if sidecar_flag_or_hooks(sidecar, "native_file_results") => {
                Some(Self::Pre)
            }
            "PreCompact" if sidecar_flag_or_hooks(sidecar, "native_compactions") => {
                Some(Self::Compact)
            }
            "Stop" if sidecar_flag_or_hooks(sidecar, "native_finals") => Some(Self::Stop),
            _ => None,
        }
    }

    fn is_prompt_or_bash(self) -> bool {
        matches!(self, Self::User | Self::Bash)
    }
}

fn native_session<'a>(payload: &'a Value, kind: &str) -> Result<&'a str, String> {
    let hook_name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or("Missing native hook identity")?;
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("Missing native hook identity")?;
    if hook_name != kind || !is_uuid(session_id) {
        return Err("Missing native hook identity".into());
    }
    Ok(session_id)
}

fn stamp_native_session(
    session: &mut Option<String>,
    generation: &mut Option<String>,
    session_id: &str,
    host_version: &str,
) {
    *session = Some(session_id.to_string());
    if generation.is_none() {
        *generation = Some(host_version.to_string());
    }
}

/// Build a live host invocation from the operator sidecar and this event.
/// `Ok(None)` means this event is not on the observation path (CQR hook may run).
pub fn resolve_host_invocation(
    kind: &str,
    sidecar: &Value,
    payload: &Value,
) -> Result<Option<HostInvocation>, String> {
    if !sidecar.is_object() {
        return Err("invalid capture invocation sidecar".into());
    }
    let tool_name = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let native = NativeKind::detect(kind, sidecar, tool_name);
    if native_flags_set(sidecar) && native.is_none() {
        return Ok(None);
    }
    let host_version = sidecar_string(sidecar, "host_version")?;
    let mut session = sidecar_optional_string(sidecar, "session")?;
    let mut generation = sidecar_optional_string(sidecar, "generation")?;
    let mut event_key = sidecar_optional_string(sidecar, "event_key")?;
    let mut invocation_context = sidecar_optional_string(sidecar, "context")?;
    let mut origins = sidecar.get("origins").cloned();
    match native {
        Some(native) if native.is_prompt_or_bash() => {
            let session_id = native_session(payload, kind)?;
            let key = if matches!(native, NativeKind::User) {
                payload
                    .get("prompt_id")
                    .and_then(Value::as_str)
                    .ok_or("Missing native hook identity")?
            } else {
                payload
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .ok_or("Missing native hook identity")?
            };
            if (matches!(native, NativeKind::User)
                && (!is_uuid(key) || !payload.get("prompt").map(Value::is_string).unwrap_or(false)))
                || (matches!(native, NativeKind::Bash) && !is_tool_use_id(key))
            {
                return Err("Missing native hook identity".into());
            }
            stamp_native_session(&mut session, &mut generation, session_id, &host_version);
            event_key = Some(key.to_string());
            origins = Some(json!({ key: "external" }));
            invocation_context = Some(format!(
                "claude-{}:{session_id}:{key}",
                if matches!(native, NativeKind::User) {
                    "user"
                } else {
                    "tool"
                }
            ));
        }
        Some(native) => {
            let session_id = native_session(payload, kind)?;
            let key = if matches!(native, NativeKind::Stop) {
                event_key.clone()
            } else if matches!(native, NativeKind::Compact) {
                event_key.clone().or_else(|| Some(session_id.to_string()))
            } else {
                payload
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            let Some(key) = key.filter(|value| !value.trim().is_empty()) else {
                return Err("Missing native hook identity".into());
            };
            stamp_native_session(&mut session, &mut generation, session_id, &host_version);
            event_key = Some(key.clone());
            if origins
                .as_ref()
                .is_none_or(|value| !value.is_object() || value.is_array())
            {
                origins = Some(json!({ key.as_str(): "external" }));
            }
            if invocation_context.is_none() {
                invocation_context = Some(format!("host-{kind}:{session_id}:{key}"));
            }
        }
        None => {}
    }
    let context = HostCaptureContext {
        host_version,
        session_id: session.ok_or("Missing trusted session")?,
        generation: generation.ok_or("Missing trusted generation")?,
        original_event_key: event_key,
        origins: parse_origins(origins.as_ref().ok_or("Missing trusted origins")?)?,
    };
    Ok(Some(HostInvocation {
        grant: sidecar_string(sidecar, "grant")?,
        context,
        invocation_context: invocation_context.ok_or("Missing trusted context")?,
        present: sidecar_optional_string(sidecar, "present")?,
    }))
}
