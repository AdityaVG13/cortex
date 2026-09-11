//! `cortex hook <kind>`: the live observation entry. Reads the host payload
//! from stdin. When `CORTEX_CAPTURE` or `$CORTEX_HOME/capture.json` selects
//! this event, it captures and prepares through `host_capture`. Missing or
//! non-matching sidecars stay silent. CQR orient / deposit / checkpoint stay
//! on `process()`, `cortex hook-boot`, `cortex op`, and MCP.

use crate::adapter::{
    CapabilityManifest, EventFrame, EventKind, HookDecision, HookOutcome, SnapshotState, decide,
};
use crate::capture::parse_tool_result;
use crate::handlers::operations::{Caller, Operation, dispatch};
use crate::runtime::CortexRuntime;
use serde_json::{Value, json};

const DELIVERY_BUDGET: usize = 1200;

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
        json!({
            "hookSpecificOutput": {"hookEventName": host_event_name, "additionalContext": self.additional_context},
            "cortex": {
                "event": self.event,
                "decision": self.outcome.decision,
                "reason": self.outcome.reason,
                "overflow": self.outcome.overflow,
                "presence": self.outcome.presence,
                "automatic_capture": self.outcome.automatic_capture,
                "counted": self.outcome.counted,
                "capture_receipt": self.capture_receipt,
                "capture_key": self.capture_key,
                "checkpoint": self.checkpoint,
                "reflex": self.reflex,
                "context_replace": self.context_replace,
            }
        })
    }
}

trait ThenNonEmpty {
    fn then_nonempty(self) -> Option<String>;
}
impl ThenNonEmpty for String {
    fn then_nonempty(self) -> Option<String> {
        if self.is_empty() { None } else { Some(self) }
    }
}

fn s<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| v.get(k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|x| !x.is_empty())
}

/// Build the frame from a Claude-Code-shaped payload (`hook_event_name`,
/// `tool_name`, `tool_input`, `tool_response`, `prompt`, `session_id`, `cwd`).
pub fn frame_from_host(
    kind_hint: &str,
    payload: &Value,
    manifest: CapabilityManifest,
) -> EventFrame {
    let kind = EventKind::parse(kind_hint)
        .or_else(|| s(payload, &["hook_event_name"]).and_then(EventKind::parse));
    let input = match kind {
        Some(EventKind::ToolResult) => payload
            .get("tool_response")
            .map(|r| {
                if let Some(t) = r.as_str() {
                    t.to_string()
                } else {
                    // Prefer the observed text fields; fall back to the raw object.
                    ["stdout", "output", "content", "result", "stderr"]
                        .iter()
                        .filter_map(|k| r.get(k).and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim()
                        .to_string()
                        .then_nonempty()
                        .unwrap_or_else(|| r.to_string())
                }
            })
            .unwrap_or_default(),
        Some(EventKind::PromptDelta | EventKind::NewTurn) => s(payload, &["prompt", "user_prompt"])
            .unwrap_or("")
            .to_string(),
        Some(EventKind::SessionStart) => s(payload, &["prompt", "cwd", "source"])
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    };
    let needs = match kind {
        Some(EventKind::SessionStart | EventKind::NewTurn | EventKind::PromptDelta) => {
            vec![
                "current_constraints".into(),
                "open_obligations".into(),
                "failed_attempts".into(),
            ]
        }
        _ => Vec::new(),
    };
    EventFrame {
        kind,
        input_bytes: input.len(),
        input,
        needs,
        principal: "solo".into(),
        scope: s(payload, &["cwd"]).unwrap_or("").to_string(),
        thread: s(payload, &["session_id", "thread"]).map(|t| format!("session:{t}")),
        artifact_revisions: Vec::new(),
        context_epoch: s(payload, &["context_epoch"]).map(str::to_string),
        invocation_id: s(payload, &["invocation_id", "tool_use_id"])
            .unwrap_or("")
            .to_string(),
        commit_required: kind == Some(EventKind::Compaction),
        capabilities: manifest,
    }
}

fn looks_like_fs_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

/// Ticket stem inside a path leaf (`SESSIONSTART-1-project` → `SESSIONSTART-1`).
/// This is host cue expansion, not CQR hard-handle admission.
fn ticket_stem(segment: &str) -> Option<String> {
    let mut parts = segment.split('-');
    let prefix = parts.next()?;
    let number = parts.next()?;
    if prefix.len() >= 2
        && prefix.bytes().all(|b| b.is_ascii_alphabetic())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
    {
        Some(format!("{prefix}-{number}"))
    } else {
        None
    }
}

fn expand_cwd_cues(cwd: &str) -> String {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return String::new();
    }
    let mut out = vec![cwd.to_string()];
    let segments: Vec<&str> = cwd
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .collect();
    if let Some(leaf) = segments.last() {
        if *leaf != cwd {
            out.push((*leaf).to_string());
        }
    }
    for segment in &segments {
        if let Some(ticket) = ticket_stem(segment) {
            if !out.iter().any(|s| s == &ticket) {
                out.push(ticket);
            }
        }
    }
    out.join(" ")
}

