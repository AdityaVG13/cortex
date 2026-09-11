//! Wave 9: Reflex snapshot. Derived, versioned, atomically published,
//! discardable; an expired snapshot is `Expired`, never "no memory"; warm
//! Level-0 answers in well under a millisecond on the fixture corpus and the
//! fallback rate is reported, never hidden. Capture scope pause/stop is
//! honored by the hook and inspectable through brain health.

use cortex_logic::adapter::{CapabilityManifest, HookDecision, SnapshotState};
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::hook_event::{frame_from_host, process};
use cortex_kernel::reflex;
use cortex_kernel::runtime::CortexRuntime;
use cortex_tests::support::solo_state;
use serde_json::json;

async fn seed(cx: &asupersync::Cx, runtime: &CortexRuntime, n: usize) {
    for i in 0..n {
        let agent = format!("seed-{i}");
        let caller = Caller {
            owner_id: None,
            agent: &agent,
            principal: "solo".into(),
        };
        let words = [
            "gateway",
            "ledger",
            "cache",
            "auth",
            "billing",
            "search",
            "export",
            "import",
            "queue",
            "scheduler",
            "metrics",
            "webhook",
        ];
        let w = words[i % words.len()];
        // Distinct vocabulary per row so the refinement policy does not fold
        // the corpus into one lineage; only the ticket anchor is shared shape.
        let filler: String = (0..6)
            .map(|k| {
                format!(
                    "{}{}",
                    [
                        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
                        "india", "juliet", "kilo", "lima", "mike", "november", "oscar", "papa",
                        "quebec", "romeo", "sierra", "tango"
                    ][(i * 7 + k * 3) % 20],
                    i * 31 + k
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        let text = format!("RFX-{i} {w} {filler} src/{w}{i}/x.rs");
        dispatch(
            cx,
            runtime.state(),
            caller,
            Operation::Commit,
            &json!({"decision": text}),
        )
        .await
        .unwrap();
    }
}

#[test]
fn snapshot_is_published_atomically_validated_against_the_frontier_and_discardable() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let runtime = CortexRuntime::from_state(solo_state());
        seed(cx, &runtime, 12).await;
        let path = reflex::snapshot_path(&runtime.state().home);
        let snapshot = {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            let s = reflex::build(&conn, 1, reflex::DEFAULT_MAX_RECORDS).unwrap();
            assert_eq!(s.header.records, 12);
            assert_eq!(s.header.format_version, reflex::REFLEX_FORMAT_VERSION);
            assert!(
                s.anchor_dictionary.contains_key("rfx-3"),
                "{:?}",
                s.anchor_dictionary.keys().take(10).collect::<Vec<_>>()
            );
            reflex::publish(&s, &path).unwrap();
            assert!(
                !path.parent().unwrap().join(".snapshot.1.tmp").exists(),
                "temp file renamed away"
            );
            s
        };
        let loaded = reflex::load(&path).expect("loaded");
        assert_eq!(loaded, snapshot);
        {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            assert_eq!(
                reflex::state_for(Some(&loaded), &conn),
                SnapshotState::Fresh
            );
            assert_eq!(reflex::state_for(None, &conn), SnapshotState::Unavailable);
        }
        // Warm Level-0: exact ticket anchor → pre-rendered line, no I/O.
        let warm = reflex::level0(&loaded, "what is the RFX-3 retry budget?", None, 4);
        assert!(!warm.fallback, "{warm:?}");
        assert!(warm.hits[0].line.contains("RFX-3"), "{warm:?}");
        let miss = reflex::level0(&loaded, "something the brain has never seen", None, 4);
        assert!(miss.fallback);
        // A new commit moves the frontier: the old generation is Expired, not empty.
        seed(cx, &runtime, 1).await;
        {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            assert_eq!(
                reflex::state_for(Some(&loaded), &conn),
                SnapshotState::Expired
            );
        }
        // Readers keep their generation while a new one is published beside them.
        {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            let next = reflex::build(&conn, 2, reflex::DEFAULT_MAX_RECORDS).unwrap();
            reflex::publish(&next, &path).unwrap();
            assert_eq!(reflex::load(&path).unwrap().header.generation, 2);
            assert_eq!(
                loaded.header.generation, 1,
                "the held snapshot is never mutated"
            );
            assert_eq!(
                reflex::state_for(reflex::load(&path).as_ref(), &conn),
                SnapshotState::Fresh
            );
        }
        // Deleting the snapshot loses nothing: the hook falls back to deep retrieval.
        std::fs::remove_file(&path).unwrap();
        let turn = json!({"hook_event_name": "UserPromptSubmit", "session_id": "r", "prompt": "RFX-3 retry budget"});
        let frame = frame_from_host(
            "UserPromptSubmit",
            &turn,
            CapabilityManifest::claude_code_plugin(),
        );
        let out = process(cx, &runtime, "hook", &frame, &turn).await.unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Deliver, "{out:?}");
        assert_eq!(out.reflex["fallback"], true);
        assert!(
            out.additional_context.contains("RFX-3"),
            "{}",
            out.additional_context
        );
        // With a fresh snapshot the same turn is answered warm.
        {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            reflex::publish(
                &reflex::build(&conn, 3, reflex::DEFAULT_MAX_RECORDS).unwrap(),
                &path,
            )
            .unwrap();
        }
        let out = process(cx, &runtime, "hook", &frame, &turn).await.unwrap();
        assert_eq!(out.reflex["level0"], true, "{out:?}");
        assert!(
            out.additional_context.contains("warm reflex"),
            "{}",
            out.additional_context
        );
    });
}

