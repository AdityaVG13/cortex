use super::*;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub(in crate::runtime::host_capture) fn sidecar_flag(sidecar: &Value, key: &str) -> bool {
    sidecar.get(key).and_then(Value::as_bool) == Some(true)
}

pub(in crate::runtime::host_capture) fn grant_spec_json(
    conn: &Connection,
    principal: &str,
    grant_key: &str,
) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT spec_json FROM host_capture_grants WHERE principal=?1 AND grant_key=?2",
        params![principal, grant_key],
        |r| r.get(0),
    )
}

pub(in crate::runtime::host_capture) fn parse_host_json(raw: &[u8]) -> Result<Value, String> {
    serde_json::from_slice(raw).map_err(|e| format!("malformed_host_record: {e}"))
}
pub(in crate::runtime::host_capture) fn sidecar_string(
    sidecar: &Value,
    key: &str,
) -> Result<String, String> {
    crate::protocol::arg_str(sidecar, &[key])
        .map(str::to_string)
        .ok_or_else(|| format!("Missing trusted {key}"))
}
pub(in crate::runtime::host_capture) fn sidecar_optional_string(
    sidecar: &Value,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = sidecar.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .and_then(crate::protocol::nonempty_str)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Invalid trusted {key}"))
        .map(Some)
}
pub(in crate::runtime::host_capture) fn is_uuid(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 36
        && b[8] == b'-'
        && b[13] == b'-'
        && b[18] == b'-'
        && b[23] == b'-'
        && (0..36).all(|i| matches!(i, 8 | 13 | 18 | 23) || b[i].is_ascii_hexdigit())
}
pub(in crate::runtime::host_capture) fn is_tool_use_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub(in crate::runtime::host_capture) fn parse_origins(
    value: &Value,
) -> Result<Vec<HostOriginBinding>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "Missing trusted origins".to_string())?;
    if object.len() > 128 {
        return Err("host_origin_limit".into());
    }
    object
        .iter()
        .map(|(event_key, origin)| {
            Ok(HostOriginBinding {
                event_key: event_key.clone(),
                origin: match origin.as_str() {
                    Some("external") => HostOrigin::External,
                    Some("cortex_delivery") => HostOrigin::CortexDelivery,
                    _ => return Err("invalid_host_origin".into()),
                },
            })
        })
        .collect()
}
mod invoke;
pub use invoke::resolve_host_invocation;
mod situation;
pub(super) use situation::situation_cues;
pub(super) fn label(s: &str) -> Result<(), String> {
    if s.trim().is_empty() || s.len() > 1024 {
        Err("invalid_host_identity".into())
    } else {
        Ok(())
    }
}
pub(super) fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("unsupported_host_schema: required string {key}"))
}
pub(super) fn source_key(grant: &str, kind: HostRecordKind) -> String {
    format!("host:{}", json!([grant, kind]))
}

pub(super) fn payload_cwd(value: &Value) -> Option<&str> {
    string_field(value, crate::protocol::CWD_KEYS)
}

pub(super) fn observation_scope_for(grant: &HostCaptureGrant, value: &Value) -> String {
    let grant_scope = observation::normalize_scope(&grant.scope);
    let Some(cwd) = payload_cwd(value) else {
        return grant_scope;
    };
    let scope = observation::normalize_scope(cwd);
    if !(scope.is_empty() || scope.len() > 1024) && observation::scope_is_path(&scope) {
        scope
    } else {
        grant_scope
    }
}

pub(super) fn observation_scope_for_raw(grant: &HostCaptureGrant, raw: &[u8]) -> String {
    match serde_json::from_slice::<Value>(raw) {
        Ok(value) => observation_scope_for(grant, &value),
        Err(_) => observation::normalize_scope(&grant.scope),
    }
}

pub(super) fn scoped_source_key(
    grant: &HostCaptureGrant,
    kind: HostRecordKind,
    scope: &str,
) -> String {
    let grant_n = observation::normalize_scope(&grant.scope);
    if scope == grant_n {
        return source_key(&grant.key, kind);
    }
    let key = format!("host:{}", json!([&grant.key, kind, scope]));
    if key.len() <= 1024 {
        key
    } else {
        format!(
            "host:{}",
            json!([&grant.key, kind, cortex_logic::traces::content_hash(scope)])
        )
    }
}

