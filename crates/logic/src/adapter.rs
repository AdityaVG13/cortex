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
    fn required_flag(self, m: &CapabilityManifest) -> Option<(&'static str, bool)> {
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

pub fn decide(frame: &EventFrame, snapshot: SnapshotState) -> HookOutcome {
    let m = &frame.capabilities;
    let overflow = frame.input_bytes > m.max_input_bytes;
    let presence = if m.attest_context_presence {
        Presence::Present
    } else if m.observe_compaction
        && matches!(frame.kind, Some(EventKind::Compaction | EventKind::Handoff))
    {
        Presence::Absent
    } else {
        Presence::Unknown
    };
    let outcome = |decision: HookDecision,
                   reason: String,
                   automatic_capture: bool,
                   counted: bool| HookOutcome {
        decision,
        reason,
        overflow,
        presence,
        automatic_capture,
        counted,
    };
    let Some(kind) = frame.kind else {
        return outcome(
            HookDecision::Noop,
            "unknown event kind".into(),
            false,
            false,
        );
    };
    if let Some((flag, held)) = kind.required_flag(m) {
        if !held {
            return outcome(
                HookDecision::Noop,
                format!("host does not declare {flag}; nothing observed, nothing claimed"),
                false,
                false,
            );
        }
    }
    if kind == EventKind::ToolResult {
        if m.read_only {
            return outcome(
                HookDecision::Noop,
                "read_only adapter: capture disabled".into(),
                false,
                false,
            );
        }
        return outcome(
            HookDecision::Deliver,
            "tool result observed: deterministic capture".into(),
            m.durable_local_capture,
            false,
        );
    }
    if matches!(kind, EventKind::Compaction | EventKind::Handoff) {
        return if m.checkpoint_on_transition {
            outcome(
                HookDecision::Deliver,
                "transition observed: checkpoint Thread state".into(),
                false,
                false,
            )
        } else {
            outcome(
                HookDecision::Noop,
                "host cannot checkpoint on transition".into(),
                false,
                false,
            )
        };
    }
    if kind == EventKind::SessionEnd {
        return outcome(
            HookDecision::Noop,
            "session end: outcome receipts only".into(),
            false,
            false,
        );
    }
    if !m.inject_context {
        return outcome(
            HookDecision::QueryRequired,
            "host cannot inject context: agent-initiated path, tokens counted".into(),
            false,
            true,
        );
    }
    match snapshot {
        SnapshotState::Unavailable => outcome(
            HookDecision::Unavailable,
            "memory unavailable: say so, never pretend emptiness".into(),
            false,
            false,
        ),
        SnapshotState::ProjectionPending => outcome(
            HookDecision::ProjectionPending,
            "projections behind the commit frontier".into(),
            false,
            false,
        ),
        SnapshotState::Expired => outcome(
            HookDecision::QueryRequired,
            "snapshot expired: an expired snapshot is not no memory".into(),
            false,
            true,
        ),
        SnapshotState::Fresh => {
            if frame.needs.is_empty() && kind == EventKind::PromptDelta {
                outcome(
                    HookDecision::Noop,
                    "no needs on this delta".into(),
                    false,
                    false,
                )
            } else {
                outcome(
                    HookDecision::Deliver,
                    format!("deliver for {kind:?}"),
                    false,
                    true,
                )
            }
        }
    }
}

/// The degradation matrix for one manifest: every event kind against the
/// fresh-snapshot decision, so a host sees exactly what it loses.
pub fn degradation_matrix(manifest: &CapabilityManifest) -> Vec<(EventKind, HookOutcome)> {
    EventKind::ALL
        .iter()
        .map(|kind| {
            let frame = EventFrame {
                kind: Some(*kind),
                needs: vec!["current_constraints".into()],
                capabilities: manifest.clone(),
                ..Default::default()
            };
            (*kind, decide(&frame, SnapshotState::Fresh))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: EventKind, m: CapabilityManifest) -> EventFrame {
        EventFrame {
            kind: Some(kind),
            needs: vec!["current_constraints".into()],
            capabilities: m,
            ..Default::default()
        }
    }

    #[test]
    fn missing_tool_result_means_no_automatic_capture_claim() {
        let out = decide(
            &frame(
                EventKind::ToolResult,
                CapabilityManifest::tools_only("mcp-only"),
            ),
            SnapshotState::Fresh,
        );
        assert_eq!(out.decision, HookDecision::Noop);
        assert!(!out.automatic_capture);
        let out = decide(
            &frame(
                EventKind::ToolResult,
                CapabilityManifest::claude_code_plugin(),
            ),
            SnapshotState::Fresh,
        );
        assert_eq!(out.decision, HookDecision::Deliver);
        assert!(out.automatic_capture);
        assert!(!out.counted, "capture spends no model tokens");
    }

    #[test]
    fn missing_compaction_means_presence_unknown() {
        let out = decide(
            &frame(EventKind::NewTurn, CapabilityManifest::tools_only("x")),
            SnapshotState::Fresh,
        );
        assert_eq!(out.presence, Presence::Unknown);
        assert_eq!(
            out.decision,
            HookDecision::Noop,
            "an unobserved turn is not a delivery opportunity"
        );
        let out = decide(
            &frame(EventKind::SessionStart, CapabilityManifest::tools_only("x")),
            SnapshotState::Fresh,
        );
        assert_eq!(
            out.decision,
            HookDecision::QueryRequired,
            "tools-only hosts get the agent-initiated path"
        );
        assert!(out.counted);
        let out = decide(
            &frame(
                EventKind::Compaction,
                CapabilityManifest::claude_code_plugin(),
            ),
            SnapshotState::Fresh,
        );
        assert_eq!(out.presence, Presence::Absent);
        assert_eq!(out.decision, HookDecision::Deliver);
    }

    #[test]
    fn expired_snapshot_is_not_no_memory_and_overflow_is_flagged() {
        let mut f = frame(
            EventKind::SessionStart,
            CapabilityManifest::claude_code_plugin(),
        );
        assert_eq!(
            decide(&f, SnapshotState::Expired).decision,
            HookDecision::QueryRequired
        );
        assert_eq!(
            decide(&f, SnapshotState::Unavailable).decision,
            HookDecision::Unavailable
        );
        assert_eq!(
            decide(&f, SnapshotState::ProjectionPending).decision,
            HookDecision::ProjectionPending
        );
        f.input_bytes = f.capabilities.max_input_bytes + 1;
        let out = decide(&f, SnapshotState::Fresh);
        assert!(out.overflow);
        assert_eq!(out.decision, HookDecision::Deliver);
    }

    #[test]
    fn matrix_and_parse_cover_every_kind() {
        let m = degradation_matrix(&CapabilityManifest::tools_only("t"));
        assert_eq!(m.len(), EventKind::ALL.len());
        assert!(
            m.iter().all(|(_, o)| o.decision != HookDecision::Deliver),
            "{m:?}"
        );
        assert_eq!(EventKind::parse("PostToolUse"), Some(EventKind::ToolResult));
        assert_eq!(EventKind::parse("PreCompact"), Some(EventKind::Compaction));
        assert_eq!(EventKind::parse("nope"), None);
        let json = CapabilityManifest::native().to_json();
        assert_eq!(json["native_in_process"], true);
        assert_eq!(json["version"], MANIFEST_VERSION);
    }
}

/// Ergonomic benchmark tally for an unfamiliar agent's first session:
/// counted events, rendered in three shapes so prose, JSON and terse
/// deliveries can be compared on the same run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErgonomicsTally {
    pub first_useful_orientation_ms: Option<u64>,
    pub schema_tokens: usize,
    pub mistaken_calls: usize,
    pub unresolved_aliases: usize,
    pub unsupported_completions: usize,
    pub expansions: usize,
    pub success: bool,
}

impl ErgonomicsTally {
    pub fn render(&self, shape: &str) -> String {
        match shape {
            "json" => serde_json::to_string(self).unwrap_or_default(),
            "terse" => format!(
                "orient={}ms schema={} mistakes={} aliases={} unsupported={} expansions={} success={}",
                self.first_useful_orientation_ms.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
                self.schema_tokens, self.mistaken_calls, self.unresolved_aliases, self.unsupported_completions, self.expansions, self.success
            ),
            _ => format!(
                "First useful orientation after {}; {} schema tokens read; {} mistaken calls; {} unresolved aliases; {} unsupported completions; {} expansions; task {}.",
                self.first_useful_orientation_ms.map(|v| format!("{v} ms")).unwrap_or_else(|| "never".into()),
                self.schema_tokens, self.mistaken_calls, self.unresolved_aliases, self.unsupported_completions, self.expansions,
                if self.success { "succeeded" } else { "did not succeed" }
            ),
        }
    }
}

/// A proposed context replacement: spans the host may swap for a bundle or
/// expansion handle. Always host-verified, never mandatory, and only ever
/// proposed to hosts that declare `replace_context_spans`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextReplacement {
    pub span_start: usize,
    pub span_end: usize,
    pub replacement: String,
    pub reason: String,
    pub bytes_saved: i64,
}

