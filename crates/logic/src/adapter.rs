//! Adapter capability manifest, the EventFrame hook protocol and explicit
//! degradation. A host declares what it can honestly deliver; the hook
//! decision is a pure function of the frame and the snapshot state. Hooks
//! never carry admission or truth rules: they route, they do not decide what
//! memory is true.

use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CapabilityManifest {
    #[serde(default = "manifest_version")]
    pub version: u32,
    pub adapter: String,
    #[serde(default)]
    pub observe_prompt_delta: bool,
    #[serde(default)]
    pub observe_tool_result: bool,
    #[serde(default)]
    pub observe_artifact_revision: bool,
    #[serde(default)]
    pub inject_context: bool,
    #[serde(default)]
    pub attest_context_presence: bool,
    #[serde(default)]
    pub observe_compaction: bool,
    #[serde(default)]
    pub checkpoint_on_transition: bool,
    #[serde(default)]
    pub replace_context_spans: bool,
    #[serde(default)]
    pub emit_outcome_receipts: bool,
    #[serde(default)]
    pub native_in_process: bool,
    #[serde(default)]
    pub durable_local_capture: bool,
    #[serde(default)]
    pub cancellation: bool,
    #[serde(default)]
    pub read_only: bool,
    /// Largest input the host hands to one hook invocation, in bytes.
    #[serde(default = "default_max_input")]
    pub max_input_bytes: usize,
    /// Largest context the host accepts back from one delivery, in bytes.
    #[serde(default = "default_max_delivery")]
    pub max_delivery_bytes: usize,
}

fn manifest_version() -> u32 {
    MANIFEST_VERSION
}
fn default_max_input() -> usize {
    256 * 1024
}
fn default_max_delivery() -> usize {
    16 * 1024
}

impl Default for CapabilityManifest {
    fn default() -> Self {
        Self::base("unknown")
    }
}

impl CapabilityManifest {
    fn base(adapter: &str) -> Self {
        Self {
            version: MANIFEST_VERSION,
            adapter: adapter.into(),
            observe_prompt_delta: false,
            observe_tool_result: false,
            observe_artifact_revision: false,
            inject_context: false,
            attest_context_presence: false,
            observe_compaction: false,
            checkpoint_on_transition: false,
            replace_context_spans: false,
            emit_outcome_receipts: false,
            native_in_process: false,
            durable_local_capture: false,
            cancellation: false,
            read_only: false,
            max_input_bytes: default_max_input(),
            max_delivery_bytes: default_max_delivery(),
        }
    }
    /// The daemon / library surface itself: everything a caller does is
    /// explicit, so nothing is *observed* but every write is durable.
    pub fn native() -> Self {
        Self {
            inject_context: true,
            attest_context_presence: true,
            checkpoint_on_transition: true,
            emit_outcome_receipts: true,
            native_in_process: true,
            durable_local_capture: true,
            ..Self::base("native")
        }
    }
    /// A host that only exposes tools: the agent-initiated path, every token counted.
    pub fn tools_only(adapter: &str) -> Self {
        Self {
            emit_outcome_receipts: true,
            ..Self::base(adapter)
        }
    }
    /// The Claude Code plugin with SessionStart, PostToolUse, PreCompact and
    /// UserPromptSubmit hooks wired. It cannot attest that injected context
    /// survived compaction, and it cannot replace spans.
    pub fn claude_code_plugin() -> Self {
        Self {
            observe_prompt_delta: true,
            observe_tool_result: true,
            inject_context: true,
            observe_compaction: true,
            checkpoint_on_transition: true,
            emit_outcome_receipts: true,
            durable_local_capture: true,
            ..Self::base("claude-code-plugin")
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionStart,
    PromptDelta,
    ToolResult,
    ArtifactRevision,
    Compaction,
    Handoff,
    NewTurn,
    SessionEnd,
}

impl EventKind {
    pub const ALL: [Self; 8] = [
        Self::SessionStart,
        Self::PromptDelta,
        Self::ToolResult,
        Self::ArtifactRevision,
        Self::Compaction,
        Self::Handoff,
        Self::NewTurn,
        Self::SessionEnd,
    ];
    pub fn parse(raw: &str) -> Option<Self> {
        Some(
            match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
                "session_start" | "sessionstart" => Self::SessionStart,
                "prompt_delta" | "userpromptsubmit" | "user_prompt_submit" => Self::PromptDelta,
                "tool_result" | "posttooluse" | "post_tool_use" => Self::ToolResult,
                "artifact_revision" => Self::ArtifactRevision,
                "compaction" | "precompact" | "pre_compact" => Self::Compaction,
                "handoff" => Self::Handoff,
                "new_turn" => Self::NewTurn,
                "session_end" | "sessionend" | "stop" => Self::SessionEnd,
                _ => return None,
            },
        )
    }
    /// The manifest flag a host must hold for this event to be *observed*.
    pub(super) fn required_flag(self, m: &CapabilityManifest) -> Option<(&'static str, bool)> {
        match self {
            Self::PromptDelta | Self::NewTurn => {
                Some(("observe_prompt_delta", m.observe_prompt_delta))
            }
            Self::ToolResult => Some(("observe_tool_result", m.observe_tool_result)),
            Self::ArtifactRevision => {
                Some(("observe_artifact_revision", m.observe_artifact_revision))
            }
            Self::Compaction | Self::Handoff => Some(("observe_compaction", m.observe_compaction)),
            Self::SessionStart | Self::SessionEnd => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EventFrame {
    pub kind: Option<EventKind>,
    /// Prompt delta or tool output handed to this invocation (may be empty).
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub input_bytes: usize,
    /// Typed needs the host or agent declared for this event.
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub principal: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub artifact_revisions: Vec<String>,
    /// Host context epoch: changes when the host rewrites its context.
    #[serde(default)]
    pub context_epoch: Option<String>,
    #[serde(default)]
    pub invocation_id: String,
    /// Whether the host requires a durable commit before it proceeds.
    #[serde(default)]
    pub commit_required: bool,
    pub capabilities: CapabilityManifest,
}

/// What the hook knows about the warm snapshot / delivery source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotState {
    Fresh,
    /// A snapshot exists but its frontier or policy epoch is stale.
    Expired,
    ProjectionPending,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HookDecision {
    Noop,
    Deliver,
    QueryRequired,
    ProjectionPending,
    Unavailable,
}

/// Presence of previously delivered context in the host's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookOutcome {
    pub decision: HookDecision,
    pub reason: String,
    /// Input exceeded the manifest bound; it was routed, not dropped silently.
    pub overflow: bool,
    pub presence: Presence,
    /// The host may claim automatic capture only when it observed the source.
    pub automatic_capture: bool,
    /// Whether this action changes reasoning and therefore counts tokens.
    pub counted: bool,
}

/// Actions that never spend model tokens: they change what is *available*,
/// not what the model reasons about. Everything else is represented and counted.
pub const ZERO_TOKEN_ACTIONS: [&str; 6] = [
    "duplicate_capture_avoidance",
    "brain_selection",
    "revision_validation",
    "cached_result_rejection",
    "authorized_read_only_defaults",
    "artifact_loading_by_reference",
];

mod decide;
pub use decide::{decide, degradation_matrix};
mod ergonomics;
pub use ergonomics::{ContextReplacement, ErgonomicsTally, propose_replacements};