pub(super) fn existing_host_source(
    conn: &rusqlite::Connection,
    principal: &str,
    generation: &str,
    event_key: &str,
    grant_key: &str,
    kind: HostRecordKind,
) -> Result<Option<String>, String> {
    let mut stmt = conn.prepare("SELECT source_key FROM observation_events WHERE principal=?1 AND generation=?2 AND event_key=?3").map_err(|e| e.to_string())?;
    let keys = stmt
        .query_map(params![principal, generation, event_key], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let unscoped = source_key(grant_key, kind);
    let stem = format!(
        "host:{}",
        json!([grant_key, kind]).to_string().trim_end_matches(']')
    );
    Ok(keys
        .into_iter()
        .find(|key| *key == unscoped || key.starts_with(&(stem.clone() + ","))))
}

pub(super) fn insert_host_metadata(
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    grant_key: &str,
    generation: &str,
    byte_offset: i64,
    kind: &str,
    line: &[u8],
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO host_capture_metadata VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            principal,
            grant_key,
            generation,
            byte_offset,
            kind,
            line.len() as i64,
            crate::handlers::digest_hex(line)
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
pub(in crate::runtime::host_capture) fn file_tool_name(name: &str) -> bool {
    matches!(name, "Edit" | "Write" | "Read" | "MultiEdit")
}
pub(super) fn tool_path(value: &Value) -> Option<&str> {
    string_field(value, &["filePath", "file_path", "path"]).or_else(|| {
        value
            .get("file")
            .and_then(|file| string_field(file, &["filePath", "file_path"]))
    })
}
pub(super) fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}
pub(super) fn history_tool_name(response: &Value) -> &'static str {
    let is_read = response.get("file").is_some()
        || (tool_path(response).is_some()
            && string_field(response, &["content"]).is_some()
            && response.get("newString").is_none()
            && response.get("edits").is_none());
    [
        (is_read, "Read"),
        (response.get("edits").is_some(), "MultiEdit"),
        (response.get("newString").is_some(), "Edit"),
    ]
    .into_iter()
    .find(|(hit, _)| *hit)
    .map(|(_, name)| name)
    .unwrap_or("Write")
}
pub(super) fn live_hook_name(raw: &[u8]) -> Result<Option<String>, String> {
    Ok(parse_host_json(raw)?
        .get("hook_event_name")
        .and_then(Value::as_str)
        .map(str::to_string))
}
pub(super) fn cursor_key(grant: &str) -> String {
    format!("host-cursor:{}", json!([grant]))
}
pub(super) fn generation(context: &HostCaptureContext) -> String {
    json!([context.session_id, context.generation]).to_string()
}
pub(super) fn validate(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    route: HostRoute,
) -> Result<(), String> {
    for s in [
        &grant.key,
        &grant.scope,
        &grant.host_version,
        &context.session_id,
        &context.generation,
    ] {
        label(s)?;
    }
    let adapter_ok = grant.adapter_version == ADAPTER_VERSION
        || (grant.adapter_version == CLAUDE_2_1_260_ADAPTER && grant.host_version == "2.1.260");
    if !adapter_ok || context.host_version != grant.host_version {
        return Err("unsupported_host_version".into());
    }
    if grant.max_bytes == 0 || grant.max_bytes > MAX_CAPTURE_BYTES {
        return Err("invalid_capture_limit".into());
    }
    match route {
        HostRoute::Live => grant.live,
        HostRoute::History => grant.history,
    }
    .then_some(())
    .ok_or_else(|| "host_route_not_authorized".into())
}

pub(super) fn user_event_key<'a>(
    grant: &HostCaptureGrant,
    context: &'a HostCaptureContext,
    route: HostRoute,
    v: &'a Value,
) -> Result<&'a str, String> {
    if grant.adapter_version == CLAUDE_2_1_260_ADAPTER {
        let key = field(
            v,
            match route {
                HostRoute::Live => "prompt_id",
                HostRoute::History => "promptId",
            },
        )?;
        if route == HostRoute::Live
            && context
                .original_event_key
                .as_deref()
                .is_some_and(|expected| expected != key)
        {
            return Err("host_original_identity_mismatch".into());
        }
        Ok(key)
    } else {
        match route {
            HostRoute::Live => context
                .original_event_key
                .as_deref()
                .ok_or_else(|| "original_event_identity_required".into()),
            HostRoute::History => field(v, "uuid"),
        }
    }
}

mod metadata;
pub(in crate::runtime::host_capture) use metadata::metadata_kind;
