//! Fail-closed host capture. The fixture adapter accepts string tool results;
//! the native adapter also accepts structured Bash and file-tool reports.
//! Neither establishes reader quality or compatibility with every host tool.
//! Callers supply authenticated invocation metadata and origin sidecars separately
//! from host JSON. This module never opens a path supplied by a hook.
use super::{
    CortexRuntime,
    observation::{
        self, MAX_BATCH_EVENTS, MAX_CAPTURE_BYTES, ObservationEvent, ObservationReceipt,
        ObservationRole, SourceSpec,
    },
};
use asupersync::Cx;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const ADAPTER_VERSION: &str = "claude-visible-subset-v1";
pub const CLAUDE_2_1_260_ADAPTER: &str = "claude-code-2.1.260-v1";
/// Default grant key written by `cortex setup`. Operator-owned, not host-derived.
pub const INSTALLED_HOST_GRANT_KEY: &str = "local-host";

/// Grant used when setup enables live host capture. Pins stay in this module.
pub fn installed_host_grant() -> HostCaptureGrant {
    HostCaptureGrant {
        key: INSTALLED_HOST_GRANT_KEY.into(),
        scope: "project".into(),
        host_version: "2.1.260".into(),
        adapter_version: CLAUDE_2_1_260_ADAPTER.into(),
        max_bytes: 65536,
        live: true,
        history: true,
    }
}

/// Sidecar that opts the installed host events into observation capture.
pub fn installed_capture_sidecar() -> Value {
    json!({
        "grant": INSTALLED_HOST_GRANT_KEY,
        "host_version": "2.1.260",
        "native_user_prompts": true,
        "native_bash_results": true,
        "native_file_results": true,
        "native_compactions": true
    })
}
const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS host_capture_grants (
 principal TEXT NOT NULL, grant_key TEXT NOT NULL, spec_json TEXT NOT NULL,
 PRIMARY KEY(principal,grant_key));
CREATE TABLE IF NOT EXISTS host_capture_origins (
 principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL,
 event_key TEXT NOT NULL, kind TEXT NOT NULL, origin TEXT NOT NULL,
 PRIMARY KEY(principal,grant_key,generation,event_key));
CREATE TABLE IF NOT EXISTS host_capture_metadata (
 principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL,
 byte_offset INTEGER NOT NULL, kind TEXT NOT NULL, byte_length INTEGER NOT NULL,
 sha256 TEXT NOT NULL, PRIMARY KEY(principal,grant_key,generation,byte_offset));
"#;

