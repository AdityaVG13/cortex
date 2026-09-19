//! `cortex hook <kind>`: the live observation entry. Reads the host payload
//! from stdin. When `CORTEX_CAPTURE` or `$CORTEX_HOME/capture.json` selects
//! this event, it captures and prepares through `host_capture`. Missing or
//! non-matching sidecars stay silent. CQR orient / deposit / checkpoint stay
//! on `process()`, `cortex hook-boot`, `cortex op`, and MCP.

use crate::adapter::{CapabilityManifest, EventFrame, EventKind, HookOutcome};
use crate::protocol::{CWD_KEYS, arg_str, nonempty_owned};
use serde_json::{Value, json};

pub(super) const DELIVERY_BUDGET: usize = 1200;

/// One processed host event, independent of process I/O so tests can
/// inspect the effective prompt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HookResult {
    pub event: Option<EventKind>,
    pub outcome: HookOutcome,
    /// Text handed back to the host as additional context (may be empty).
    pub additional_context: String,
    /// Deposit receipt id when a capture was committed durably.
    pub capture_receipt: Option<String>,
    pub capture_key: Option<String>,
    pub checkpoint: Option<Value>,
    /// Reflex Level-0 accounting: snapshot state, whether the warm path
    /// answered, and whether the deep path was needed (fallback).
    pub reflex: Value,
    /// Proposed context replacements. Only ever non-empty when the host
    /// declares `replace_context_spans`; always host-verified, never mandatory.
    pub context_replace: Vec<Value>,
}

impl HookResult {
    pub fn host_envelope(&self, host_event_name: &str) -> Value {
        json!({"hookSpecificOutput":{"hookEventName":host_event_name,"additionalContext":self.additional_context},"cortex":{"event":self.event,"decision":self.outcome.decision,"reason":self.outcome.reason,"overflow":self.outcome.overflow,"presence":self.outcome.presence,"automatic_capture":self.outcome.automatic_capture,"counted":self.outcome.counted,"capture_receipt":self.capture_receipt,"capture_key":self.capture_key,"checkpoint":self.checkpoint,"reflex":self.reflex,"context_replace":self.context_replace}})
    }
}

/// Build the frame from a Claude-Code-shaped payload (`hook_event_name`,
/// `tool_name`, `tool_input`, `tool_response`, `prompt`, `session_id`, `cwd`,
/// `cwd_path`, `working_directory`).
pub fn frame_from_host(
    kind_hint: &str,
    payload: &Value,
    manifest: CapabilityManifest,
) -> EventFrame {
    let kind = EventKind::parse(kind_hint)
        .or_else(|| arg_str(payload, &["hook_event_name"]).and_then(EventKind::parse));
    let input = match kind {
        Some(EventKind::ToolResult) => payload
            .get("tool_response")
            .map(|r| {
                if let Some(t) = r.as_str() {
                    t.to_string()
                } else {
                    nonempty_owned(
                        ["stdout", "output", "content", "result", "stderr"]
                            .iter()
                            .filter_map(|k| r.get(k).and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                            .trim()
                            .to_string(),
                    )
                    .unwrap_or_else(|| r.to_string())
                }
            })
            .unwrap_or_default(),
        Some(EventKind::PromptDelta | EventKind::NewTurn) => {
            arg_str(payload, &["prompt", "user_prompt"])
                .unwrap_or("")
                .to_string()
        }
        Some(EventKind::SessionStart) => arg_str(payload, &["prompt", "cwd", "source"])
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let needs = match kind {
        Some(EventKind::SessionStart | EventKind::NewTurn | EventKind::PromptDelta) => vec![
            "current_constraints".into(),
            "open_obligations".into(),
            "failed_attempts".into(),
        ],
        _ => Vec::new(),
    };
    EventFrame {
        kind,
        input_bytes: input.len(),
        input,
        needs,
        principal: "solo".into(),
        scope: arg_str(payload, CWD_KEYS).unwrap_or("").to_string(),
        thread: arg_str(payload, &["session_id", "thread"]).map(|t| format!("session:{t}")),
        artifact_revisions: Vec::new(),
        context_epoch: arg_str(payload, &["context_epoch"]).map(str::to_string),
        invocation_id: arg_str(payload, &["invocation_id", "tool_use_id"])
            .unwrap_or("")
            .to_string(),
        commit_required: kind == Some(EventKind::Compaction),
        capabilities: manifest,
    }
}

mod process;
pub use process::process;
mod sidecar;
pub use sidecar::{load_capture_sidecar, run, run_with_paths, write_installed_capture_sidecar};
