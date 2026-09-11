//! Retired listener-only malformed-body/status and bearer-header tests.
//! Surviving validation, corruption honesty, and concurrent losslessness use
//! explicit capabilities at the kernel/MCP seams, not fabricated responses.
use cortex_daemon::handlers::health::build_health_payload;
use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;
use cortex_kernel::CortexRuntime;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

#[test]
fn mcp_rejects_vague_decision_with_validation_evidence() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let response = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cortex_store","arguments":{"decision":"x"}}}), None, None).await.unwrap();
        assert_eq!(response["id"], 1);
        // Commit carries domain denial inside the tool payload, not JSON-RPC error.
        assert!(response.get("error").is_none(), "{response}");
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let error: Value = serde_json::from_str(text).unwrap();
        assert!(
            error["error"]
                .as_str()
                .unwrap()
                .contains("Memory too vague"),
            "{error}"
        );
        assert_eq!(error["status"], "unavailable");
        let mut conn = state.db.lock(&cx).await.unwrap();
        // The adapter serializes an error string; structured quality evidence
        // remains available at the typed store boundary.
        use cortex_kernel::handlers::store::{
            store_decision_with_input_embedding_and_provenance_retention, DecisionProvenance,
            StoreError,
        };
        let denied = store_decision_with_input_embedding_and_provenance_retention(
            &mut conn,
            "x",
            None,
            Some("decision".into()),
            "failure-classes".into(),
            DecisionProvenance::from_fields("failure-classes", None, None),
            None,
            None,
            None,
            None,
            None,
        )
        .expect_err("vague input");
        match denied {
            StoreError::Validation {
                message,
                quality,
                factors,
            } => {
                assert_eq!(message, "Memory too vague");
                assert_eq!(
                    quality,
                    factors.length_score + factors.specificity_bonus + factors.question_penalty
                );
                assert!(factors.as_json().is_object());
            }
            other => panic!("expected structured validation, got {other:?}"),
        }
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn health_reports_db_corruption_without_crashing() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        state.db_corrupted.store(true, Ordering::SeqCst);
        let payload = build_health_payload(&cx, &state, false).await.unwrap();
        assert_eq!(payload["status"], "degraded");
        assert_eq!(payload["db_corrupted"], true);
        assert_eq!(payload["degraded"], true);
    });
}

#[test]
fn concurrent_store_requests_serialize_without_loss() {
    let runtime = CortexRuntime::from_state(solo_state());
    let barrier = Arc::new(Barrier::new(12));
    let handles: Vec<_> = (0..12).map(|i| {
        let runtime = runtime.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            run_with_cx(|cx| async move {
                runtime.deposit(&cx, &format!("concurrent-{i}"), &format!("concurrent sentinel memory number {i} with enough specificity to pass the quality gate"), "failure-classes", None).await.expect("each concurrent deposit acknowledged");
            });
        })
    }).collect();
    for handle in handles {
        handle.join().unwrap();
    }
    run_with_cx(|cx| async move {
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let (rows, merged, decision, context): (i64, i64, String, String) = conn.query_row("SELECT COUNT(*), COALESCE(MAX(merged_count), -1), COALESCE(MAX(decision), ''), COALESCE(MAX(context), '') FROM decisions WHERE decision LIKE 'concurrent sentinel memory number %'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
        assert_eq!(rows, 1);
        assert_eq!(merged, 11);
        let all = format!("{decision}\n\n{context}");
        for i in 0..12 {
            assert!(all.contains(&format!("concurrent sentinel memory number {i} with enough specificity to pass the quality gate")), "ack {i} lost");
        }
        let (stored, merges): (i64, i64) = conn.query_row("SELECT (SELECT COUNT(*) FROM events WHERE type = 'decision_stored'), (SELECT COUNT(*) FROM events WHERE type = 'decision_agreement_merge')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(stored, 1);
        assert_eq!(merges, 11);
    });
}