/// Register only through the authenticated operator/library boundary. A version
/// is an exact caller assertion, not detected from untrusted event contents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostCaptureGrant {
    pub key: String,
    pub scope: String,
    pub host_version: String,
    pub adapter_version: String,
    pub max_bytes: usize,
    pub live: bool,
    pub history: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRoute {
    Live,
    History,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRecordKind {
    User,
    Tool,
    Final,
}
impl HostRecordKind {
    fn role(self) -> ObservationRole {
        match self {
            Self::User => ObservationRole::UserStatement,
            Self::Tool => ObservationRole::ToolReport,
            Self::Final => ObservationRole::AgentAssertion,
        }
    }
    fn role_name(self) -> &'static str {
        match self {
            Self::User => "user_statement",
            Self::Tool => "tool_report",
            Self::Final => "agent_assertion",
        }
    }
}
/// External means the trusted host adapter resolved native origin, not that the
/// payload claimed to be external. Unknown origin cannot enter factual capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOrigin {
    External,
    CortexDelivery,
    Unknown,
}
#[derive(Debug, Clone)]
pub struct HostOriginBinding {
    pub event_key: String,
    pub origin: HostOrigin,
}
/// Not deserializable deliberately: CLI integrations must not deserialize this
/// authority object from hook stdin. History needs per-original-event lineage.
#[derive(Debug, Clone)]
pub struct HostCaptureContext {
    pub host_version: String,
    pub session_id: String,
    pub generation: String,
    /// Transcript UUID for live final and legacy user records. Native user
    /// events carry prompt_id; this field optionally checks that identity.
    /// Never synthesize an identity from content or import time.
    pub original_event_key: Option<String>,
    pub origins: Vec<HostOriginBinding>,
}
/// Operator sidecar plus host payload, never deserialized from stdin alone.
#[derive(Debug, Clone)]
pub struct HostInvocation {
    pub grant: String,
    pub context: HostCaptureContext,
    pub invocation_context: String,
    pub present: Option<String>,
}
fn sidecar_flag(sidecar: &Value, key: &str) -> bool {
    sidecar.get(key).and_then(Value::as_bool) == Some(true)
}
fn sidecar_string(sidecar: &Value, key: &str) -> Result<String, String> {
    sidecar
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Missing trusted {key}"))
}
fn sidecar_optional_string(sidecar: &Value, key: &str) -> Result<Option<String>, String> {
    match sidecar.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => {
            if value.trim().is_empty() {
                Err(format!("Invalid trusted {key}"))
            } else {
                Ok(Some(value.trim().to_string()))
            }
        }
        _ => Err(format!("Invalid trusted {key}")),
    }
}
fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    let hex = |index: usize| bytes[index].is_ascii_hexdigit();
    (0..8).all(hex)
        && bytes[8] == b'-'
        && (9..13).all(hex)
        && bytes[13] == b'-'
        && (14..18).all(hex)
        && bytes[18] == b'-'
        && (19..23).all(hex)
        && bytes[23] == b'-'
        && (24..36).all(hex)
}
fn is_tool_use_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn parse_origins(value: &Value) -> Result<Vec<HostOriginBinding>, String> {
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
fn native_flags_set(sidecar: &Value) -> bool {
    sidecar_flag(sidecar, "native_user_prompts")
        || sidecar_flag(sidecar, "native_bash_results")
        || sidecar_flag(sidecar, "native_file_results")
        || sidecar_flag(sidecar, "native_hooks")
        || sidecar_flag(sidecar, "native_compactions")
        || sidecar_flag(sidecar, "native_finals")
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
    let tool_name = payload.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let native_user = kind == "UserPromptSubmit" && sidecar_flag(sidecar, "native_user_prompts");
    let native_bash =
        kind == "PostToolUse" && sidecar_flag(sidecar, "native_bash_results") && tool_name == "Bash";
    let native_file = kind == "PostToolUse"
        && (sidecar_flag(sidecar, "native_file_results") || sidecar_flag(sidecar, "native_hooks"))
        && file_tool_name(tool_name);
    let native_pre = kind == "PreToolUse"
        && (sidecar_flag(sidecar, "native_file_results") || sidecar_flag(sidecar, "native_hooks"));
    let native_compact = kind == "PreCompact"
        && (sidecar_flag(sidecar, "native_compactions") || sidecar_flag(sidecar, "native_hooks"));
    let native_stop =
        kind == "Stop" && (sidecar_flag(sidecar, "native_finals") || sidecar_flag(sidecar, "native_hooks"));
    if native_flags_set(sidecar)
        && !(native_user || native_bash || native_file || native_pre || native_compact || native_stop)
    {
        return Ok(None);
    }
    let host_version = sidecar_string(sidecar, "host_version")?;
    let mut session = sidecar_optional_string(sidecar, "session")?;
    let mut generation = sidecar_optional_string(sidecar, "generation")?;
    let mut event_key = sidecar_optional_string(sidecar, "event_key")?;
    let mut invocation_context = sidecar_optional_string(sidecar, "context")?;
    let mut origins = sidecar.get("origins").cloned();
    if native_user || native_bash {
        let hook_name = payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .ok_or("Missing native hook identity")?;
        let session_id = payload
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or("Missing native hook identity")?;
        let key = if native_user {
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
        if hook_name != kind
            || !is_uuid(session_id)
            || (native_user && (!is_uuid(key) || !payload.get("prompt").map(Value::is_string).unwrap_or(false)))
            || (native_bash && !is_tool_use_id(key))
        {
            return Err("Missing native hook identity".into());
        }
        session = Some(session_id.to_string());
        if generation.is_none() {
            generation = Some(host_version.clone());
        }
        event_key = Some(key.to_string());
        origins = Some(json!({ key: "external" }));
        invocation_context = Some(format!(
            "claude-{}:{session_id}:{key}",
            if native_user { "user" } else { "tool" }
        ));
    } else if native_file || native_pre || native_compact || native_stop {
        let hook_name = payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .ok_or("Missing native hook identity")?;
        let session_id = payload
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or("Missing native hook identity")?;
        let key = if native_stop {
            event_key.clone()
        } else if native_compact {
            event_key
                .clone()
                .or_else(|| Some(session_id.to_string()))
        } else {
            payload
                .get("tool_use_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let Some(key) = key.filter(|value| !value.trim().is_empty()) else {
            return Err("Missing native hook identity".into());
        };
        if hook_name != kind || !is_uuid(session_id) {
            return Err("Missing native hook identity".into());
        }
        session = Some(session_id.to_string());
        if generation.is_none() {
            generation = Some(host_version.clone());
        }
        event_key = Some(key.clone());
        if origins.as_ref().is_none_or(|value| !value.is_object() || value.is_array()) {
            origins = Some(json!({ key.as_str(): "external" }));
        }
        if invocation_context.is_none() {
            invocation_context = Some(format!("host-{kind}:{session_id}:{key}"));
        }
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
#[derive(Debug, Clone, Serialize)]
pub struct NormalizedHostEvent {
    pub kind: HostRecordKind,
    pub event: ObservationEvent,
}
#[derive(Debug, Clone, Serialize)]
pub struct HostCaptureReceipt {
    pub adapter_version: String,
    pub accepted: Vec<ObservationReceipt>,
    pub excluded_deliveries: usize,
    pub ignored_metadata: usize,
    pub next_offset: Option<u64>,
    pub uncommitted_tail_bytes: usize,
}
fn label(s: &str) -> Result<(), String> {
    if s.trim().is_empty() || s.len() > 1024 {
        Err("invalid_host_identity".into())
    } else {
        Ok(())
    }
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("unsupported_host_schema: required string {key}"))
}
fn source_key(grant: &str, kind: HostRecordKind) -> String {
    format!("host:{}", json!([grant, kind]))
}

fn payload_cwd(value: &Value) -> Option<&str> {
    string_field(value, &["cwd", "cwd_path", "working_directory"])
}

fn observation_scope_for(grant: &HostCaptureGrant, value: &Value) -> String {
    let grant_scope = observation::normalize_scope(&grant.scope);
    let Some(cwd) = payload_cwd(value) else {
        return grant_scope;
    };
    let scope = observation::normalize_scope(cwd);
    if scope.is_empty() || scope.len() > 1024 {
        return grant_scope;
    }
    if scope.contains('/') || scope.contains('\\') {
        scope
    } else {
        grant_scope
    }
}

fn observation_scope_for_raw(grant: &HostCaptureGrant, raw: &[u8]) -> String {
    match serde_json::from_slice::<Value>(raw) {
        Ok(value) => observation_scope_for(grant, &value),
        Err(_) => observation::normalize_scope(&grant.scope),
    }
}

fn scoped_source_key(grant: &HostCaptureGrant, kind: HostRecordKind, scope: &str) -> String {
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

fn existing_host_source(
    conn: &rusqlite::Connection,
    principal: &str,
    generation: &str,
    event_key: &str,
    grant_key: &str,
    kind: HostRecordKind,
) -> Result<Option<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT source_key FROM observation_events WHERE principal=?1 AND generation=?2 AND event_key=?3",
        )
        .map_err(|e| e.to_string())?;
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
fn file_tool_name(name: &str) -> bool {
    matches!(name, "Edit" | "Write" | "Read" | "MultiEdit")
}
fn tool_path(value: &Value) -> Option<&str> {
    value
        .get("filePath")
        .or_else(|| value.get("file_path"))
        .or_else(|| value.get("path"))
        .and_then(Value::as_str)
        .or_else(|| {
            value.get("file").and_then(|file| {
                file.get("filePath")
                    .or_else(|| file.get("file_path"))
                    .and_then(Value::as_str)
            })
        })
}
fn string_field<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
}
fn history_tool_name(response: &Value) -> &'static str {
    if response.get("file").is_some()
        || (tool_path(response).is_some()
            && string_field(response, &["content"]).is_some()
            && response.get("newString").is_none()
            && response.get("edits").is_none())
    {
        "Read"
    } else if response.get("newString").is_some() || response.get("edits").is_some() {
        if response.get("edits").is_some() {
            "MultiEdit"
        } else {
            "Edit"
        }
    } else {
        "Write"
    }
}
fn collect_situation(value: &Value, out: &mut Vec<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, child) in object {
        match key.to_ascii_lowercase().as_str() {
            "filepath" | "file_path" | "path" | "stdout" | "stderr" | "content" | "newstring"
            | "oldstring" | "new_string" | "old_string" | "command" | "prompt" => {
                if let Some(text) = child.as_str() {
                    out.push(text.to_string());
                }
            }
            "file" | "tool_input" | "tool_response" => collect_situation(child, out),
            "edits" => {
                if let Some(items) = child.as_array() {
                    for item in items {
                        collect_situation(item, out);
                    }
                }
            }
            _ => {}
        }
    }
}
fn situation_cues(text: &str) -> Vec<String> {
    let tokens = if let Ok(value) = serde_json::from_str::<Value>(text) {
        let mut parts = Vec::new();
        collect_situation(&value, &mut parts);
        parts.join("\n")
    } else {
        text.to_string()
    };
    tokens
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '.' && c != '/')
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .map(str::to_lowercase)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(32)
        .collect()
}
fn live_hook_name(raw: &[u8]) -> Result<Option<String>, String> {
    let value: Value =
        serde_json::from_slice(raw).map_err(|e| format!("malformed_host_record: {e}"))?;
    Ok(value
        .get("hook_event_name")
        .and_then(Value::as_str)
        .map(str::to_string))
}
fn cursor_key(grant: &str) -> String {
    format!("host-cursor:{}", json!([grant]))
}
fn generation(context: &HostCaptureContext) -> String {
    json!([context.session_id, context.generation]).to_string()
}
fn validate(
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
    if (grant.adapter_version != ADAPTER_VERSION
        && (grant.adapter_version != CLAUDE_2_1_260_ADAPTER || grant.host_version != "2.1.260"))
        || context.host_version != grant.host_version
    {
        return Err("unsupported_host_version".into());
    }
    if grant.max_bytes == 0 || grant.max_bytes > MAX_CAPTURE_BYTES {
        return Err("invalid_capture_limit".into());
    }
    if !match route {
        HostRoute::Live => grant.live,
        HostRoute::History => grant.history,
    } {
        return Err("host_route_not_authorized".into());
    }
    Ok(())
}