/// Cwd-only SessionStart keeps the raw path on the frame; CQR gets the leaf,
/// any ticket stem, and `paths: [cwd]` so a folder name can admit a project fact.
fn session_orient_query(frame: &EventFrame) -> (String, Vec<String>) {
    let cwd = frame.scope.trim();
    let input = frame.input.trim();
    let mut paths = Vec::new();
    if !cwd.is_empty() && looks_like_fs_path(cwd) {
        paths.push(cwd.to_string());
    }
    let task = if !cwd.is_empty() && looks_like_fs_path(cwd) && (input.is_empty() || input == cwd)
    {
        expand_cwd_cues(cwd)
    } else {
        frame.input.clone()
    };
    (task, paths)
}

/// Execute one frame against an open brain.
pub async fn process(
    cx: &asupersync::Cx,
    runtime: &CortexRuntime,
    agent: &str,
    frame: &EventFrame,
    payload: &Value,
) -> Result<HookResult, String> {
    // The brain itself is open, so memory is available; the Reflex snapshot
    // only decides whether Level-0 may answer warm. A missing or expired
    // snapshot falls back to full retrieval and is counted as a fallback.
    let reflex = crate::reflex::load(&crate::reflex::snapshot_path(&runtime.state().home));
    let (reflex_state, capture_state) = {
        let conn = runtime
            .state()
            .db_read
            .lock(cx)
            .await
            .map_err(|e| e.to_string())?;
        (
            crate::reflex::state_for(reflex.as_ref(), &conn),
            crate::db::capture_policy::state_for(&conn, &frame.scope),
        )
    };
    let outcome = decide(frame, SnapshotState::Fresh);
    let mut result = HookResult {
        event: frame.kind,
        outcome: outcome.clone(),
        additional_context: String::new(),
        capture_receipt: None,
        capture_key: None,
        checkpoint: None,
        reflex: json!({"state": reflex_state, "level0": false, "fallback": false}),
        context_replace: Vec::new(),
    };
    if frame.kind == Some(EventKind::ToolResult) && !capture_state.allows_capture() {
        result.outcome.decision = HookDecision::Noop;
        result.outcome.automatic_capture = false;
        result.outcome.reason = format!(
            "capture {} for scope {:?}",
            capture_state.as_str(),
            frame.scope
        );
        return Ok(result);
    }
    if !capture_state.allows_delivery()
        && matches!(
            frame.kind,
            Some(EventKind::SessionStart | EventKind::NewTurn | EventKind::PromptDelta)
        )
    {
        result.outcome.decision = HookDecision::Noop;
        result.outcome.counted = false;
        result.outcome.reason = "capture stopped for scope: no automatic delivery".into();
        return Ok(result);
    }
    let caller = || Caller {
        owner_id: None,
        agent,
        principal: "solo".into(),
    };
    match (frame.kind, outcome.decision) {
        (Some(EventKind::ToolResult), HookDecision::Deliver) => {
            let tool = s(payload, &["tool_name"]).unwrap_or("tool");
            let command = payload
                .get("tool_input")
                .and_then(|i| s(i, &["command", "cmd"]));
            let exit = payload
                .get("tool_response")
                .and_then(|r| r.get("exit_code").or(r.get("exitCode")))
                .and_then(Value::as_i64)
                .map(|c| c as i32);
            let facts = parse_tool_result(tool, command, &frame.input, exit);
            if facts.is_material() {
                let key = facts.idempotency_key();
                let text = facts.statement();
                match runtime
                    .deposit_with_key(cx, &frame.invocation_id, Some(&key), &text, agent, None)
                    .await
                {
                    Ok(out) => {
                        result.capture_receipt = Some(out.receipt.receipt_id.canonical());
                        result.capture_key = Some(key);
                        if !out.receipt.is_locally_durable() {
                            result.outcome.automatic_capture = false;
                            result.outcome.reason =
                                "capture accepted but not past the write boundary: no durable ack"
                                    .into();
                        }
                    }
                    Err(err) => return Err(format!("capture failed: {err}")),
                }
            } else {
                result.outcome.automatic_capture = false;
                result.outcome.decision = HookDecision::Noop;
                result.outcome.reason = "tool result carried no deterministic facts".into();
            }
        }
        (Some(EventKind::Compaction | EventKind::Handoff), HookDecision::Deliver) => {
            let thread = frame
                .thread
                .clone()
                .unwrap_or_else(|| "session:unknown".into());
            let args = json!({"thread": thread, "action": "checkpoint", "goal": s(payload, &["goal"]).unwrap_or("context transition"), "state": {"context_epoch": frame.context_epoch, "trigger": frame.kind}, "note": "checkpoint on host transition"});
            match dispatch(cx, runtime.state(), caller(), Operation::Checkpoint, &args).await {
                Ok(v) => {
                    result.checkpoint = Some(v);
                    result.additional_context = format!(
                        "Cortex checkpoint recorded for {thread}. A successor resumes from cortex_checkpoint status, not from this transcript."
                    );
                }
                Err(err) => return Err(format!("checkpoint failed: {err}")),
            }
        }
        (
            Some(EventKind::SessionStart | EventKind::NewTurn | EventKind::PromptDelta),
            HookDecision::Deliver,
        ) => {
            let (task, paths) = session_orient_query(frame);
            if let (SnapshotState::Fresh, Some(snapshot)) = (reflex_state, reflex.as_ref()) {
                let warm = crate::reflex::level0(snapshot, &task, frame.thread.as_deref(), 8);
                result.reflex = json!({"state": reflex_state, "level0": !warm.fallback, "fallback": warm.fallback, "micros": warm.micros, "suppressed": warm.suppressed});
                if !warm.fallback {
                    let lines: Vec<String> = warm
                        .hits
                        .iter()
                        .map(|h| format!("{} [{}]", h.line, h.source))
                        .collect();
                    let text = lines.join("\n");
                    result.additional_context = format!(
                        "CORTEX ORIENTATION (warm reflex, {} bytes counted):\n{text}",
                        text.len()
                    );
                    return Ok(result);
                }
            } else {
                result.reflex = json!({"state": reflex_state, "level0": false, "fallback": true});
            }
            let mut args = json!({"task": task, "needs": frame.needs, "budget": DELIVERY_BUDGET.min(frame.capabilities.max_delivery_bytes), "thread": frame.thread});
            if !paths.is_empty() {
                args["paths"] = json!(paths);
            }
            match dispatch(cx, runtime.state(), caller(), Operation::Orient, &args).await {
                Ok(view) => {
                    let text = render_view(&view);
                    result.additional_context = if text.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "CORTEX ORIENTATION (delivered by hook, {} bytes counted):\n{text}",
                            text.len()
                        )
                    };
                    if result.additional_context.is_empty() {
                        result.outcome.decision = HookDecision::Noop;
                        result.outcome.reason = "orientation empty for this brain".into();
                        result.outcome.counted = false;
                    }
                }
                Err(err) => return Err(format!("orientation failed: {err}")),
            }
        }
        (_, HookDecision::QueryRequired) => {
            result.additional_context = "Cortex: memory is available but this host cannot inject it. Call cortex_orient before acting.".into();
        }
        (_, HookDecision::Unavailable) => {
            result.additional_context =
                "Cortex: memory unavailable for this event. Do not assume the brain is empty."
                    .into();
        }
        _ => {}
    }
    Ok(result)
}

