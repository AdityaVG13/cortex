//! Public-API fixtures only: these do not validate an installed Claude release.
use cortex_kernel::runtime::{
    CortexRuntime,
    host_capture::{
        ADAPTER_VERSION, HostCaptureContext, HostCaptureGrant, HostOrigin, HostOriginBinding,
        HostRoute, normalize_host_event, resolve_host_invocation,
    },
};
use cortex_tests::support::run_with_cx;
use serde_json::json;

fn grant() -> HostCaptureGrant {
    HostCaptureGrant {
        key: "authorized-host".into(),
        scope: "repo-a".into(),
        host_version: "fixture-v1".into(),
        adapter_version: ADAPTER_VERSION.into(),
        max_bytes: 4096,
        live: true,
        history: true,
    }
}
fn context(key: &str, origin: HostOrigin) -> HostCaptureContext {
    HostCaptureContext {
        host_version: "fixture-v1".into(),
        session_id: "session-a".into(),
        generation: "generation-a".into(),
        original_event_key: Some(key.into()),
        origins: vec![HostOriginBinding {
            event_key: key.into(),
            origin,
        }],
    }
}
fn live(text: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"session_id":"session-a","hook_event_name":"UserPromptSubmit","prompt":text}),
    )
    .unwrap()
}
fn history(key: &str, text: &str) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&json!({"sessionId":"session-a","type":"user","uuid":key,"message":{"role":"user","content":text}})).unwrap();
    bytes.push(b'\n');
    bytes
}
#[test]
fn claude_2_1_260_structured_bash_and_control_records_replay_without_losing_fields() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let mut native = grant();
        native.host_version = "2.1.260".into();
        native.adapter_version = "claude-code-2.1.260-v1".into();
        runtime
            .register_host_capture(&cx, native.clone())
            .await
            .unwrap();
        let mut ctx = context("toolu_fixture", HostOrigin::External);
        ctx.host_version = "2.1.260".into();
        ctx.original_event_key = None;
        ctx.origins.push(HostOriginBinding {
            event_key: "final-uuid".into(),
            origin: HostOrigin::External,
        });
        let response = json!({"stdout":"out\n","stderr":"warning\n","interrupted":false,"isImage":false,"noOutputExpected":false});
        let live=serde_json::to_vec(&json!({"session_id":"session-a","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"printf fixture"},"tool_use_id":"toolu_fixture","tool_response":response})).unwrap();
        let tool = runtime
            .capture_host_event(&cx, &native.key, &ctx, &live)
            .await
            .unwrap();
        let exact = runtime
            .read_observation(&cx, &tool.accepted[0].source_id)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&exact.text).unwrap(),
            response
        );
        runtime
            .register_source(
                &cx,
                cortex_kernel::runtime::observation::SourceSpec::document("envelope-doc", "repo-a"),
            )
            .await
            .unwrap();
        let unrelated = runtime
            .observe(
                &cx,
                "envelope-doc",
                "g",
                cortex_kernel::runtime::observation::ObservationEvent {
                    event_key: "one".into(),
                    text: "stdout stderr interrupted isImage noOutputExpected false".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let (_, prepared) = runtime
            .capture_and_prepare_host(&cx, &native.key, &ctx, &live, "native-tool-context", None)
            .await
            .unwrap();
        assert!(
            prepared
                .unwrap()
                .evidence
                .iter()
                .all(|e| e.source_id != unrelated.source_id),
            "JSON envelope fields are not task cues"
        );
        let stop=serde_json::to_vec(&json!({"session_id":"session-a","hook_event_name":"Stop","prompt_id":"prompt-not-final-id","last_assistant_message":"finished"})).unwrap();
        assert!(
            runtime
                .capture_host_event(&cx, &native.key, &ctx, &stop)
                .await
                .unwrap_err()
                .contains("identity_required")
        );
        ctx.original_event_key = Some("final-uuid".into());
        let final_event = runtime
            .capture_host_event(&cx, &native.key, &ctx, &stop)
            .await
            .unwrap();
        let records = [
            json!({"type":"atis-latch","atis":"","sessionId":"session-a"}),
            json!({"type":"assistant","uuid":"request-uuid","sessionId":"session-a","version":"2.1.260","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","id":"toolu_fixture","name":"Bash","input":{"command":"printf fixture"}}]}}),
            json!({"type":"user","uuid":"tool-wrapper-uuid","sessionId":"session-a","version":"2.1.260","toolUseResult":response,"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_fixture","content":"rendered report"}]}}),
            json!({"type":"assistant","uuid":"final-uuid","sessionId":"session-a","version":"2.1.260","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"finished"}]}}),
            json!({"type":"last-prompt","lastPrompt":"fixture","leafUuid":"final-uuid","sessionId":"session-a"}),
        ];
        let mut raw = Vec::new();
        for record in records {
            raw.extend(serde_json::to_vec(&record).unwrap());
            raw.push(b'\n');
        }
        let tail = runtime
            .tail_host_transcript(&cx, &native.key, &ctx, 0, &raw)
            .await
            .unwrap();
        assert_eq!(tail.accepted.len(), 2);
        assert!(tail.accepted.iter().all(|r| r.duplicate));
        assert_eq!(tail.accepted[0].source_id, tool.accepted[0].source_id);
        assert_eq!(
            tail.accepted[1].source_id,
            final_event.accepted[0].source_id
        );
        assert_eq!(tail.next_offset, Some(raw.len() as u64));
        assert_eq!(serde_json::to_value(&tail).unwrap()["ignored_metadata"], 3);
        let unknown=br#"{"type":"attachment","sessionId":"session-a","version":"2.1.260","attachment":{"type":"unknown-private-shape"}}
"#;
        assert!(
            runtime
                .tail_host_transcript(&cx, &native.key, &ctx, raw.len() as u64, unknown)
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .host_capture_offset(&cx, &native.key, &ctx)
                .await
                .unwrap(),
            raw.len() as u64
        );
    });
}
#[test]
fn claude_2_1_260_prompt_identity_and_metadata_tail_match_observed_host_records() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let mut native = grant();
        native.host_version = "2.1.260".into();
        native.adapter_version = "claude-code-2.1.260-v1".into();
        runtime
            .register_host_capture(&cx, native.clone())
            .await
            .unwrap();
        let mut ctx = context("prompt-1", HostOrigin::External);
        ctx.host_version = "2.1.260".into();
        ctx.original_event_key = None;
        let live=serde_json::to_vec(&json!({"session_id":"session-a","hook_event_name":"UserPromptSubmit","prompt_id":"prompt-1","permission_mode":"default","prompt":"offline fixture"})).unwrap();
        let first = runtime
            .capture_host_event(&cx, &native.key, &ctx, &live)
            .await
            .unwrap();
        let records = [
            json!({"type":"queue-operation","operation":"enqueue","sessionId":"session-a","content":"offline fixture"}),
            json!({"type":"queue-operation","operation":"dequeue","sessionId":"session-a"}),
            json!({"type":"user","message":{"role":"user","content":"offline fixture"},"promptId":"prompt-1","uuid":"different-transcript-uuid","sessionId":"session-a","version":"2.1.260","timestamp":"2026-09-09T07:29:08.157Z"}),
            json!({"type":"attachment","attachment":{"type":"total_tokens_reminder","text":"fixture budget"},"uuid":"metadata-1","sessionId":"session-a","version":"2.1.260"}),
            json!({"type":"attachment","attachment":{"type":"hook_additional_context","content":["fixture delivery"],"hookEvent":"UserPromptSubmit"},"uuid":"metadata-2","sessionId":"session-a","version":"2.1.260"}),
        ];
        let mut raw = Vec::new();
        for record in records {
            raw.extend(serde_json::to_vec(&record).unwrap());
            raw.push(b'\n');
        }
        let tail = runtime
            .tail_host_transcript(&cx, &native.key, &ctx, 0, &raw)
            .await
            .unwrap();
        assert_eq!(tail.accepted.len(), 1);
        assert!(tail.accepted[0].duplicate);
        assert_eq!(tail.accepted[0].source_id, first.accepted[0].source_id);
        assert_eq!(tail.next_offset, Some(raw.len() as u64));
        assert_eq!(serde_json::to_value(&tail).unwrap()["ignored_metadata"], 4);
        let mut malformed = serde_json::to_vec(
            &json!({"type":"queue-operation","operation":"dequeue","sessionId":"session-a"}),
        )
        .unwrap();
        malformed.extend_from_slice(b"\n{\"type\":\"unknown\",\"sessionId\":\"session-a\"}\n");
        assert!(
            runtime
                .tail_host_transcript(&cx, &native.key, &ctx, raw.len() as u64, &malformed)
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .host_capture_offset(&cx, &native.key, &ctx)
                .await
                .unwrap(),
            raw.len() as u64
        );
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let metadata: i64 = conn
            .query_row("SELECT count(*) FROM host_capture_metadata", [], |r| {
                r.get(0)
            })
            .unwrap();
        let evidence: i64 = conn
            .query_row("SELECT count(*) FROM observation_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            metadata, 4,
            "metadata markers must roll back with the raw cursor"
        );
        assert_eq!(
            evidence, 1,
            "queue copies and delivered context must not become evidence"
        );
        drop(conn);
        ctx.original_event_key = Some("wrong-id".into());
        assert!(
            runtime
                .capture_host_event(&cx, &native.key, &ctx, &live)
                .await
                .unwrap_err()
                .contains("identity_mismatch")
        );
    });
}
#[test]
fn fixture_normalization_preserves_user_tool_final_and_original_keys() {
    let text = " \n quoted α\t\r\n ";
    let ctx = context("uuid-1", HostOrigin::External);
    let a = normalize_host_event(&grant(), &ctx, HostRoute::Live, &live(text)).unwrap();
    let b =
        normalize_host_event(&grant(), &ctx, HostRoute::History, &history("uuid-1", text)).unwrap();
    assert_eq!(a.event, b.event);
    assert_eq!(a.event.text, text);
    for (live, historical) in [
        (
            json!({"session_id":"session-a","hook_event_name":"PostToolUse","tool_use_id":"tool-1","tool_response":text}),
            json!({"sessionId":"session-a","type":"user","uuid":"wrapper-id","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":text}]}}),
        ),
        (
            json!({"session_id":"session-a","hook_event_name":"Stop","last_assistant_message":text}),
            json!({"sessionId":"session-a","type":"assistant","uuid":"uuid-1","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]}}),
        ),
    ] {
        let a = normalize_host_event(
            &grant(),
            &ctx,
            HostRoute::Live,
            &serde_json::to_vec(&live).unwrap(),
        )
        .unwrap();
        let b = normalize_host_event(
            &grant(),
            &ctx,
            HostRoute::History,
            &serde_json::to_vec(&historical).unwrap(),
        )
        .unwrap();
        assert_eq!(a.event, b.event);
        assert_eq!(a.event.text, text);
    }
    let mut missing = ctx.clone();
    missing.original_event_key = None;
    assert!(
        normalize_host_event(&grant(), &missing, HostRoute::Live, &live(text))
            .unwrap_err()
            .contains("identity_required")
    );
    let mut wrong = ctx.clone();
    wrong.host_version = "unknown".into();
    assert!(normalize_host_event(&grant(), &wrong, HostRoute::Live, &live(text)).is_err());
    let forged = json!({"session_id":"session-a","hook_event_name":"UserPromptSubmit","prompt":"x","role":"document"});
    assert!(
        normalize_host_event(
            &grant(),
            &ctx,
            HostRoute::Live,
            &serde_json::to_vec(&forged).unwrap()
        )
        .is_err()
    );
}
#[test]
fn live_history_overlap_exact_lookup_and_raw_cursor_survive_reopen() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("brain.db");
        let runtime = CortexRuntime::open_db(&path).unwrap();
        let ctx = context("event-1", HostOrigin::External);
        let bytes = live(" \nbody\t ");
        assert!(
            runtime
                .capture_host_event(&cx, &grant().key, &ctx, &bytes)
                .await
                .is_err()
        );
        runtime.register_host_capture(&cx, grant()).await.unwrap();
        let first = runtime
            .capture_host_event(&cx, &grant().key, &ctx, &bytes)
            .await
            .unwrap();
        let captured = runtime
            .read_observation(&cx, &first.accepted[0].source_id)
            .await
            .unwrap();
        assert_eq!(captured.text, " \nbody\t ");
        assert_eq!(captured.role, "user_statement");
        let (_, view) = runtime
            .capture_and_prepare_host(&cx, &grant().key, &ctx, &bytes, "invocation-a", None)
            .await
            .unwrap();
        let view = view.unwrap();
        assert_eq!(view.status, "ready");
        assert!(view.payload_bytes > 0);
        let (_, present) = runtime
            .capture_and_prepare_host(
                &cx,
                &grant().key,
                &ctx,
                &bytes,
                "invocation-a",
                view.delivery_id.as_deref(),
            )
            .await
            .unwrap();
        assert_eq!(present.unwrap().payload_bytes, 0);
        let (_, compacted) = runtime
            .capture_and_prepare_host(
                &cx,
                &grant().key,
                &ctx,
                &bytes,
                "invocation-b",
                view.delivery_id.as_deref(),
            )
            .await
            .unwrap();
        assert!(compacted.unwrap().payload_bytes > 0);
        let mut chunk = history("event-1", " \nbody\t ");
        let cutoff = chunk.len();
        chunk.extend_from_slice(b"{\"partial\"");
        let tail = runtime
            .tail_host_transcript(&cx, &grant().key, &ctx, 0, &chunk)
            .await
            .unwrap();
        assert!(tail.accepted[0].duplicate);
        assert_eq!(tail.accepted[0].source_id, first.accepted[0].source_id);
        assert_eq!(tail.next_offset, Some(cutoff as u64));
        assert_eq!(tail.uncommitted_tail_bytes, chunk.len() - cutoff);
        drop(runtime);
        let runtime = CortexRuntime::open_db(&path).unwrap();
        assert_eq!(
            runtime
                .host_capture_offset(&cx, &grant().key, &ctx)
                .await
                .unwrap(),
            cutoff as u64
        );
        assert!(
            runtime
                .tail_host_transcript(&cx, &grant().key, &ctx, 0, &chunk)
                .await
                .unwrap_err()
                .contains("cursor_conflict")
        );
    });
}
#[test]
fn malformed_unknown_private_and_unresolved_origins_never_advance_cursor() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime.register_host_capture(&cx, grant()).await.unwrap();
        let ctx = context("event-1", HostOrigin::External);
        for suffix in [
            b"not-json\n".as_slice(),
            b"{}\n",
            b"{\"sessionId\":\"session-a\",\"type\":\"thinking\"}\n",
        ] {
            let mut chunk = history("event-1", "must roll back");
            chunk.extend_from_slice(suffix);
            assert!(
                runtime
                    .tail_host_transcript(&cx, &grant().key, &ctx, 0, &chunk)
                    .await
                    .is_err()
            );
            assert_eq!(
                runtime
                    .host_capture_offset(&cx, &grant().key, &ctx)
                    .await
                    .unwrap(),
                0
            );
        }
        let unknown = context("event-1", HostOrigin::Unknown);
        assert!(
            runtime
                .tail_host_transcript(&cx, &grant().key, &unknown, 0, &history("event-1", "x"))
                .await
                .unwrap_err()
                .contains("origin_unresolved")
        );
        let first = runtime
            .capture_host_event(
                &cx,
                &grant().key,
                &ctx,
                &live("different body proves rollback"),
            )
            .await
            .unwrap();
        assert!(!first.accepted[0].duplicate);
        let private = json!({"sessionId":"session-a","type":"assistant","uuid":"event-1","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"thinking","thinking":"private"}]}});
        assert!(
            normalize_host_event(
                &grant(),
                &ctx,
                HostRoute::History,
                &serde_json::to_vec(&private).unwrap()
            )
            .is_err()
        );
    });
}
#[test]
fn own_delivery_is_not_evidence_and_cannot_be_promoted_on_replay() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime.register_host_capture(&cx, grant()).await.unwrap();
        let own = context("delivery-1", HostOrigin::CortexDelivery);
        let result = runtime
            .capture_host_event(&cx, &grant().key, &own, &live("Cortex context"))
            .await
            .unwrap();
        assert!(result.accepted.is_empty());
        assert_eq!(result.excluded_deliveries, 1);
        let forged = context("delivery-1", HostOrigin::External);
        assert!(
            runtime
                .capture_host_event(&cx, &grant().key, &forged, &live("Cortex context"))
                .await
                .unwrap_err()
                .contains("origin_identity_conflict")
        );
        let raw = history("delivery-1", "Cortex context");
        let result = runtime
            .tail_host_transcript(&cx, &grant().key, &own, 0, &raw)
            .await
            .unwrap();
        assert!(result.accepted.is_empty());
        assert_eq!(result.next_offset, Some(raw.len() as u64));
        // Identical bytes from a genuinely new external occurrence remain new.
        let correction = context("user-correction", HostOrigin::External);
        assert_eq!(
            runtime
                .capture_host_event(&cx, &grant().key, &correction, &live("Cortex context"))
                .await
                .unwrap()
                .accepted
                .len(),
            1
        );
    });
}
#[test]
fn file_tool_reports_keep_exact_path_and_ignore_envelope_keys() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime.register_host_capture(&cx, grant()).await.unwrap();
        runtime
            .register_source(
                &cx,
                cortex_kernel::runtime::observation::SourceSpec::document("notes", "repo-a"),
            )
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                cortex_kernel::runtime::observation::ObservationEvent {
                    event_key: "path-note".into(),
                    text: "payments/retry.rs must stay idempotent".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                cortex_kernel::runtime::observation::ObservationEvent {
                    event_key: "json-keys".into(),
                    text: "filePath content newString tool_response".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let ctx = context("tool-edit", HostOrigin::External);
        let live = serde_json::to_vec(&json!({
            "session_id":"session-a",
            "hook_event_name":"PostToolUse",
            "tool_name":"Edit",
            "tool_use_id":"tool-edit",
            "tool_response":{"filePath":"payments/retry.rs","oldString":"old","newString":"new"}
        }))
        .unwrap();
        let captured = runtime
            .capture_host_event(&cx, &grant().key, &ctx, &live)
            .await
            .unwrap();
        let exact = runtime
            .read_observation(&cx, &captured.accepted[0].source_id)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&exact.text).unwrap()["filePath"],
            "payments/retry.rs"
        );
        let (_, prepared) = runtime
            .capture_and_prepare_host(&cx, &grant().key, &ctx, &live, "edit-context", None)
            .await
            .unwrap();
        let view = prepared.unwrap();
        assert!(
            view.evidence
                .iter()
                .any(|item| item.text.contains("idempotent"))
        );
        assert!(
            view.evidence
                .iter()
                .all(|item| !item.text.contains("tool_response")),
            "JSON field names are not retrieval cues"
        );
        let read = serde_json::to_vec(&json!({
            "session_id":"session-a",
            "hook_event_name":"PostToolUse",
            "tool_name":"Read",
            "tool_use_id":"tool-read",
            "tool_response":{"file":{"filePath":"payments/retry.rs","content":"fn retry() {}"}}
        }))
        .unwrap();
        let read_ctx = context("tool-read", HostOrigin::External);
        let read_capture = runtime
            .capture_host_event(&cx, &grant().key, &read_ctx, &read)
            .await
            .unwrap();
        assert_eq!(read_capture.accepted.len(), 1);
        let compact = serde_json::to_vec(&json!({
            "session_id":"session-a",
            "hook_event_name":"PreCompact",
            "trigger":"auto"
        }))
        .unwrap();
        let compact_ctx = context("session-a", HostOrigin::External);
        let compacted = runtime
            .capture_host_event(&cx, &grant().key, &compact_ctx, &compact)
            .await
            .unwrap();
        assert!(compacted.accepted.is_empty());
        assert_eq!(compacted.ignored_metadata, 1);
    });
}
#[test]
fn pretool_prepares_from_path_without_storing_the_request() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime.register_host_capture(&cx, grant()).await.unwrap();
        runtime
            .register_source(
                &cx,
                cortex_kernel::runtime::observation::SourceSpec::document("notes", "repo-a"),
            )
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                cortex_kernel::runtime::observation::ObservationEvent {
                    event_key: "one".into(),
                    text: "src/ledger.rs write must be atomic".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let raw = serde_json::to_vec(&json!({
            "session_id":"session-a",
            "hook_event_name":"PreToolUse",
            "tool_name":"Read",
            "tool_use_id":"tool-pre",
            "tool_input":{"file_path":"src/ledger.rs"}
        }))
        .unwrap();
        let ctx = context("tool-pre", HostOrigin::External);
        let (receipt, view) = runtime
            .capture_and_prepare_host(&cx, &grant().key, &ctx, &raw, "pretool-context", None)
            .await
            .unwrap();
        assert!(receipt.accepted.is_empty());
        assert!(
            view.unwrap()
                .evidence
                .iter()
                .any(|item| item.text.contains("atomic"))
        );
        let events: i64 = runtime
            .state()
            .db
            .lock(&cx)
            .await
            .unwrap()
            .query_row("SELECT count(*) FROM observation_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1, "a tool request is not an observation");
    });
}

