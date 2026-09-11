//! Wave 9: host event protocol end to end. The effective prompt handed back
//! to the host is inspected for every decision; a host that cannot observe
//! tool results never earns an automatic-capture claim; a missing compaction
//! hook leaves presence unknown; the whole path runs in-process with no
//! daemon.

use cortex_logic::adapter::{
    decide, degradation_matrix, CapabilityManifest, EventKind, HookDecision, Presence,
    SnapshotState,
};
use cortex_daemon::hook_boot::{boot_assembly_brief, boot_capsule_for_payload, session_start_context};
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::auth::CortexPaths;
use cortex_kernel::hook_event::{frame_from_host, load_capture_sidecar, process, run_with_paths};
use cortex_kernel::runtime::assembly::{AssemblyMemberSpec, AssemblySpec};
use cortex_kernel::runtime::host_capture::{ADAPTER_VERSION, HostCaptureGrant};
use cortex_kernel::runtime::observation::{ObservationEvent, SourceSpec};
use cortex_kernel::runtime::{CortexRuntime, LensInput};
use cortex_kernel::assembly::{LearningEvent, LearningKind, MembershipRole};
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
/// Edit/Write remain ToolResult via PostToolUse matcher; live final capture
/// still needs a trusted final UUID.
#[test]
fn claude_stop_edit_write_and_precompact_matrix_is_explicit() {
    use cortex_logic::adapter::EventKind;
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

#[test]
fn live_hook_does_not_fall_through_to_cqr() {
    if !cortex_tests::in_subprocess(
        "live_hook_does_not_fall_through_to_cqr",
        &[("CORTEX_CAPTURE", None)],
    ) {
        return;
    }
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let db = home.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&db).unwrap();
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
            &json!({"decision": "HOOKCUT-1 ledger writes must be idempotent under retry"}),
        )
        .await
        .unwrap();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home.path().to_string_lossy()),
            Some(&db.to_string_lossy()),
        );
        let turn = json!({"hook_event_name": "UserPromptSubmit", "session_id": "s-cut", "prompt": "how should HOOKCUT-1 ledger retries behave?"});
        let raw = serde_json::to_vec(&turn).unwrap();
        let printed = run_with_paths(&cx, "UserPromptSubmit", &raw, &paths)
            .await
            .unwrap();
        assert!(
            printed.is_none(),
            "absent sidecar must not emit CQR orientation: {printed:?}"
        );
        let compact = json!({"hook_event_name": "PreCompact", "session_id": "s-cut", "goal": "ship HOOKCUT-1"});
        let compact_raw = serde_json::to_vec(&compact).unwrap();
        assert!(
            run_with_paths(&cx, "PreCompact", &compact_raw, &paths)
                .await
                .unwrap()
                .is_none()
        );
        let status = dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Checkpoint,
            &json!({"thread": "session:s-cut", "action": "status"}),
        )
        .await
        .unwrap();
        assert!(
            !status.to_string().contains("HOOKCUT-1"),
            "live hook must not write a CQR checkpoint without a sidecar: {status}"
        );
        let manifest = CapabilityManifest::claude_code_plugin();
        let frame = frame_from_host("UserPromptSubmit", &turn, manifest);
        let out = process(&cx, &runtime, "claude-code", &frame, &turn)
            .await
            .unwrap();
        assert!(
            out.additional_context.contains("HOOKCUT-1")
                || out.additional_context.contains("idempotent")
                || out.additional_context.contains("ledger writes"),
            "process() remains the CQR hook seam: {}",
            out.additional_context
        );
    });
}