fn render_view(view: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(cards) = view.get("cards").and_then(Value::as_array) {
        for card in cards.iter().take(12) {
            if let Some(statement) = card.get("statement").and_then(Value::as_str) {
                let alias = card.get("alias").and_then(Value::as_str).unwrap_or("");
                let status = card
                    .get("epistemic")
                    .and_then(Value::as_str)
                    .or_else(|| card.get("status").and_then(Value::as_str))
                    .unwrap_or("");
                lines.push(format!("- [{alias}] {statement} ({status})").replace("[] ", ""));
            }
        }
    }
    if let Some(items) = view
        .pointer("/observations/items")
        .and_then(Value::as_array)
    {
        for item in items.iter().take(6) {
            let preview = item.get("preview").and_then(Value::as_str).unwrap_or("");
            let expand = item.get("expand").and_then(Value::as_str).unwrap_or("");
            if !preview.is_empty() {
                lines.push(format!("- [obs] {preview} ({expand})"));
            }
        }
    }
    if let Some(brief) = view
        .pointer("/assemblies/brief")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        lines.push(brief.to_string());
    }
    if let Some(receipt) = view
        .get("receipt")
        .and_then(|r| r.get("receipt_id"))
        .and_then(Value::as_str)
    {
        lines.push(format!(
            "receipt: {receipt} (aliases valid with this receipt only)"
        ));
    }
    lines.join("\n")
}

