//! Versioned, fail-closed host capture. The legacy adapter is fixture-defined;
//! the pinned 2.1.260 subset follows an isolated installed-host protocol probe.
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
    /// Transcript UUID for live final and legacy user records. Native 2.1.260
    /// user events carry prompt_id; this field optionally checks that identity.
    /// Never synthesize an identity from content or import time.
    pub original_event_key: Option<String>,
    pub origins: Vec<HostOriginBinding>,
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
/// Normalize evidence records, not transcript control frames. The legacy adapter
/// accepts user/tool strings and a single final assistant text block. The pinned
/// native adapter additionally uses prompt_id/promptId and structured Bash reports.
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
        return native_bash_event(grant, &v, route);
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
                "PostToolUse" => (
                    HostRecordKind::Tool,
                    field(&v, "tool_use_id")?,
                    field(&v, "tool_response")?,
                ),
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
        let receipt = self.capture_host_event(cx, grant_key, context, raw).await?;
        if receipt.accepted.is_empty() {
            return Ok((receipt, None));
        }
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
        let event = normalize_host_event(&grant, context, HostRoute::Live, raw)?;
        if event.kind == HostRecordKind::Final {
            return Ok((receipt, None));
        }
        let situation: std::borrow::Cow<'_, str> = if grant.adapter_version
            == CLAUDE_2_1_260_ADAPTER
            && event.kind == HostRecordKind::Tool
        {
            let report: Value =
                serde_json::from_str(&event.event.text).map_err(|e| e.to_string())?;
            std::borrow::Cow::Owned(format!(
                "{}\n{}",
                field(&report, "stdout")?,
                field(&report, "stderr")?
            ))
        } else {
            std::borrow::Cow::Borrowed(&event.event.text)
        };
        let cues: Vec<String> = situation
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .map(str::to_lowercase)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(32)
            .collect();
        if cues.is_empty() {
            return Ok((receipt, None));
        }
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
                scope: grant.scope,
                cues,
                max_results: 32,
                max_bytes: 32768,
                ttl_seconds: 3600,
                learned: false,
            },
        )
        .await?;
        let view = self
            .prepare_observations(cx, &id, invocation_context, present_delivery)
            .await?;
        Ok((receipt, Some(view)))
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
            cx.checkpoint().map_err(|e| e.to_string())?;
            let normalized = normalize_host_event(&grant, context, route, line)?;
            let source = source_key(grant_key, normalized.kind);
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
