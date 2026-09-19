//! Fail-closed host capture. The fixture adapter accepts string tool results;
//! the native adapter also accepts structured Bash and file-tool reports.
//! Neither establishes reader quality or compatibility with every host tool.
//! Callers supply authenticated invocation metadata and origin sidecars separately
//! from host JSON. This module never opens a path supplied by a hook.
use super::observation::{
    self, MAX_CAPTURE_BYTES, ObservationEvent, ObservationReceipt, ObservationRole, SourceSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
mod helpers;
mod native;
mod normalize;
mod runtime;
pub use helpers::resolve_host_invocation;
use helpers::*;
pub use normalize::normalize_host_event;

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
    json!({"grant":INSTALLED_HOST_GRANT_KEY,"host_version":"2.1.260","native_user_prompts":true,"native_bash_results":true,"native_file_results":true,"native_compactions":true})
}
const DDL: &str = "CREATE TABLE IF NOT EXISTS host_capture_grants (principal TEXT NOT NULL, grant_key TEXT NOT NULL, spec_json TEXT NOT NULL, PRIMARY KEY(principal,grant_key)); CREATE TABLE IF NOT EXISTS host_capture_origins (principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL, event_key TEXT NOT NULL, kind TEXT NOT NULL, origin TEXT NOT NULL, PRIMARY KEY(principal,grant_key,generation,event_key)); CREATE TABLE IF NOT EXISTS host_capture_metadata (principal TEXT NOT NULL, grant_key TEXT NOT NULL, generation TEXT NOT NULL, byte_offset INTEGER NOT NULL, kind TEXT NOT NULL, byte_length INTEGER NOT NULL, digest TEXT NOT NULL, PRIMARY KEY(principal,grant_key,generation,byte_offset));";

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
        self.role().as_str()
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