#[test]
fn sidecar_without_native_flags_uses_explicit_trusted_fields() {
    let sidecar = json!({
        "grant":"approved",
        "host_version":"fixture-v1",
        "session":"s",
        "generation":"g",
        "event_key":"u1",
        "origins":{"u1":"external"},
        "context":"c1"
    });
    let invocation = resolve_host_invocation("PostToolUse", &sidecar, &json!({})).unwrap().unwrap();
    assert_eq!(invocation.grant, "approved");
    assert_eq!(invocation.context.session_id, "s");
    assert_eq!(invocation.context.original_event_key.as_deref(), Some("u1"));
    assert_eq!(invocation.invocation_context, "c1");
}

#[test]
fn sidecar_user_prompt_derives_session_and_prompt_identity() {
    let session = "225407e0-7c32-4812-b130-aa0d54039690";
    let prompt = "ea0dd413-83bc-43a2-8913-506964f73808";
    let sidecar = json!({"grant":"native-user","host_version":"fixture-v1","native_user_prompts":true});
    let payload = json!({
        "session_id":session,
        "prompt_id":prompt,
        "hook_event_name":"UserPromptSubmit",
        "prompt":"offline fixture"
    });
    let invocation = resolve_host_invocation("UserPromptSubmit", &sidecar, &payload).unwrap().unwrap();
    assert_eq!(invocation.context.session_id, session);
    assert_eq!(invocation.context.original_event_key.as_deref(), Some(prompt));
    assert_eq!(invocation.invocation_context, format!("claude-user:{session}:{prompt}"));
    assert!(
        resolve_host_invocation(
            "UserPromptSubmit",
            &sidecar,
            &json!({
                "session_id":session,
                "prompt_id":"not-a-native-uuid",
                "hook_event_name":"UserPromptSubmit",
                "prompt":"offline fixture"
            })
        )
        .is_err()
    );
}