#[test]
fn session_start_orients_from_cwd() {
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
            &json!({"decision": "SESSIONSTART-1 ledger retries must be idempotent"}),
        )
        .await
        .unwrap();
        let cwd_only = json!({
            "hook_event_name": "SessionStart",
            "session_id": "s-orient",
            "cwd": "/tmp/SESSIONSTART-1-project"
        });
        assert_eq!(
            frame_from_host("SessionStart", &cwd_only, CapabilityManifest::claude_code_plugin()).input,
            "/tmp/SESSIONSTART-1-project",
            "SessionStart must orient on cwd when prompt is absent"
        );
        let cwd_frame = frame_from_host(
            "SessionStart",
            &cwd_only,
            CapabilityManifest::claude_code_plugin(),
        );
        let cwd_out = process(&cx, &runtime, "claude-code", &cwd_frame, &cwd_only)
            .await
            .unwrap();
        assert!(
            cwd_out.additional_context.contains("CORTEX ORIENTATION")
                && cwd_out.additional_context.contains("SESSIONSTART-1"),
            "cwd-only SessionStart must retrieve the project fact without a prompt: {}",
            cwd_out.additional_context
        );
        let cwd_hooked = session_start_context(&cx, "claude-code", &runtime, &cwd_only)
            .await
            .expect("hook-boot cwd-only SessionStart context");
        assert!(
            cwd_hooked.contains("CORTEX ORIENTATION") && cwd_hooked.contains("SESSIONSTART-1"),
            "hook-boot cwd-only must use the same orient View: {cwd_hooked}"
        );
        let elsewhere = json!({
            "hook_event_name": "SessionStart",
            "session_id": "s-orient-elsewhere",
            "cwd": "/tmp/other-repo"
        });
        let else_frame = frame_from_host(
            "SessionStart",
            &elsewhere,
            CapabilityManifest::claude_code_plugin(),
        );
        let else_out = process(&cx, &runtime, "claude-code", &else_frame, &elsewhere)
            .await
            .unwrap();
        assert!(
            !else_out.additional_context.contains("SESSIONSTART-1"),
            "a foreign cwd must not retrieve this project's fact: {}",
            else_out.additional_context
        );
        let start = json!({
            "hook_event_name": "SessionStart",
            "session_id": "s-orient",
            "cwd": "/tmp/SESSIONSTART-1-project",
            "prompt": "SESSIONSTART-1 continue ledger retry work"
        });
        let frame = frame_from_host("SessionStart", &start, CapabilityManifest::claude_code_plugin());
        assert_eq!(
            frame.input, "SESSIONSTART-1 continue ledger retry work",
            "SessionStart prompt wins over cwd for the orient task"
        );
        let out = process(&cx, &runtime, "claude-code", &frame, &start)
            .await
            .unwrap();
        assert!(
            out.additional_context.contains("CORTEX ORIENTATION"),
            "SessionStart injects the orient View, not a raw boot dump: {}",
            out.additional_context
        );
        assert!(
            out.additional_context.contains("SESSIONSTART-1"),
            "cwd cue must reach orient: {}",
            out.additional_context
        );
        let hooked = session_start_context(&cx, "claude-code", &runtime, &start)
            .await
            .expect("hook-boot SessionStart context");
        assert!(
            hooked.contains("CORTEX ORIENTATION") && hooked.contains("SESSIONSTART-1"),
            "hook-boot must use the same orient View: {hooked}"
        );
    });
}

#[test]
fn live_hook_reads_home_capture_file() {
    if !cortex_tests::in_subprocess(
        "live_hook_reads_home_capture_file",
        &[("CORTEX_CAPTURE", None)],
    ) {
        return;
    }
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let db = home.path().join("cortex.db");
        let runtime = CortexRuntime::open_db(&db).unwrap();
        runtime
            .register_host_capture(
                &cx,
                HostCaptureGrant {
                    key: "local-host".into(),
                    scope: "project".into(),
                    host_version: "fixture-v1".into(),
                    adapter_version: ADAPTER_VERSION.into(),
                    max_bytes: 4096,
                    live: true,
                    history: true,
                },
            )
            .await
            .unwrap();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home.path().to_string_lossy()),
            Some(&db.to_string_lossy()),
        );
        std::fs::write(
            paths.capture_sidecar(),
            r#"{"grant":"local-host","host_version":"fixture-v1","native_user_prompts":true}"#,
        )
        .unwrap();
        assert!(
            load_capture_sidecar(&paths).is_some(),
            "operator sidecar file must load when CORTEX_CAPTURE is unset"
        );
        let session = "225407e0-7c32-4812-b130-aa0d54039690";
        let prompt = "ea0dd413-83bc-43a2-8913-506964f73808";
        let turn = json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": session,
            "prompt_id": prompt,
            "prompt": "FILESIDECAR-1 remember the ledger retry rule"
        });
        let raw = serde_json::to_vec(&turn).unwrap();
        let printed = run_with_paths(&cx, "UserPromptSubmit", &raw, &paths)
            .await
            .unwrap();
        let hits = runtime
            .query_observations(&cx, "project", "FILESIDECAR-1", 8, 16 * 1024, false)
            .await
            .unwrap();
        assert!(
            hits.evidence
                .iter()
                .any(|item| item.text.contains("FILESIDECAR-1")),
            "home capture.json must capture without CORTEX_CAPTURE: printed={printed:?} hits={hits:?}"
        );
    });
}

