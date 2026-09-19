use super::*;
use serde_json::Value;

fn native_result_pair<'a>(v: &'a Value, route: HostRoute) -> Result<(&'a str, &'a Value), String> {
    match route {
        HostRoute::Live => Ok((
            field(v, "tool_use_id")?,
            v.get("tool_response")
                .ok_or("missing_native_tool_response")?,
        )),
        HostRoute::History => {
            if sidecar_flag(v, "isSidechain") {
                return Err("unsupported_host_sidechain".into());
            }
            let blocks = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .filter(|b| b.len() == 1)
                .ok_or("unsupported_host_content_blocks")?;
            if field(&blocks[0], "type")? != "tool_result" {
                return Err("unsupported_host_content_block".into());
            }
            Ok((
                field(&blocks[0], "tool_use_id")?,
                v.get("toolUseResult")
                    .ok_or("missing_native_tool_response")?,
            ))
        }
    }
}

fn is_bash_response(response: &Value) -> bool {
    response.as_object().is_some_and(|object| {
        object.len() == 5
            && ["stdout", "stderr"]
                .iter()
                .all(|key| object.get(*key).is_some_and(Value::is_string))
            && ["interrupted", "isImage", "noOutputExpected"]
                .iter()
                .all(|key| object.get(*key).and_then(Value::as_bool).is_some())
    })
}

fn native_tool_record(
    grant: &HostCaptureGrant,
    key: &str,
    response: &Value,
) -> Result<NormalizedHostEvent, String> {
    label(key)?;
    let text = serde_json::to_string(response).map_err(|e| e.to_string())?;
    if text.len() > grant.max_bytes {
        return Err("capture_byte_limit".into());
    }
    Ok(NormalizedHostEvent {
        kind: HostRecordKind::Tool,
        event: ObservationEvent {
            event_key: key.into(),
            text,
            observed_at: None,
        },
    })
}

fn native_live_named<'a>(
    v: &'a Value,
    route: HostRoute,
    ok: impl FnOnce(&str) -> bool,
) -> Result<(&'a str, &'a Value), String> {
    if route == HostRoute::Live && !ok(field(v, "tool_name")?) {
        return Err("unsupported_native_tool".into());
    }
    native_result_pair(v, route)
}

fn native_bash_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let (key, response) = native_live_named(v, route, |name| name == "Bash")?;
    if !is_bash_response(response) {
        return Err("unsupported_native_tool_response".into());
    }
    native_tool_record(grant, key, response)
}

pub(super) fn native_file_tool_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let (key, response) = native_live_named(v, route, file_tool_name)?;
    if !response.is_object() || tool_path(response).is_none() {
        return Err("unsupported_native_tool_response".into());
    }
    let name = match route {
        HostRoute::Live => field(v, "tool_name")?,
        HostRoute::History => history_tool_name(response),
    };
    match name {
        "Read" => {
            let content = response
                .get("file")
                .and_then(|file| file.get("content"))
                .or_else(|| response.get("content"));
            if !content.is_some_and(Value::is_string) {
                return Err("unsupported_native_tool_response".into());
            }
        }
        "Edit" | "MultiEdit" => {
            if response.get("newString").is_none()
                && response.get("edits").is_none()
                && response.get("structuredPatch").is_none()
            {
                return Err("unsupported_native_tool_response".into());
            }
        }
        "Write" => {}
        _ => return Err("unsupported_native_tool".into()),
    }
    native_tool_record(grant, key, response)
}

pub(super) fn native_tool_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let name = match route {
        HostRoute::Live => field(v, "tool_name")?,
        HostRoute::History => match v.get("toolUseResult") {
            Some(response) if is_bash_response(response) => "Bash",
            Some(response) => history_tool_name(response),
            None => return Err("missing_native_tool_response".into()),
        },
    };
    match name {
        "Bash" => native_bash_event(grant, v, route),
        name if file_tool_name(name) => native_file_tool_event(grant, v, route),
        _ => Err("unsupported_native_tool".into()),
    }
}
