use super::{DELIVERY_BUDGET, HookResult};
use crate::adapter::{EventFrame, EventKind, HookDecision, SnapshotState, decide};
use crate::capture::parse_tool_result;
use crate::handlers::looks_like_fs_path;
use crate::handlers::operations::{Caller, Operation, dispatch};
use crate::protocol::arg_str;
use crate::protocol::json_i64;
use crate::runtime::CortexRuntime;
use serde_json::{Value, json};

fn tool_exit_status(payload: &Value) -> Option<i32> {
    payload
        .get("tool_response")
        .and_then(|r| r.get("exit_code").or_else(|| r.get("exitCode")))
        .and_then(json_i64)
        .and_then(|c| i32::try_from(c).ok())
}

/// Ticket stem inside a path leaf (`SESSIONSTART-1-project` → `SESSIONSTART-1`).
/// This is host cue expansion, not CQR hard-handle admission.
fn ticket_stem(segment: &str) -> Option<String> {
    let mut parts = segment.split('-');
    let prefix = parts.next()?;
    let number = parts.next()?;
    (prefix.len() >= 2
        && prefix.bytes().all(|b| b.is_ascii_alphabetic())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit()))
    .then(|| format!("{prefix}-{number}"))
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
    let paths = write_paths(frame);
    let task = if !cwd.is_empty() && looks_like_fs_path(cwd) && (input.is_empty() || input == cwd) {
        expand_cwd_cues(cwd)
    } else {
        frame.input.clone()
    };
    (task, paths)
}

fn orientation_banner(kind: &str, text: &str) -> String {
    format!(
        "CORTEX ORIENTATION ({kind}, {} bytes counted):\n{text}",
        text.len()
    )
}

fn write_paths(frame: &EventFrame) -> Vec<String> {
    let cwd = frame.scope.trim();
    (!cwd.is_empty() && looks_like_fs_path(cwd))
        .then(|| vec![cwd.to_string()])
        .unwrap_or_default()
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
        owner_id: runtime.state().default_owner_id,
        agent,
        principal: "solo".into(),
    };
    match (frame.kind, outcome.decision) {
        (Some(EventKind::ToolResult), HookDecision::Deliver) => {
            let tool = arg_str(payload, &["tool_name"]).unwrap_or("tool");
            let command = payload.get("tool_input").and_then(|i| arg_str(i, &["command", "cmd"]));
            let exit = tool_exit_status(payload);
            let facts = parse_tool_result(tool, command, &frame.input, exit);
            if facts.is_material() {
                let key = facts.idempotency_key();
                let text = facts.statement();
                let out = runtime.deposit_with_scope(cx, &frame.invocation_id, Some(&key), &text, agent, None, &write_paths(frame), frame.thread.as_deref()).await.map_err(|err| format!("capture failed: {err}"))?;
                result.capture_receipt = Some(out.receipt.receipt_id.canonical());
                result.capture_key = Some(key);
                if !out.receipt.is_locally_durable() {
                    result.outcome.automatic_capture = false;
                    result.outcome.reason = "capture accepted but not past the write boundary: no durable ack".into();
                }
            } else {
                result.outcome.automatic_capture = false;
                result.outcome.decision = HookDecision::Noop;
                result.outcome.reason = "tool result carried no deterministic facts".into();
            }
        }
        (Some(EventKind::Compaction | EventKind::Handoff), HookDecision::Deliver) => {
            let thread = frame.thread.clone().unwrap_or_else(|| "session:unknown".into());
            let args = json!({"thread": thread, "action": "checkpoint", "goal": arg_str(payload, &["goal"]).unwrap_or("context transition"), "state": {"context_epoch": frame.context_epoch, "trigger": frame.kind}, "note": "checkpoint on host transition"});
            let v = dispatch(cx, runtime.state(), caller(), Operation::Checkpoint, &args).await.map_err(|err| format!("checkpoint failed: {err}"))?;
            result.checkpoint = Some(v);
            result.additional_context = format!("Cortex checkpoint recorded for {thread}. A successor resumes from cortex_checkpoint status, not from this transcript.");
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
                    let lines: Vec<String> = warm.hits.iter().map(|h| format!("{} [{}]", h.line, h.source)).collect();
                    let text = lines.join("\n");
                    result.additional_context = orientation_banner("warm reflex", &text);
                    return Ok(result);
                }
            } else {
                result.reflex = json!({"state": reflex_state, "level0": false, "fallback": true});
            }
            let mut args = json!({"task": task, "needs": frame.needs, "budget": DELIVERY_BUDGET.min(frame.capabilities.max_delivery_bytes), "thread": frame.thread});
              if !paths.is_empty() { args["paths"] = json!(paths); }
            let view = dispatch(cx, runtime.state(), caller(), Operation::Orient, &args).await.map_err(|err| format!("orientation failed: {err}"))?;
            let text = render_view(&view);
            result.additional_context = (!text.is_empty()).then(|| orientation_banner("delivered by hook", &text)).unwrap_or_default();
            if result.additional_context.is_empty() {
                result.outcome.decision = HookDecision::Noop;
                result.outcome.reason = "orientation empty for this brain".into();
                result.outcome.counted = false;
            }
        }
        (_, HookDecision::QueryRequired) => result.additional_context = "Cortex: memory is available but this host cannot inject it. Call cortex_orient before acting.".into(),
        (_, HookDecision::Unavailable) => result.additional_context = "Cortex: memory unavailable for this event. Do not assume the brain is empty.".into(),
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