#[test]
fn sidecar_bash_flag_does_not_take_file_tools_and_file_flag_does_not_take_bash() {
    let session = "225407e0-7c32-4812-b130-aa0d54039690";
    let bash = json!({
        "session_id":session,
        "tool_use_id":"toolu_fixture_1",
        "hook_event_name":"PostToolUse",
        "tool_name":"Bash",
        "tool_response":{"stdout":"fixture","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false}
    });
    let read = json!({
        "session_id":session,
        "tool_use_id":"toolu_read_1",
        "hook_event_name":"PostToolUse",
        "tool_name":"Read",
        "tool_response":{"filePath":"/tmp/a.rs","content":"fn main() {}"}
    });
    assert!(
        resolve_host_invocation(
            "PostToolUse",
            &json!({"grant":"native-tools","host_version":"fixture-v1","native_bash_results":true}),
            &read
        )
        .unwrap()
        .is_none()
    );
    let bash_invocation = resolve_host_invocation(
        "PostToolUse",
        &json!({"grant":"native-tools","host_version":"fixture-v1","native_bash_results":true}),
        &bash,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        bash_invocation.invocation_context,
        format!("claude-tool:{session}:toolu_fixture_1")
    );
    assert!(
        resolve_host_invocation(
            "PostToolUse",
            &json!({"grant":"native-files","host_version":"fixture-v1","native_file_results":true}),
            &bash
        )
        .unwrap()
        .is_none()
    );
    let file_invocation = resolve_host_invocation(
        "PostToolUse",
        &json!({"grant":"native-files","host_version":"fixture-v1","native_file_results":true}),
        &read,
    )
    .unwrap()
    .unwrap();
    assert_eq!(file_invocation.context.original_event_key.as_deref(), Some("toolu_read_1"));
}

const REPO_A: &str = "/Users/x/repoa";
const REPO_B: &str = "/Users/x/repob";

fn project_grant() -> HostCaptureGrant {
    HostCaptureGrant {
        key: "cwd-host".into(),
        scope: "project".into(),
        host_version: "fixture-v1".into(),
        adapter_version: ADAPTER_VERSION.into(),
        max_bytes: 4096,
        live: true,
        history: true,
    }
}

fn live_prompt(text: &str, cwd: Option<&str>) -> Vec<u8> {
    let mut value = json!({
        "session_id": "session-a",
        "hook_event_name": "UserPromptSubmit",
        "prompt": text
    });
    if let Some(cwd) = cwd {
        value["cwd"] = json!(cwd);
    }
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn host_capture_with_cwd_stays_in_that_repository() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let grant = project_grant();
        runtime
            .register_host_capture(&cx, grant.clone())
            .await
            .unwrap();

        let unscoped = runtime
            .capture_host_event(
                &cx,
                &grant.key,
                &context("cwd-unscoped", HostOrigin::External),
                &live_prompt("HOST-CWD-UNSCOPED signing cache warm", None),
            )
            .await
            .unwrap();
        let in_a = runtime
            .capture_host_event(
                &cx,
                &grant.key,
                &context("cwd-a", HostOrigin::External),
                &live_prompt("HOST-CWD-A ledger retry in payments", Some(REPO_A)),
            )
            .await
            .unwrap();
        let in_b = runtime
            .capture_host_event(
                &cx,
                &grant.key,
                &context("cwd-b", HostOrigin::External),
                &live_prompt("HOST-CWD-B signing key rotation", Some(REPO_B)),
            )
            .await
            .unwrap();
        assert_eq!(unscoped.accepted.len(), 1);
        assert_eq!(in_a.accepted.len(), 1);
        assert_eq!(in_b.accepted.len(), 1);

        let retry = runtime
            .capture_host_event(
                &cx,
                &grant.key,
                &context("cwd-a", HostOrigin::External),
                &live_prompt("HOST-CWD-A ledger retry in payments", None),
            )
            .await
            .unwrap();
        assert!(
            retry.accepted.iter().all(|r| r.duplicate),
            "a later capture of the same event without cwd must reuse the first source, not mint a project row: {retry:?}"
        );

        let project = runtime
            .query_observations(&cx, "project", "HOST-CWD", 32, 65536, false)
            .await
            .unwrap();
        assert!(
            project
                .source_refs
                .contains(&unscoped.accepted[0].source_id),
            "events without cwd stay in the grant bucket: {project:?}"
        );
        assert!(
            !project.source_refs.contains(&in_a.accepted[0].source_id),
            "cwd-stamped host capture must not leak into an exact project pull: {project:?}"
        );
        assert!(
            !project.source_refs.contains(&in_b.accepted[0].source_id),
            "cwd-stamped host capture must not leak into an exact project pull: {project:?}"
        );

        let paths_a = vec![REPO_A.to_string()];
        let scoped_a = runtime
            .query_observations_for_paths(&cx, "HOST-CWD", &paths_a, None, 32, 65536, false)
            .await
            .unwrap();
        assert!(
            scoped_a
                .source_refs
                .contains(&unscoped.accepted[0].source_id),
            "path pull still surfaces the unscoped project bucket: {scoped_a:?}"
        );
        assert!(
            scoped_a.source_refs.contains(&in_a.accepted[0].source_id),
            "path pull must return host evidence captured in that repository: {scoped_a:?}"
        );
        assert!(
            !scoped_a.source_refs.contains(&in_b.accepted[0].source_id),
            "path pull must not return the sibling repository's host evidence: {scoped_a:?}"
        );

        let paths_b = vec![REPO_B.to_string()];
        let scoped_b = runtime
            .query_observations_for_paths(&cx, "HOST-CWD", &paths_b, None, 32, 65536, false)
            .await
            .unwrap();
        assert!(
            scoped_b.source_refs.contains(&in_b.accepted[0].source_id),
            "sibling path pull must return its own host evidence: {scoped_b:?}"
        );
        assert!(
            !scoped_b.source_refs.contains(&in_a.accepted[0].source_id),
            "sibling path pull must not return this repository's host evidence: {scoped_b:?}"
        );

        let (_, prepared) = runtime
            .capture_and_prepare_host(
                &cx,
                &grant.key,
                &context("cwd-a", HostOrigin::External),
                &live_prompt("HOST-CWD-A ledger retry in payments", Some(REPO_A)),
                "native-cwd-context",
                None,
            )
            .await
            .unwrap();
        let prepared = prepared.expect("cwd capture must prepare a need in that repository");
        assert!(
            prepared
                .evidence
                .iter()
                .any(|e| e.source_id == in_a.accepted[0].source_id),
            "prepare must join host evidence in the payload cwd: {prepared:?}"
        );
        assert!(
            prepared
                .evidence
                .iter()
                .all(|e| e.source_id != in_b.accepted[0].source_id),
            "prepare in one repository must not join the sibling's host evidence: {prepared:?}"
        );
    });
}