/// Benchmark harness: warm Level-0 at a declared percentile on a declared
/// corpus. Numbers are printed with the fallback rate; the gate is generous
/// (debug build) and the report is the deliverable.
#[test]
fn warm_level0_latency_is_reported_with_fallback_rate() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let runtime = CortexRuntime::from_state(solo_state());
        seed(cx, &runtime, 200).await;
        let snapshot = {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            reflex::build(&conn, 1, reflex::DEFAULT_MAX_RECORDS).unwrap()
        };
        let queries: Vec<String> = (0..300)
            .map(|i| {
                if i % 3 == 0 {
                    format!("unrelated question number {i}")
                } else {
                    format!("RFX-{} retry budget", i % 200)
                }
            })
            .collect();
        let mut micros = Vec::new();
        let mut fallbacks = 0usize;
        for q in &queries {
            let out = reflex::level0(&snapshot, q, None, 4);
            micros.push(out.micros);
            if out.fallback {
                fallbacks += 1;
            }
        }
        micros.sort();
        let p50 = reflex::percentile(&micros, 50.0);
        let p99 = reflex::percentile(&micros, 99.0);
        let report = json!({
            "corpus_records": snapshot.header.records,
            "queries": queries.len(),
            "p50_micros": p50,
            "p99_micros": p99,
            "fallback_rate": fallbacks as f64 / queries.len() as f64,
            "hardware": std::env::consts::ARCH,
            "build": if cfg!(debug_assertions) { "debug" } else { "release" },
            "measures": "level0 only: no spawn, no page faults, no ACL, no tokenizer, no rendering, no writes, no deep retrieval",
        });
        eprintln!("REFLEX_BENCH {report}");
        assert!(
            (fallbacks as f64 / queries.len() as f64 - 1.0 / 3.0).abs() < 0.05,
            "fallback rate must be reported honestly: {report}"
        );
        assert!(
            p99 < 5_000,
            "warm Level-0 p99 {p99}µs exceeds the debug-build gate (5 ms): {report}"
        );
    });
}

#[test]
fn capture_scope_pause_and_stop_are_honored_and_inspectable() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        use cortex_kernel::db::capture_policy::{self, CaptureState};
        let runtime = CortexRuntime::from_state(solo_state());
        let payload = json!({"hook_event_name": "PostToolUse", "session_id": "c", "cwd": "/Users/x/repoa", "tool_name": "Bash", "tool_use_id": "t1", "tool_input": {"command": "cargo test"}, "tool_response": {"stdout": "test result: FAILED. 1 passed; 2 failed", "exit_code": 101}});
        let frame = frame_from_host(
            "PostToolUse",
            &payload,
            CapabilityManifest::claude_code_plugin(),
        );
        {
            let conn = runtime.state().db.lock(cx).await.unwrap();
            capture_policy::set_state(
                &conn,
                "/Users/x/repoa",
                CaptureState::Paused,
                Some("operator pause"),
            )
            .unwrap();
        }
        let out = process(cx, &runtime, "hook", &frame, &payload)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Noop);
        assert!(!out.outcome.automatic_capture);
        assert!(out.outcome.reason.contains("paused"), "{out:?}");
        // Paused still delivers; stopped does not.
        let turn = json!({"hook_event_name": "UserPromptSubmit", "session_id": "c", "cwd": "/Users/x/repoa", "prompt": "anything"});
        let tf = frame_from_host(
            "UserPromptSubmit",
            &turn,
            CapabilityManifest::claude_code_plugin(),
        );
        assert_ne!(
            process(cx, &runtime, "hook", &tf, &turn)
                .await
                .unwrap()
                .outcome
                .reason,
            "capture stopped for scope: no automatic delivery"
        );
        {
            let conn = runtime.state().db.lock(cx).await.unwrap();
            capture_policy::set_state(&conn, "*", CaptureState::Stopped, None).unwrap();
            assert_eq!(
                capture_policy::state_for(&conn, "/Users/x/repoa"),
                CaptureState::Paused,
                "exact scope wins over global"
            );
            assert_eq!(
                capture_policy::state_for(&conn, "/elsewhere"),
                CaptureState::Stopped
            );
        }
        let elsewhere = json!({"hook_event_name": "UserPromptSubmit", "session_id": "c", "cwd": "/elsewhere", "prompt": "anything"});
        let ef = frame_from_host(
            "UserPromptSubmit",
            &elsewhere,
            CapabilityManifest::claude_code_plugin(),
        );
        let out = process(cx, &runtime, "hook", &ef, &elsewhere)
            .await
            .unwrap();
        assert_eq!(out.outcome.decision, HookDecision::Noop);
        assert!(out.outcome.reason.contains("stopped"));
        // Brain health exposes the policy, capture receipts and reflex state.
        let health = {
            let conn = runtime.state().db_read.lock(cx).await.unwrap();
            cortex_kernel::db::outbox::brain_health(&conn, &runtime.state().home)
        };
        assert_eq!(health["capture_policy"]["global"], "stopped");
        assert!(
            health["capture_policy"]["scopes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["scope"] == "/Users/x/repoa" && s["state"] == "paused"),
            "{health}"
        );
        assert!(health["capture_receipts"]["hook_captures"].is_number());
        assert_eq!(health["reflex"]["state"], "unavailable");
        assert!(
            health["commit_durability"].is_string() && health["last_verified_restore"].is_null()
                || health["last_verified_restore"].is_object()
        );
    });
}