fn user_event_key<'a>(
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

fn metadata_kind(
    grant: &HostCaptureGrant,
    context: &HostCaptureContext,
    line: &[u8],
) -> Result<Option<&'static str>, String> {
    if grant.adapter_version != CLAUDE_2_1_260_ADAPTER {
        return Ok(None);
    }
    let v: Value =
        serde_json::from_slice(line).map_err(|e| format!("malformed_host_record: {e}"))?;
    if field(&v, "sessionId")? != context.session_id {
        return Err("host_session_mismatch".into());
    }
    if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
        return Err("unsupported_host_sidechain".into());
    }
    if v.get("version")
        .is_some_and(|version| version.as_str() != Some("2.1.260"))
    {
        return Err("unsupported_host_record_version".into());
    }
    match v.get("type").and_then(Value::as_str) {
        Some("queue-operation") => match field(&v, "operation")? {
            "enqueue" => {
                field(&v, "content")?;
                Ok(Some("queue_enqueue"))
            }
            "dequeue" => Ok(Some("queue_dequeue")),
            _ => Err("unsupported_host_queue_operation".into()),
        },
        Some("atis-latch") => {
            field(&v, "atis")?;
            Ok(Some("atis_latch"))
        }
        Some("last-prompt") => {
            field(&v, "lastPrompt")?;
            label(field(&v, "leafUuid")?)?;
            Ok(Some("last_prompt"))
        }
        Some("attachment") => {
            if field(&v, "version")? != "2.1.260" {
                return Err("unsupported_host_record_version".into());
            }
            Ok(
                match v
                    .get("attachment")
                    .and_then(|a| a.get("type"))
                    .and_then(Value::as_str)
                {
                    Some("total_tokens_reminder") => Some("token_budget_attachment"),
                    Some("hook_additional_context") => Some("hook_context_attachment"),
                    _ => None,
                },
            )
        }
        Some("assistant") => {
            if field(&v, "version")? != "2.1.260" {
                return Err("unsupported_host_record_version".into());
            }
            let Some(message) = v.get("message") else {
                return Ok(None);
            };
            if message.get("role").and_then(Value::as_str) != Some("assistant")
                || message.get("stop_reason").and_then(Value::as_str) != Some("tool_use")
            {
                return Ok(None);
            }
            let Some(blocks) = message
                .get("content")
                .and_then(Value::as_array)
                .filter(|b| b.len() == 1)
            else {
                return Ok(None);
            };
            let block = &blocks[0];
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Bash")
            {
                return Ok(None);
            }
            label(field(block, "id")?)?;
            field(
                block.get("input").ok_or("unsupported_host_tool_input")?,
                "command",
            )?;
            Ok(Some("bash_request"))
        }
        _ => Ok(None),
    }
}
fn native_bash_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let (key, response) = match route {
        HostRoute::Live => {
            if field(v, "tool_name")? != "Bash" {
                return Err("unsupported_native_tool".into());
            }
            (
                field(v, "tool_use_id")?,
                v.get("tool_response")
                    .ok_or("missing_native_tool_response")?,
            )
        }
        HostRoute::History => {
            if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
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
            (
                field(&blocks[0], "tool_use_id")?,
                v.get("toolUseResult")
                    .ok_or("missing_native_tool_response")?,
            )
        }
    };
    label(key)?;
    let object = response
        .as_object()
        .ok_or("unsupported_native_tool_response")?;
    if object.len() != 5
        || ["stdout", "stderr"]
            .iter()
            .any(|key| !object.get(*key).is_some_and(Value::is_string))
        || ["interrupted", "isImage", "noOutputExpected"]
            .iter()
            .any(|key| object.get(*key).and_then(Value::as_bool).is_none())
    {
        return Err("unsupported_native_tool_response".into());
    }
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
fn native_file_tool_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let (key, response) = match route {
        HostRoute::Live => {
            if !file_tool_name(field(v, "tool_name")?) {
                return Err("unsupported_native_tool".into());
            }
            (
                field(v, "tool_use_id")?,
                v.get("tool_response")
                    .ok_or("missing_native_tool_response")?,
            )
        }
        HostRoute::History => {
            if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
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
            (
                field(&blocks[0], "tool_use_id")?,
                v.get("toolUseResult")
                    .ok_or("missing_native_tool_response")?,
            )
        }
    };
    label(key)?;
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
fn native_tool_event(
    grant: &HostCaptureGrant,
    v: &Value,
    route: HostRoute,
) -> Result<NormalizedHostEvent, String> {
    let name = match route {
        HostRoute::Live => field(v, "tool_name")?,
        HostRoute::History => {
            if let Some(response) = v.get("toolUseResult") {
                if response.as_object().is_some_and(|object| {
                    object.len() == 5
                        && ["stdout", "stderr"]
                            .iter()
                            .all(|key| object.get(*key).is_some_and(Value::is_string))
                        && ["interrupted", "isImage", "noOutputExpected"]
                            .iter()
                            .all(|key| object.get(*key).and_then(Value::as_bool).is_some())
                }) {
                    "Bash"
                } else {
                    history_tool_name(response)
                }
            } else {
                return Err("missing_native_tool_response".into());
            }
        }
    };
    if name == "Bash" {
        native_bash_event(grant, v, route)
    } else if file_tool_name(name) {
        native_file_tool_event(grant, v, route)
    } else {
        Err("unsupported_native_tool".into())
    }
}
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
    if raw.len() > MAX_CAPTURE_BYTES {
        return Err("capture_batch_byte_limit".into());
    }
    let v: Value =
        serde_json::from_slice(raw).map_err(|e| format!("malformed_host_record: {e}"))?;
    if !v.is_object() {
        return Err("unsupported_host_schema: expected object".into());
    }
    // Authority-bearing normalized envelopes are not native host records.
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
    let session = match route {
        HostRoute::Live => field(&v, "session_id")?,
        HostRoute::History => field(&v, "sessionId")?,
    };
    if session != context.session_id {
        return Err("host_session_mismatch".into());
    }
    if grant.adapter_version == CLAUDE_2_1_260_ADAPTER
        && route == HostRoute::History
        && field(&v, "version")? != "2.1.260"
    {
        return Err("unsupported_host_record_version".into());
    }
    if grant.adapter_version == CLAUDE_2_1_260_ADAPTER
        && ((route == HostRoute::Live
            && v.get("hook_event_name").and_then(Value::as_str) == Some("PostToolUse"))
            || (route == HostRoute::History
                && v.get("type").and_then(Value::as_str) == Some("user")
                && v.get("message").is_some_and(|m| {
                    m.get("role").and_then(Value::as_str) == Some("user")
                        && m.get("content").is_some_and(Value::is_array)
                })))
    {
        return native_tool_event(grant, &v, route);
    }
    let (kind, key, text) = match route {
        HostRoute::Live => {
            let name = field(&v, "hook_event_name")?;
            match name {
                "UserPromptSubmit" | "Stop" => {
                    let key = if name == "UserPromptSubmit" {
                        user_event_key(grant, context, route, &v)?
                    } else {
                        context
                            .original_event_key
                            .as_deref()
                            .ok_or("original_event_identity_required")?
                    };
                    if name == "UserPromptSubmit" {
                        (HostRecordKind::User, key, field(&v, "prompt")?)
                    } else {
                        (
                            HostRecordKind::Final,
                            key,
                            field(&v, "last_assistant_message")?,
                        )
                    }
                }
                "PostToolUse" => {
                    if v.get("tool_name")
                        .and_then(Value::as_str)
                        .is_some_and(file_tool_name)
                        && v.get("tool_response").is_some_and(Value::is_object)
                    {
                        return native_file_tool_event(grant, &v, route);
                    }
                    (
                        HostRecordKind::Tool,
                        field(&v, "tool_use_id")?,
                        field(&v, "tool_response")?,
                    )
                }
                _ => return Err(format!("unsupported_host_event: {name}")),
            }
        }
        HostRoute::History => {
            if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
                return Err("unsupported_host_sidechain".into());
            }
            let message = v
                .get("message")
                .ok_or("unsupported_host_schema: missing message")?;
            let content = message
                .get("content")
                .ok_or("unsupported_host_schema: missing content")?;
            match (field(&v, "type")?, field(message, "role")?) {
                ("user", "user") if content.is_string() => (
                    HostRecordKind::User,
                    user_event_key(grant, context, route, &v)?,
                    content.as_str().unwrap(),
                ),
                ("user", "user") => {
                    let blocks = content
                        .as_array()
                        .filter(|a| a.len() == 1)
                        .ok_or("unsupported_host_content_blocks")?;
                    if field(&blocks[0], "type")? != "tool_result" {
                        return Err("unsupported_host_content_block".into());
                    }
                    (
                        HostRecordKind::Tool,
                        field(&blocks[0], "tool_use_id")?,
                        field(&blocks[0], "content")?,
                    )
                }
                ("assistant", "assistant") => {
                    if field(message, "stop_reason")? != "end_turn" {
                        return Err("unsupported_host_nonfinal_assistant".into());
                    }
                    let blocks = content
                        .as_array()
                        .filter(|a| a.len() == 1)
                        .ok_or("unsupported_host_content_blocks")?;
                    if field(&blocks[0], "type")? != "text" {
                        return Err("unsupported_host_private_or_unknown_block".into());
                    }
                    (
                        HostRecordKind::Final,
                        field(&v, "uuid")?,
                        field(&blocks[0], "text")?,
                    )
                }
                _ => return Err("unsupported_host_record_type".into()),
            }
        }
    };
    label(key)?;
    if text.len() > grant.max_bytes {
        return Err("capture_byte_limit".into());
    }
    // Timestamp intentionally stays unknown: the hook and transcript often
    // disagree on which timestamp denotes original observation. Do not replace
    // unknown with capture/import time or create overlap conflicts.
    Ok(NormalizedHostEvent {
        kind,
        event: ObservationEvent {
            event_key: key.into(),
            text: text.into(),
            observed_at: None,
        },
    })
}
fn origin(context: &HostCaptureContext, key: &str) -> Result<HostOrigin, String> {
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

impl CortexRuntime {
    /// Explicit grants authorize only this finite adapter contract. There is no
    /// discovery, installation, host configuration change, or implicit grant.
    pub async fn register_host_capture(
        &self,
        cx: &Cx,
        grant: HostCaptureGrant,
    ) -> Result<(), String> {
        let probe = HostCaptureContext {
            host_version: grant.host_version.clone(),
            session_id: "validation".into(),
            generation: "validation".into(),
            original_event_key: None,
            origins: vec![],
        };
        validate(
            &grant,
            &probe,
            if grant.live {
                HostRoute::Live
            } else {
                HostRoute::History
            },
        )?;
        for kind in [
            HostRecordKind::User,
            HostRecordKind::Tool,
            HostRecordKind::Final,
        ] {
            self.register_source(
                cx,
                SourceSpec {
                    key: source_key(&grant.key, kind),
                    scope: grant.scope.clone(),
                    role: kind.role(),
                    max_bytes: grant.max_bytes,
                },
            )
            .await?;
        }
        self.register_source(
            cx,
            SourceSpec {
                key: cursor_key(&grant.key),
                scope: grant.scope.clone(),
                role: ObservationRole::DeliveryOnly,
                max_bytes: grant.max_bytes,
            },
        )
        .await?;
        let principal = self.observation_principal()?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        conn.execute_batch(DDL).map_err(|e| e.to_string())?;
        let serialized = serde_json::to_string(&grant).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO host_capture_grants VALUES(?1,?2,?3)",
            params![principal, grant.key, serialized],
        )
        .map_err(|e| e.to_string())?;
        let old: String = conn
            .query_row(
                "SELECT spec_json FROM host_capture_grants WHERE principal=?1 AND grant_key=?2",
                params![principal, grant.key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if old != serialized {
            return Err("host_registration_conflict".into());
        }
        Ok(())
    }
    pub async fn host_capture_offset(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
    ) -> Result<u64, String> {
        self.source_offset(cx, &cursor_key(grant_key), &generation(context))
            .await
    }
    /// Capture first, then mechanically maintain a scoped need from this supported event.
    /// Returned payload is for the adapter's supported context channel, never a store narration.
    pub async fn capture_and_prepare_host(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        raw: &[u8],
        invocation_context: &str,
        present_delivery: Option<&str>,
    ) -> Result<(HostCaptureReceipt, Option<super::cycle::PreparedView>), String> {
        let principal = self.observation_principal()?;
        let grant: HostCaptureGrant = {
            let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
            let spec: String = conn
                .query_row(
                    "SELECT spec_json FROM host_capture_grants WHERE principal=?1 AND grant_key=?2",
                    params![principal, grant_key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            serde_json::from_str(&spec).map_err(|e| e.to_string())?
        };
        validate(&grant, context, HostRoute::Live)?;
        let hook = live_hook_name(raw)?;
        let value: Value =
            serde_json::from_slice(raw).map_err(|e| format!("malformed_host_record: {e}"))?;
        let scope = observation_scope_for(&grant, &value);
        if hook.as_deref() == Some("PreToolUse") {
            if field(&value, "session_id")? != context.session_id {
                return Err("host_session_mismatch".into());
            }
            let input = value.get("tool_input").cloned().unwrap_or_else(|| json!({}));
            let cues = situation_cues(&input.to_string());
            let receipt = HostCaptureReceipt {
                adapter_version: grant.adapter_version.clone(),
                accepted: vec![],
                excluded_deliveries: 0,
                ignored_metadata: 0,
                next_offset: None,
                uncommitted_tail_bytes: 0,
            };
            if cues.is_empty() {
                return Ok((receipt, None));
            }
            let view = self
                .prepare_host_need(
                    cx,
                    grant_key,
                    context,
                    &scope,
                    cues,
                    invocation_context,
                    present_delivery,
                )
                .await?;
            return Ok((receipt, Some(view)));
        }
        let receipt = self.capture_host_event(cx, grant_key, context, raw).await?;
        if receipt.accepted.is_empty() || hook.as_deref() == Some("PreCompact") {
            return Ok((receipt, None));
        }
        let event = normalize_host_event(&grant, context, HostRoute::Live, raw)?;
        if event.kind == HostRecordKind::Final {
            return Ok((receipt, None));
        }
        let cues = situation_cues(&event.event.text);
        if cues.is_empty() {
            return Ok((receipt, None));
        }
        let view = self
            .prepare_host_need(
                cx,
                grant_key,
                context,
                &scope,
                cues,
                invocation_context,
                present_delivery,
            )
            .await?;
        Ok((receipt, Some(view)))
    }
    async fn prepare_host_need(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        scope: &str,
        cues: Vec<String>,
        invocation_context: &str,
        present_delivery: Option<&str>,
    ) -> Result<super::cycle::PreparedView, String> {
        let id = format!(
            "host-need:{}",
            cortex_logic::traces::content_hash(
                &json!([grant_key, context.session_id, context.generation]).to_string()
            )
        );
        self.subscribe_observations(
            cx,
            super::cycle::NeedSpec {
                id: id.clone(),
                scope: observation::normalize_scope(scope),
                cues: cues.clone(),
                exclude_cues: Vec::new(),
                max_results: 32,
                max_bytes: 32768,
                ttl_seconds: 3600,
                learned: false,
            },
        )
        .await?;
        let mut view = self
            .prepare_observations(cx, &id, invocation_context, present_delivery)
            .await?;
        if let Ok(compiled) = self
            .compile_assemblies(cx, scope, &cues, 4, None, None, None, "")
            .await
        {
            view.assembly_brief = compiled.brief;
        }
        Ok(view)
    }
    pub async fn capture_host_event(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        raw: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        self.capture_host_batch(cx, grant_key, context, HostRoute::Live, None, raw)
            .await
    }
    /// `chunk` starts exactly at the durable raw JSONL byte cursor. Complete
    /// records, duplicates, exclusions, and the RAW (not normalized) byte cursor
    /// commit together. Malformed/unknown records roll the whole batch back.
    /// Caller must safely open an authorized file; this API accepts bytes only.
    pub async fn tail_host_transcript(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        start: u64,
        chunk: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        self.capture_host_batch(
            cx,
            grant_key,
            context,
            HostRoute::History,
            Some(start),
            chunk,
        )
        .await
    }
    async fn capture_host_batch(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        route: HostRoute,
        start: Option<u64>,
        raw: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        if raw.len() > MAX_CAPTURE_BYTES {
            return Err("capture_batch_byte_limit".into());
        }
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        observation::ensure(&conn)?;
        conn.execute_batch(DDL).map_err(|e| e.to_string())?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let stored: String = tx
            .query_row(
                "SELECT spec_json FROM host_capture_grants WHERE principal=?1 AND grant_key=?2",
                params![principal, grant_key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("host_capture_not_authorized")?;
        let grant: HostCaptureGrant = serde_json::from_str(&stored).map_err(|e| e.to_string())?;
        validate(&grant, context, route)?;
        let cursor = cursor_key(grant_key);
        let generation = generation(context);
        observation::granted(&tx, &principal, &cursor, true)?;
        let cutoff = if start.is_some() {
            raw.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
        } else {
            raw.len()
        };
        let next = if let Some(start) = start {
            if observation::offset(&tx, &principal, &cursor, &generation)? != start {
                return Err("cursor_conflict".into());
            }
            Some(
                start
                    .checked_add(cutoff as u64)
                    .filter(|v| *v <= i64::MAX as u64)
                    .ok_or("invalid_source_cursor")?,
            )
        } else {
            None
        };
        let lines: Vec<&[u8]> = if start.is_some() {
            raw[..cutoff].split_inclusive(|b| *b == b'\n').collect()
        } else {
            vec![raw]
        };
        if lines.len() > MAX_BATCH_EVENTS {
            return Err("capture_batch_event_limit".into());
        }
        let mut result = HostCaptureReceipt {
            adapter_version: grant.adapter_version.clone(),
            accepted: vec![],
            excluded_deliveries: 0,
            ignored_metadata: 0,
            next_offset: next,
            uncommitted_tail_bytes: raw.len() - cutoff,
        };
        let mut raw_offset = start.unwrap_or(0);
        for line in lines {
            let line_start = raw_offset;
            raw_offset += line.len() as u64;
            if route == HostRoute::History {
                cx.checkpoint().map_err(|e| e.to_string())?;
                if let Some(kind) = metadata_kind(&grant, context, line)? {
                    let digest: String = Sha256::digest(line)
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect();
                    tx.execute(
                        "INSERT INTO host_capture_metadata VALUES(?1,?2,?3,?4,?5,?6,?7)",
                        params![
                            principal,
                            grant_key,
                            generation,
                            line_start as i64,
                            kind,
                            line.len() as i64,
                            digest
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    result.ignored_metadata += 1;
                    continue;
                }
            }
            if route == HostRoute::Live && live_hook_name(line)?.as_deref() == Some("PreCompact") {
                let value: Value = serde_json::from_slice(line)
                    .map_err(|e| format!("malformed_host_record: {e}"))?;
                if field(&value, "session_id")? != context.session_id {
                    return Err("host_session_mismatch".into());
                }
                field(&value, "trigger")?;
                let next: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(byte_offset),-1)+1 FROM host_capture_metadata WHERE principal=?1 AND grant_key=?2 AND generation=?3",
                        params![principal, grant_key, generation],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let digest: String = Sha256::digest(line)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                tx.execute(
                    "INSERT INTO host_capture_metadata VALUES(?1,?2,?3,?4,?5,?6,?7)",
                    params![
                        principal,
                        grant_key,
                        generation,
                        next,
                        "precompact",
                        line.len() as i64,
                        digest
                    ],
                )
                .map_err(|e| e.to_string())?;
                result.ignored_metadata += 1;
                continue;
            }
            cx.checkpoint().map_err(|e| e.to_string())?;
            let normalized = normalize_host_event(&grant, context, route, line)?;
            let source = if let Some(existing) = existing_host_source(
                &tx,
                &principal,
                &generation,
                &normalized.event.event_key,
                grant_key,
                normalized.kind,
            )? {
                existing
            } else {
                let scope = observation_scope_for_raw(&grant, line);
                let source = scoped_source_key(&grant, normalized.kind, &scope);
                observation::ensure_source(
                    &tx,
                    &principal,
                    &SourceSpec {
                        key: source.clone(),
                        scope,
                        role: normalized.kind.role(),
                        max_bytes: grant.max_bytes,
                    },
                )?;
                source
            };
            let registered = observation::granted(&tx, &principal, &source, true)?;
            if registered.role != normalized.kind.role_name() {
                return Err("host_source_role_mismatch".into());
            }
            let resolved = origin(context, &normalized.event.event_key)?;
            let origin_name = if resolved == HostOrigin::CortexDelivery {
                "cortex_delivery"
            } else {
                "external"
            };
            let previous: Option<(String,String)> = tx.query_row("SELECT kind,origin FROM host_capture_origins WHERE principal=?1 AND grant_key=?2 AND generation=?3 AND event_key=?4",params![principal,grant_key,generation,normalized.event.event_key],|r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e| e.to_string())?;
            if previous.as_ref().is_some_and(|(kind, origin)| {
                kind != normalized.kind.role_name() || origin != origin_name
            }) {
                return Err("host_origin_identity_conflict".into());
            }
            tx.execute(
                "INSERT OR IGNORE INTO host_capture_origins VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    principal,
                    grant_key,
                    generation,
                    normalized.event.event_key,
                    normalized.kind.role_name(),
                    origin_name
                ],
            )
            .map_err(|e| e.to_string())?;
            if resolved == HostOrigin::CortexDelivery {
                result.excluded_deliveries += 1;
                continue;
            }
            result.accepted.push(observation::capture(
                &tx,
                &principal,
                &source,
                &generation,
                &registered,
                normalized.event,
            )?);
        }
        if let Some(next) = next {
            tx.execute("INSERT INTO observation_cursors VALUES(?1,?2,?3,?4) ON CONFLICT(principal,source_key,generation) DO UPDATE SET byte_offset=excluded.byte_offset",params![principal,cursor,generation,next as i64]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}