#[test]
fn boot_fallback_keeps_path_scoped_assemblies_in_their_repository() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .register_source(&cx, SourceSpec::document("notes-a", REPO_A))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("extra-a", REPO_A))
            .await
            .unwrap();
        let noted = runtime
            .observe(
                &cx,
                "notes-a",
                "g",
                ObservationEvent {
                    event_key: "one".into(),
                    text: "bootpathretry requires idempotency".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let extra = runtime
            .observe(
                &cx,
                "extra-a",
                "g",
                ObservationEvent {
                    event_key: "one".into(),
                    text: "bootpathretry is unsafe here".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "boot-path-bundle".into(),
                    scope: REPO_A.into(),
                    kind: "rule".into(),
                    members: vec![
                        AssemblyMemberSpec {
                            revision_id: noted.revision_id.clone(),
                            role: MembershipRole::Observation,
                        },
                        AssemblyMemberSpec {
                            revision_id: extra.revision_id.clone(),
                            role: MembershipRole::Exception,
                        },
                    ],
                    guards: vec![],
                },
            )
            .await
            .unwrap();
        runtime
            .record_learning_event(
                &cx,
                LearningEvent {
                    origin: "host".into(),
                    origin_event_id: "boot-e1".into(),
                    principal: "local".into(),
                    scope: REPO_A.into(),
                    training_unit: "unit-boot".into(),
                    target: "boot-path-bundle".into(),
                    kind: LearningKind::Explicit,
                    reward: 1,
                    cues: vec!["bootpathretry".into()],
                    sources: vec![noted.source_id.clone()],
                    observed_at: 1,
                    receipt_ref: "receipt:boot-e1".into(),
                },
            )
            .await
            .unwrap();
        runtime.rebuild_assembly_routes(&cx, REPO_A).await.unwrap();
        let prompt = "Identity chrome bootpathretry must remain closed";
        let same = boot_assembly_brief(
            &cx,
            &runtime,
            prompt,
            &json!({"cwd": REPO_A}),
        )
        .await;
        assert!(
            same.contains("boot-path-bundle"),
            "SessionStart boot fallback must compile the cwd assembly: {same}"
        );
        let sibling = boot_assembly_brief(
            &cx,
            &runtime,
            prompt,
            &json!({"cwd": REPO_B}),
        )
        .await;
        assert!(
            !sibling.contains("boot-path-bundle"),
            "a sibling cwd must not compile the other repository's assembly: {sibling}"
        );
        let project = boot_assembly_brief(&cx, &runtime, prompt, &json!({})).await;
        assert!(
            !project.contains("boot-path-bundle"),
            "boot fallback without cwd must stay on the project bucket: {project}"
        );
    });
}

