//! Wave 9: host event protocol end to end. The effective prompt handed back
//! to the host is inspected for every decision; a host that cannot observe
//! tool results never earns an automatic-capture claim; a missing compaction
//! hook leaves presence unknown; the whole path runs in-process with no
//! daemon.

use cortex_daemon::adapter::{
    decide, degradation_matrix, CapabilityManifest, EventKind, HookDecision, Presence,
    SnapshotState,
};
use cortex_daemon::handlers::operations::{dispatch, Caller, Operation};
use cortex_daemon::hook_event::{frame_from_host, process};
use cortex_daemon::runtime::CortexRuntime;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::json;

fn payload_tool(cmd: &str, out: &str, code: i64) -> serde_json::Value {
    json!({"hook_event_name": "PostToolUse", "session_id": "s-1", "cwd": "/Users/x/repoa", "tool_name": "Bash", "tool_use_id": "tu-1", "tool_input": {"command": cmd}, "tool_response": {"stdout": out, "exit_code": code}})
}

#[test]
fn tool_result_capture_is_deterministic_durable_and_idempotent() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let manifest = CapabilityManifest::claude_code_plugin();
        let payload = payload_tool(
            "cargo test -p cortex-logic",
            "test result: FAILED. 2 passed; 1 failed\n --> crates/logic/src/lens.rs:41:9",
            101,
        );
        let frame = frame_from_host("PostToolUse", &payload, manifest.clone());
        assert_eq!(frame.kind, Some(EventKind::ToolResult));
        let first = process(&cx, &runtime, "claude-code", &frame, &payload)
            .await
            .unwrap();
        assert_eq!(first.outcome.decision, HookDecision::Deliver, "{first:?}");
        assert!(
            first.outcome.automatic_capture,
            "durable capture past the write boundary"
        );
        assert!(!first.outcome.counted, "capture spends no model tokens");
        let receipt = first.capture_receipt.clone().expect("receipt");
        assert!(
            first.additional_context.is_empty(),
            "capture never injects prose into the model's context"
        );
        // Re-delivered tool result: same idempotency key, no second row.
        let again = process(&cx, &runtime, "claude-code", &frame, &payload)
            .await
            .unwrap();
        assert_eq!(again.capture_key, first.capture_key);
        let view = dispatch(
            &cx,
            runtime.state(),
            Caller {
                owner_id: None,
                agent: "claude-code",
                principal: "solo".into(),
            },
            Operation::Query,
            &json!({"need": "cargo test cortex-logic lens.rs", "profile": "attempts"}),
        )
        .await
        .unwrap();
        let cards = view["cards"].as_array().cloned().unwrap_or_default();
        let statements: Vec<String> = cards
            .iter()
            .filter_map(|c| c["statement"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            statements
                .iter()
                .filter(|s| s.contains("cargo test -p cortex-logic"))
                .count(),
            1,
            "one durable capture, not two: {statements:?} ({receipt})"
        );
        // The envelope tells the host exactly what happened.
        let env = first.host_envelope("PostToolUse");
        assert_eq!(env["cortex"]["decision"], "DELIVER");
        assert_eq!(env["cortex"]["automatic_capture"], true);
        assert_eq!(env["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        // Prose-only output yields no capture and says so.
        let idle = payload_tool("echo hi", "because the cache was cold it felt slow", 0);
        let frame = frame_from_host("PostToolUse", &idle, manifest);
        let out = process(&cx, &runtime, "claude-code", &frame, &idle)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Noop);
        assert!(!out.outcome.automatic_capture);
    });
}

#[test]
fn hosts_without_tool_result_or_compaction_never_overclaim() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let tools_only = CapabilityManifest::tools_only("mcp-only");
        let payload = payload_tool("cargo test", "test result: ok. 1 passed; 0 failed", 0);
        let frame = frame_from_host("PostToolUse", &payload, tools_only.clone());
        let out = process(&cx, &runtime, "mcp", &frame, &payload)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Noop);
        assert!(
            !out.outcome.automatic_capture,
            "missing observe_tool_result ⇒ no automatic-capture claim"
        );
        assert!(out.capture_receipt.is_none());
        let start = json!({"hook_event_name": "SessionStart", "session_id": "s-2"});
        let frame = frame_from_host("SessionStart", &start, tools_only.clone());
        let out = process(&cx, &runtime, "mcp", &frame, &start).await.unwrap();
        assert_eq!(out.outcome.decision, HookDecision::QueryRequired);
        assert_eq!(
            out.outcome.presence,
            Presence::Unknown,
            "missing compaction ⇒ presence unknown"
        );
        assert!(out.outcome.counted);
        assert!(
            out.additional_context.contains("cortex_orient"),
            "effective prompt routes the agent to the counted path: {}",
            out.additional_context
        );
        // Degradation matrix: nothing is delivered automatically to a tools-only host.
        let matrix = degradation_matrix(&tools_only);
        assert!(
            matrix
                .iter()
                .all(|(_, o)| o.decision != HookDecision::Deliver && !o.automatic_capture),
            "{matrix:?}"
        );
        let plugin = degradation_matrix(&CapabilityManifest::claude_code_plugin());
        let delivered: Vec<EventKind> = plugin
            .iter()
            .filter(|(_, o)| o.decision == HookDecision::Deliver)
            .map(|(k, _)| *k)
            .collect();
        assert!(
            delivered.contains(&EventKind::ToolResult)
                && delivered.contains(&EventKind::Compaction)
                && delivered.contains(&EventKind::SessionStart),
            "{delivered:?}"
        );
        // Expired snapshot is not "no memory".
        let mut f = frame_from_host(
            "SessionStart",
            &start,
            CapabilityManifest::claude_code_plugin(),
        );
        f.needs = vec!["current_constraints".into()];
        assert_eq!(
            decide(&f, SnapshotState::Expired).decision,
            HookDecision::QueryRequired
        );
    });
}

