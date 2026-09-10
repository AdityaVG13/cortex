//! Public-API fixtures only: these do not validate an installed Claude release.
use cortex_daemon::runtime::{
    CortexRuntime,
    host_capture::{
        ADAPTER_VERSION, HostCaptureContext, HostCaptureGrant, HostOrigin, HostOriginBinding,
        HostRoute, normalize_host_event,
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
                cortex_daemon::runtime::observation::SourceSpec::document("envelope-doc", "repo-a"),
            )
            .await
            .unwrap();
        let unrelated = runtime
            .observe(
                &cx,
                "envelope-doc",
                "g",
                cortex_daemon::runtime::observation::ObservationEvent {
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