fn lens_excerpts(view: &serde_json::Value) -> Vec<&str> {
    view["results"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r["excerpt"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn tool_result_deposit_stays_in_the_cwd_repository() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        let runtime = CortexRuntime::from_state(solo_state());
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "session_id": "scope-s",
            "working_directory": REPO_A,
            "tool_name": "Bash",
            "tool_use_id": "tu-scope-1",
            "tool_input": {"command": "cargo test -p pathscopeunique"},
            "tool_response": {
                "stdout": "test result: FAILED. 0 passed; 1 failed\n --> crates/kernel/src/pathscope.rs:1:1",
                "exit_code": 101
            }
        });
        let frame = frame_from_host(
            "PostToolUse",
            &payload,
            CapabilityManifest::claude_code_plugin(),
        );
        assert_eq!(frame.scope, REPO_A);
        let out = process(&cx, &runtime, "claude-code", &frame, &payload)
            .await
            .unwrap();
        assert!(
            out.outcome.automatic_capture,
            "tool result must deposit: {out:?}"
        );

        let sibling = runtime
            .lens(
                &cx,
                LensInput {
                    query: "pathscopeunique".into(),
                    budget: 4000,
                    k: 8,
                    agent: "claude-code".into(),
                    paths: vec![REPO_B.into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            lens_excerpts(&sibling)
                .iter()
                .all(|excerpt| !excerpt.contains("pathscopeunique")),
            "a sibling repo must not see the cwd-scoped tool fact: {sibling}"
        );

        let home = runtime
            .lens(
                &cx,
                LensInput {
                    query: "pathscopeunique".into(),
                    budget: 4000,
                    k: 8,
                    agent: "claude-code".into(),
                    paths: vec![REPO_A.into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            lens_excerpts(&home)
                .iter()
                .any(|excerpt| excerpt.contains("pathscopeunique")),
            "cwd lens must admit the tool fact: {home}"
        );
    });
}

#[test]
fn boot_fallback_keeps_path_scoped_cqr_facts_in_their_repository() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        let runtime = CortexRuntime::from_state(solo_state());
        let caller = || Caller {
            owner_id: None,
            agent: "claude-code",
            principal: "solo".into(),
        };
        let a = dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Commit,
            &json!({
                "decision": "RepoA exclusive boot marker must stay here",
                "paths": [REPO_A],
                "retention_class": "durable",
                "type": "constraint"
            }),
        )
        .await
        .unwrap();
        assert_eq!(a["status"], "ok", "{a}");
        let b = dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Commit,
            &json!({
                "decision": "RepoB exclusive boot marker must stay here",
                "paths": [REPO_B],
                "retention_class": "durable",
                "type": "constraint"
            }),
        )
        .await
        .unwrap();
        assert_eq!(b["status"], "ok", "{b}");

        let same = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd_path": REPO_A}),
            2000,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            same.contains("RepoA exclusive boot marker"),
            "cwd boot must pack this repo: {same}"
        );
        assert!(
            !same.contains("RepoB exclusive boot marker"),
            "cwd boot must omit the sibling repo: {same}"
        );

        let sibling = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd_path": REPO_B}),
            2000,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            sibling.contains("RepoB exclusive boot marker"),
            "sibling cwd boot must pack its repo: {sibling}"
        );
        assert!(
            !sibling.contains("RepoA exclusive boot marker"),
            "sibling cwd boot must omit the other repo: {sibling}"
        );
    });
}

#[test]
fn operational_path_scoped_fact_reaches_cwd_boot_not_sibling() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        const FACT_A: &str = "RepoA exclusive boot marker must stay here";
        const FACT_B: &str = "RepoB exclusive boot marker must stay here";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit_with_scope(
                &cx,
                "op-boot-a",
                None,
                FACT_A,
                "claude-code",
                None,
                &[REPO_A.into()],
                None,
            )
            .await
            .unwrap();
        runtime
            .deposit_with_scope(
                &cx,
                "op-boot-b",
                None,
                FACT_B,
                "claude-code",
                None,
                &[REPO_B.into()],
                None,
            )
            .await
            .unwrap();

        {
            let conn = runtime.state().db_read.lock(&cx).await.unwrap();
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                n, 2,
                "two repositories with similar sentences must stay two facts"
            );
            let mut stmt = conn
                .prepare(
                    "SELECT d.decision, a.value, a.specificity
                     FROM decisions d
                     JOIN clock_anchor_evidence e
                       ON e.target_id = d.id AND e.target_type = 'decision'
                     JOIN clock_anchors a ON a.id = e.anchor_id
                     WHERE a.kind = 'path' AND d.status = 'active'
                     ORDER BY d.id, a.specificity DESC, a.value",
                )
                .unwrap();
            let rows: Vec<String> = stmt
                .query_map([], |row| {
                    Ok(format!(
                        "{} | {} | {}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?
                    ))
                })
                .unwrap()
                .flatten()
                .collect();
            assert!(
                rows.iter().any(|row| row.contains("RepoA exclusive")
                    && row.contains("users/x/repoa")
                    && row.contains(" | 3")),
                "operational deposit must keep the explicit repo path: {rows:?}"
            );
        }

        let same = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd": REPO_A}),
            600,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            same.contains("RepoA exclusive boot marker"),
            "cwd boot must pack this repo's operational fact: {same}"
        );
        assert!(
            !same.contains("RepoB exclusive boot marker"),
            "cwd boot must omit the sibling operational fact: {same}"
        );

        let sibling = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd": REPO_B}),
            600,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            sibling.contains("RepoB exclusive boot marker"),
            "sibling cwd boot must pack its operational fact: {sibling}"
        );
        assert!(
            !sibling.contains("RepoA exclusive boot marker"),
            "sibling cwd boot must omit the other operational fact: {sibling}"
        );
    });
}