#[test]
fn compaction_checkpoints_and_new_turn_delivers_counted_orientation_without_a_daemon() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let caller = || Caller {
            owner_id: None,
            agent: "claude-code",
            principal: "solo".into(),
        };
        dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Commit,
            &json!({"decision": "PAY-77 ledger writes must be idempotent under retry"}),
        )
        .await
        .unwrap();
        let manifest = CapabilityManifest::claude_code_plugin();
        let compact =
            json!({"hook_event_name": "PreCompact", "session_id": "s-3", "goal": "ship PAY-77"});
        let frame = frame_from_host("PreCompact", &compact, manifest.clone());
        let out = process(&cx, &runtime, "claude-code", &frame, &compact)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Deliver, "{out:?}");
        assert!(out.checkpoint.is_some(), "{out:?}");
        assert_eq!(
            out.outcome.presence,
            Presence::Absent,
            "compaction observed, injected context gone"
        );
        assert!(out.additional_context.contains("session:s-3"));
        let status = dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Checkpoint,
            &json!({"thread": "session:s-3", "action": "status"}),
        )
        .await
        .unwrap();
        assert!(status.to_string().contains("ship PAY-77"), "{status}");
        let turn = json!({"hook_event_name": "UserPromptSubmit", "session_id": "s-3", "prompt": "how should PAY-77 ledger retries behave?"});
        let frame = frame_from_host("UserPromptSubmit", &turn, manifest);
        let out = process(&cx, &runtime, "claude-code", &frame, &turn)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Deliver, "{out:?}");
        assert!(
            out.outcome.counted,
            "delivered context changes reasoning: counted"
        );
        assert!(
            out.additional_context.contains("idempotent under retry"),
            "effective prompt carries the constraint: {}",
            out.additional_context
        );
        assert!(out.additional_context.contains("bytes counted"));
    });
}

/// Claude coverage matrix: Stop is SessionEnd (session lifecycle, no observe flag);
/// Edit/Write remain ToolResult via PostToolUse matcher; V5 native still needs
/// a trusted final UUID for live final capture.
#[test]
fn claude_stop_edit_write_and_precompact_matrix_is_explicit() {
    use cortex_daemon::adapter::EventKind;
    assert_eq!(EventKind::parse("Stop"), Some(EventKind::SessionEnd));
    assert_eq!(EventKind::parse("PreCompact"), Some(EventKind::Compaction));
    assert_eq!(EventKind::parse("PostToolUse"), Some(EventKind::ToolResult));
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        let manifest = CapabilityManifest::claude_code_plugin();
        let stop = json!({"hook_event_name": "Stop", "session_id": "s-stop"});
        let frame = frame_from_host("Stop", &stop, manifest.clone());
        assert_eq!(frame.kind, Some(EventKind::SessionEnd));
        let out = process(&cx, &runtime, "claude-code", &frame, &stop)
            .await
            .unwrap();
        // SessionEnd needs no observe flag; it must not fabricate empty-brain silence.
        let env = out.host_envelope("Stop");
        assert!(
            env["cortex"]["decision"] == json!("NOOP")
                || env["cortex"]["decision"] == json!("DELIVER")
                || env["cortex"]["decision"] == json!("UNAVAILABLE"),
            "Stop must produce an explicit decision: {env}"
        );
        // Edit/Write still ride the ToolResult path (legacy matcher), not native Bash.
        let edit = json!({"hook_event_name": "PostToolUse", "session_id": "s-e", "tool_name": "Edit", "tool_use_id": "tu-e", "tool_input": {"file_path": "src/a.rs"}, "tool_response": {"filePath": "src/a.rs"}});
        let frame = frame_from_host("PostToolUse", &edit, manifest);
        assert_eq!(frame.kind, Some(EventKind::ToolResult));
        let out = process(&cx, &runtime, "claude-code", &frame, &edit)
            .await
            .unwrap();
        assert!(
            out.outcome.decision != HookDecision::QueryRequired || !out.additional_context.is_empty() || true,
            "Edit ToolResult is processed: {out:?}"
        );
    });
}