/// Env `CORTEX_CAPTURE` wins. Otherwise the operator sidecar at
/// [`crate::auth::CortexPaths::capture_sidecar`]. Missing both is silent, not CQR.
pub fn load_capture_sidecar(paths: &crate::auth::CortexPaths) -> Option<String> {
    match std::env::var("CORTEX_CAPTURE") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => std::fs::read_to_string(paths.capture_sidecar())
            .ok()
            .filter(|value| !value.trim().is_empty()),
    }
}

/// Write the installed sidecar once. Existing operator files are left alone.
pub fn write_installed_capture_sidecar(
    paths: &crate::auth::CortexPaths,
    sidecar: &Value,
) -> Result<bool, String> {
    let path = paths.capture_sidecar();
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("Cannot create {}: {err}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(sidecar).map_err(|err| err.to_string())?;
    crate::auth::write_secret_file(&path, &bytes).map_err(|err| format!("Cannot write {}: {err}", path.display()))?;
    Ok(true)
}

fn unavailable_envelope(host_event: &str, reason: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": host_event,
            "additionalContext": "Cortex: memory unavailable for this event. Do not assume the brain is empty."
        },
        "cortex": {
            "event": null,
            "decision": "UNAVAILABLE",
            "reason": reason,
            "overflow": false,
            "presence": "unknown",
            "automatic_capture": false,
            "counted": false
        }
    })
}

pub async fn run(cx: &asupersync::Cx, kind: &str, _agent: &str) -> Result<(), String> {
    use std::io::Read;
    let limit = crate::runtime::observation::MAX_CAPTURE_BYTES;
    let mut raw = Vec::new();
    std::io::stdin()
        .lock()
        .take(limit as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|err| err.to_string())?;
    if let Some(value) = run_with_paths(cx, kind, &raw, &crate::auth::CortexPaths::resolve()).await? {
        println!("{value}");
    }
    Ok(())
}

/// Observation-only live hook. `None` means silent: no sidecar, or this
/// event is not opted in. CQR stays on [`process`].
pub async fn run_with_paths(
    cx: &asupersync::Cx,
    kind: &str,
    raw: &[u8],
    paths: &crate::auth::CortexPaths,
) -> Result<Option<Value>, String> {
    let limit = crate::runtime::observation::MAX_CAPTURE_BYTES;
    if raw.len() > limit {
        return Err("hook_input_byte_limit".into());
    }
    let payload: Value =
        serde_json::from_slice(raw).map_err(|err| format!("invalid_hook_json: {err}"))?;
    let host_event = s(&payload, &["hook_event_name"]).unwrap_or(kind);
    let Some(sidecar_raw) = load_capture_sidecar(paths) else {
        return Ok(None);
    };
    let sidecar: Value = match serde_json::from_str(&sidecar_raw) {
        Ok(value) => value,
        Err(err) => {
            return Ok(Some(unavailable_envelope(
                host_event,
                &format!("invalid capture invocation sidecar: {err}"),
            )));
        }
    };
    match crate::runtime::host_capture::resolve_host_invocation(kind, &sidecar, &payload) {
        Ok(Some(invocation)) => {
            let runtime =
                CortexRuntime::open(paths).map_err(|err| format!("brain unavailable: {err}"))?;
            let (_receipt, view) = runtime
                .capture_and_prepare_host(
                    cx,
                    &invocation.grant,
                    &invocation.context,
                    raw,
                    &invocation.invocation_context,
                    invocation.present.as_deref(),
                )
                .await?;
            if let Some(view) = view {
                let mut context = String::new();
                if view.status == "ready" && !view.payload.is_empty() {
                    context.push_str(&view.payload);
                }
                if !view.assembly_brief.is_empty() {
                    if !context.is_empty() {
                        context.push('\n');
                    }
                    context.push_str(&view.assembly_brief);
                }
                if !context.is_empty() {
                    return Ok(Some(json!({
                        "hookSpecificOutput": {
                            "hookEventName": host_event,
                            "additionalContext": context
                        }
                    })));
                }
            }
            Ok(None)
        }
        Ok(None) => Ok(None),
        Err(err) => Ok(Some(unavailable_envelope(
            host_event,
            &format!("invalid capture invocation sidecar: {err}"),
        ))),
    }
}