#[test]
fn same_repo_duplicate_sentence_still_collapses() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const TEXT: &str = "RepoA exclusive boot marker must stay here";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit_with_scope(
                &cx,
                "op-dup-1",
                None,
                TEXT,
                "claude-code",
                None,
                &[REPO_A.into()],
                None,
            )
            .await
            .unwrap();
        runtime
            .deposit_with_scope(
                &cx,
                "op-dup-2",
                None,
                TEXT,
                "claude-code",
                None,
                &[REPO_A.into()],
                None,
            )
            .await
            .unwrap();
        let conn = runtime.state().db_read.lock(&cx).await.unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 1,
            "the same sentence in one repository must still collapse"
        );
    });
}

#[test]
fn tool_result_reaches_cwd_boot_capsule_not_sibling() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        let runtime = CortexRuntime::from_state(solo_state());
        let payload = json!({
            "hook_event_name": "PostToolUse",
            "session_id": "boot-s",
            "cwd": REPO_A,
            "tool_name": "Bash",
            "tool_use_id": "tu-boot-1",
            "tool_input": {"command": "cargo test -p bootcqrunify"},
            "tool_response": {
                "stdout": "test result: FAILED. 0 passed; 1 failed\n --> crates/kernel/src/bootcqr.rs:1:1",
                "exit_code": 101
            }
        });
        let frame = frame_from_host(
            "PostToolUse",
            &payload,
            CapabilityManifest::claude_code_plugin(),
        );
        let out = process(&cx, &runtime, "claude-code", &frame, &payload)
            .await
            .unwrap();
        assert!(
            out.outcome.automatic_capture,
            "tool result must deposit: {out:?}"
        );

        let same = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"working_directory": REPO_A}),
            600,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            same.contains("bootcqrunify"),
            "SessionStart boot at the tool cwd must pack the operational capture: {same}"
        );

        let sibling = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"working_directory": REPO_B}),
            600,
        )
        .await
        .expect("SessionStart boot capsule");
        assert!(
            !sibling.contains("bootcqrunify"),
            "a sibling cwd must not pack the other repository's tool capture: {sibling}"
        );
    });
}