pub fn propose_replacements(
    manifest: &CapabilityManifest,
    candidates: Vec<ContextReplacement>,
) -> Vec<ContextReplacement> {
    if manifest.replace_context_spans {
        candidates
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod ergonomics_tests {
    use super::*;

    #[test]
    fn tally_renders_three_shapes_and_replacements_respect_the_manifest() {
        let t = ErgonomicsTally {
            first_useful_orientation_ms: Some(420),
            schema_tokens: 310,
            mistaken_calls: 1,
            unresolved_aliases: 0,
            unsupported_completions: 0,
            expansions: 2,
            success: true,
        };
        assert!(t.render("prose").contains("420 ms"));
        assert!(t.render("terse").starts_with("orient=420ms"));
        assert_eq!(
            serde_json::from_str::<ErgonomicsTally>(&t.render("json")).unwrap(),
            t
        );
        let c = vec![ContextReplacement {
            span_start: 0,
            span_end: 100,
            replacement: "m1".into(),
            reason: "bundle".into(),
            bytes_saved: 98,
        }];
        assert!(
            propose_replacements(&CapabilityManifest::claude_code_plugin(), c.clone()).is_empty(),
            "plugin cannot replace spans"
        );
        let mut can = CapabilityManifest::native();
        can.replace_context_spans = true;
        assert_eq!(propose_replacements(&can, c.clone()), c);
    }
}
