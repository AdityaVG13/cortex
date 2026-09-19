use super::native::{native_file_tool_event, native_tool_event};
use super::*;
use serde_json::Value;

/// Normalize evidence records, not transcript control frames. The fixture adapter
/// accepts user/tool strings and a single final assistant text block. The native
/// adapter additionally uses prompt_id/promptId and structured tool reports.
/// `tail_host_transcript` accounts known non-evidence control records separately.
/// Unknown or mixed/private evidence shapes fail; reported strings are not trimmed.
pub fn normalize_host_event(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    route: HostRoute,
    raw: &[u8],
) -> Result<NormalizedHostEvent, String> {
    validate(grant, context, route)?;
    let v = parse_host_object(raw)?;
    reject_untrusted_authority(&v)?;
    require_matching_session(&v, context, route)?;
    require_history_version(grant, &v, route)?;
    if is_native_tool_envelope(grant, &v, route) {
        return native_tool_event(grant, &v, route);
    }
    match route {
        HostRoute::Live => normalize_live(grant, context, &v),
        HostRoute::History => normalize_history(grant, context, &v),
    }
}

fn parse_host_object(raw: &[u8]) -> Result<Value, String> {
    if raw.len() > MAX_CAPTURE_BYTES {
        return Err("capture_batch_byte_limit".into());
    }
    let v = parse_host_json(raw)?;
    v.is_object()
        .then_some(v)
        .ok_or_else(|| "unsupported_host_schema: expected object".into())
}

fn reject_untrusted_authority(v: &Value) -> Result<(), String> {
    for forbidden in [
        "role",
        "source_role",
        "owner",
        "principal",
        "grant",
        "origin",
    ] {
        if v.get(forbidden).is_some() {
            return Err(format!("untrusted_host_authority_field: {forbidden}"));
        }
    }
    Ok(())
}

fn require_matching_session(
    v: &Value,
    context: &HostCaptureContext,
    route: HostRoute,
) -> Result<(), String> {
    let session = match route {
        HostRoute::Live => field(v, "session_id")?,
        HostRoute::History => field(v, "sessionId")?,
    };
    (session == context.session_id)
        .then_some(())
        .ok_or_else(|| "host_session_mismatch".into())
}

fn require_history_version(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<(), String> {
    if grant.adapter_version == CLAUDE_2_1_260_ADAPTER
        && route == HostRoute::History
        && field(v, "version")? != "2.1.260"
    {
        Err("unsupported_host_record_version".into())
    } else {
        Ok(())
    }
}

fn is_native_tool_envelope(grant: &HostCaptureGrant, v: &Value, route: HostRoute) -> bool {
    grant.adapter_version == CLAUDE_2_1_260_ADAPTER
        && ((route == HostRoute::Live
            && v.get("hook_event_name").and_then(Value::as_str) == Some("PostToolUse"))
            || (route == HostRoute::History
                && v.get("type").and_then(Value::as_str) == Some("user")
                && v.get("message").is_some_and(|m| {
                    m.get("role").and_then(Value::as_str) == Some("user")
                        && m.get("content").is_some_and(Value::is_array)
                })))
}

fn finish_normalized(
    grant: &HostCaptureGrant,
    kind: HostRecordKind,
    key: &str,
    text: &str,
) -> Result<NormalizedHostEvent, String> {
    label(key)?;
    if text.len() > grant.max_bytes {
        return Err("capture_byte_limit".into());
    }
    Ok(NormalizedHostEvent {
        kind,
        event: ObservationEvent {
            event_key: key.into(),
            text: text.into(),
            observed_at: None,
        },
    })
}

fn normalize_live(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    v: &Value,
) -> Result<NormalizedHostEvent, String> {
    let name = field(v, "hook_event_name")?;
    let route = HostRoute::Live;
    let (kind, key, text) = match name {
        "UserPromptSubmit" => (
            HostRecordKind::User,
            user_event_key(grant, context, route, v)?,
            field(v, "prompt")?,
        ),
        "Stop" => (
            HostRecordKind::Final,
            context
                .original_event_key
                .as_deref()
                .ok_or("original_event_identity_required")?,
            field(v, "last_assistant_message")?,
        ),
        "PostToolUse" => {
            if v.get("tool_name")
                .and_then(Value::as_str)
                .is_some_and(file_tool_name)
                && v.get("tool_response").is_some_and(Value::is_object)
            {
                return native_file_tool_event(grant, v, route);
            }
            (
                HostRecordKind::Tool,
                field(v, "tool_use_id")?,
                field(v, "tool_response")?,
            )
        }
        _ => return Err(format!("unsupported_host_event: {name}")),
    };
    finish_normalized(grant, kind, key, text)
}

fn single_content_block(content: &Value) -> Result<&Value, String> {
    content
        .as_array()
        .filter(|a| a.len() == 1)
        .map(|a| &a[0])
        .ok_or_else(|| "unsupported_host_content_blocks".into())
}

fn normalize_history(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    v: &Value,
) -> Result<NormalizedHostEvent, String> {
    if sidecar_flag(v, "isSidechain") {
        return Err("unsupported_host_sidechain".into());
    }
    let message = v
        .get("message")
        .ok_or("unsupported_host_schema: missing message")?;
    let content = message
        .get("content")
        .ok_or("unsupported_host_schema: missing content")?;
    let route = HostRoute::History;
    let (kind, key, text) = match (field(v, "type")?, field(message, "role")?) {
        ("user", "user") if content.is_string() => (
            HostRecordKind::User,
            user_event_key(grant, context, route, v)?,
            content.as_str().unwrap(),
        ),
        ("user", "user") => {
            let block = single_content_block(content)?;
            if field(block, "type")? != "tool_result" {
                return Err("unsupported_host_content_block".into());
            }
            (
                HostRecordKind::Tool,
                field(block, "tool_use_id")?,
                field(block, "content")?,
            )
        }
        ("assistant", "assistant") => {
            if field(message, "stop_reason")? != "end_turn" {
                return Err("unsupported_host_nonfinal_assistant".into());
            }
            let block = single_content_block(content)?;
            if field(block, "type")? != "text" {
                return Err("unsupported_host_private_or_unknown_block".into());
            }
            (
                HostRecordKind::Final,
                field(v, "uuid")?,
                field(block, "text")?,
            )
        }
        _ => return Err("unsupported_host_record_type".into()),
    };
    finish_normalized(grant, kind, key, text)
}
pub(super) fn origin(context: &HostCaptureContext, key: &str) -> Result<HostOrigin, String> {
    let mut entries = context.origins.iter().filter(|b| b.event_key == key);
    let value = entries.next().ok_or("host_origin_unresolved")?.origin;
    if entries.next().is_some() {
        return Err("host_origin_ambiguous".into());
    }
    if value == HostOrigin::Unknown {
        return Err("host_origin_unresolved".into());
    }
    Ok(value)
}