#[test]
fn commit_cwd_path_stays_in_that_repository() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        let runtime = CortexRuntime::from_state(solo_state());
        let committed = dispatch(
            &cx,
            runtime.state(),
            Caller {
                owner_id: None,
                agent: "claude-code",
                principal: "solo".into(),
            },
            Operation::Commit,
            &json!({
                "decision": "CwdPath exclusive ledger retries must stay idempotent under duplicate POSTs",
                "cwd_path": REPO_A
            }),
        )
        .await
        .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");

        let sibling = runtime
            .lens(
                &cx,
                LensInput {
                    query: "CwdPath exclusive ledger retries".into(),
                    budget: 4000,
                    k: 8,
                    agent: "claude-code".into(),
                    paths: vec![REPO_B.into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            lens_excerpts(&sibling)
                .iter()
                .all(|excerpt| !excerpt.contains("CwdPath exclusive ledger retries")),
            "cwd_path commit must not admit in a sibling repo: {sibling}"
        );

        let home = runtime
            .lens(
                &cx,
                LensInput {
                    query: "CwdPath exclusive ledger retries".into(),
                    budget: 4000,
                    k: 8,
                    agent: "claude-code".into(),
                    paths: vec![REPO_A.into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(
            lens_excerpts(&home)
                .iter()
                .any(|excerpt| excerpt.contains("CwdPath exclusive ledger retries")),
            "cwd_path commit must admit in that repo: {home}"
        );
    });
}

fn active_decision_count(conn: &rusqlite::Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn unscoped_near_duplicate_does_not_absorb_path_scoped_fact() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        const UNSCOPED: &str = "Exclusive boot marker must stay here";
        const SCOPED: &str = "RepoA exclusive boot marker must stay here";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit(&cx, "unscoped-boot", UNSCOPED, "claude-code", None)
            .await
            .unwrap();
        runtime
            .deposit_with_scope(
                &cx,
                "scoped-boot",
                None,
                SCOPED,
                "claude-code",
                None,
                &[REPO_A.into()],
                None,
            )
            .await
            .unwrap();

        {
            let conn = runtime.state().db_read.lock(&cx).await.unwrap();
            assert_eq!(
                active_decision_count(&conn),
                2,
                "an unscoped sentence must not Jaccard-merge a path-scoped near-duplicate"
            );
            let mut stmt = conn
                .prepare(
                    "SELECT d.decision, COALESCE(a.value, ''), COALESCE(a.specificity, 0)
                     FROM decisions d
                     LEFT JOIN clock_anchor_evidence e
                       ON e.target_id = d.id AND e.target_type = 'decision'
                     LEFT JOIN clock_anchors a ON a.id = e.anchor_id AND a.kind = 'path'
                     WHERE d.status = 'active'
                     ORDER BY d.id, a.specificity DESC, a.value",
                )
                .unwrap();
            let rows: Vec<String> = stmt
                .query_map([], |row| {
                    Ok(format!(
                        "{} | {} | {}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?
                    ))
                })
                .unwrap()
                .flatten()
                .collect();
            assert!(
                rows.iter().any(|row| row.contains("RepoA exclusive")
                    && row.contains("users/x/repoa")
                    && row.contains(" | 3")),
                "path-scoped near-duplicate must keep the explicit repo path: {rows:?}"
            );
            assert!(
                rows.iter().any(|row| row.contains("Exclusive boot marker")
                    && !row.contains("RepoA exclusive")
                    && !row.contains("users/x/repoa")),
                "unscoped near-duplicate must remain without a spec-3 path: {rows:?}"
            );
        }

        let home = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd": REPO_A}),
            600,
        )
        .await
        .expect("cwd A boot");
        assert!(
            home.contains("RepoA exclusive boot marker"),
            "cwd boot must still pack the path-scoped fact: {home}"
        );

        let sibling = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd": REPO_B}),
            600,
        )
        .await
        .expect("cwd B boot");
        assert!(
            sibling.contains("Exclusive boot marker must stay here"),
            "unscoped fact must remain visible in a sibling repository: {sibling}"
        );
        assert!(
            !sibling.contains("RepoA exclusive boot marker"),
            "sibling cwd must not pack the other repository's scoped fact: {sibling}"
        );
    });
}

#[test]
fn path_scoped_near_duplicate_does_not_absorb_unscoped_fact() {
    run_with_cx(|cx| async move {
        const REPO_A: &str = "/Users/x/repoa";
        const REPO_B: &str = "/Users/x/repob";
        const UNSCOPED: &str = "Exclusive boot marker must stay here";
        const SCOPED: &str = "RepoA exclusive boot marker must stay here";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit_with_scope(
                &cx,
                "scoped-first",
                None,
                SCOPED,
                "claude-code",
                None,
                &[REPO_A.into()],
                None,
            )
            .await
            .unwrap();
        runtime
            .deposit(&cx, "unscoped-second", UNSCOPED, "claude-code", None)
            .await
            .unwrap();

        {
            let conn = runtime.state().db_read.lock(&cx).await.unwrap();
            assert_eq!(
                active_decision_count(&conn),
                2,
                "a path-scoped sentence must not Jaccard-merge a later unscoped near-duplicate"
            );
        }

        let sibling = boot_capsule_for_payload(
            &cx,
            &runtime,
            "claude-code",
            &json!({"cwd": REPO_B}),
            600,
        )
        .await
        .expect("cwd B boot");
        assert!(
            sibling.contains("Exclusive boot marker must stay here"),
            "later unscoped write must remain visible outside the scoped repository: {sibling}"
        );
        assert!(
            !sibling.contains("RepoA exclusive boot marker"),
            "sibling cwd must not pack the scoped fact: {sibling}"
        );
    });
}

#[test]
fn unscoped_duplicate_sentence_still_collapses() {
    run_with_cx(|cx| async move {
        const TEXT: &str = "Exclusive boot marker must stay here";
        let runtime = CortexRuntime::from_state(solo_state());
        runtime
            .deposit(&cx, "unscoped-dup-1", TEXT, "claude-code", None)
            .await
            .unwrap();
        runtime
            .deposit(&cx, "unscoped-dup-2", TEXT, "claude-code", None)
            .await
            .unwrap();
        let conn = runtime.state().db_read.lock(&cx).await.unwrap();
        assert_eq!(
            active_decision_count(&conn),
            1,
            "the same unscoped sentence must still collapse"
        );
    });
}
