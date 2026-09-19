//! Eight-operation contracts over MCP and the library: tools/list advertises
//! exactly the eight operations; legacy names remain callable and no tool
//! answers with a fabricated `{"ok":true}`; commit → orient/query → expand
//! roundtrip with receipt-scoped aliases; coverage names unmet needs;
//! needs_more_budget never truncates a Card.

use cortex_daemon::handlers::mcp::handle_mcp_message_with_caller;

use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_logic::lens::{EvidenceDepth, LensProfile, Need, NeedFrame};
use cortex_tests::support::solo_state;
use serde_json::{json, Value};

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "ops-agent",
        principal: "solo".into(),
    }
}

fn tool_text(body: &Value) -> Value {
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool text missing: {body}"));
    serde_json::from_str(text).unwrap_or_else(|_| json!({"raw": text}))
}

#[test]
fn lens_profiles_needs_and_handles_parse_conservatively() {
    assert_eq!(LensProfile::parse("Orient"), Some(LensProfile::Orient));
    assert_eq!(LensProfile::parse("summarize"), None);
    assert_eq!(Need::parse("failed_attempts"), Some(Need::FailedAttempts));
    assert_eq!(
        Need::parse("recipe:retry-policy"),
        Some(Need::Recipe {
            name: "retry-policy".into()
        })
    );
    let frame = NeedFrame::build(
        LensProfile::Orient,
        "what did PAY-77 change in src/pay/retry.rs::handle \"ledger commit\" as of 2026-01-01",
        &["conflicts".into(), "telepathy".into()],
        EvidenceDepth::Support,
    );
    assert!(frame.handles.tickets.contains("PAY-77"));
    assert!(frame.handles.paths.contains("src/pay/retry.rs"));
    assert!(frame.handles.symbols.contains("src/pay/retry.rs::handle"));
    assert!(frame.handles.quoted.contains("ledger commit"));
    assert!(
        frame.handles.time_cues.contains("as of") && frame.handles.time_cues.contains("2026-01-01")
    );
    assert!(
        frame.needs.contains(&Need::Conflicts) && frame.needs.contains(&Need::CurrentConstraints)
    );
    assert_eq!(
        frame.unresolved,
        vec!["unknown need `telepathy`".to_string()],
        "unknown intent is reported, not guessed"
    );
}

#[test]
fn every_legacy_tool_maps_to_an_operation_or_is_explicit() {
    for (legacy, op) in [
        ("cortex_boot", Operation::Orient),
        ("cortex_recall", Operation::Query),
        ("cortex_store", Operation::Commit),
        ("cortex_unfold", Operation::Expand),
        ("cortex_conflicts_resolve", Operation::Resolve),
        ("cortex_agent_feedback_record", Operation::Feedback),
        ("cortex_health", Operation::Capabilities),
        ("cortex_focus_end", Operation::Checkpoint),
    ] {
        assert_eq!(Operation::from_tool_name(legacy), Some(op), "{legacy}");
    }
    assert_eq!(Operation::from_tool_name("cortex_permissions_list"), None);
}

#[test]
fn commit_orient_expand_roundtrip_with_receipt_scoped_aliases() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let committed = dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"local_id": "rule", "text": "ROUND-1 never retry a payment after the ledger commit succeeded", "kind": "decision"}], "idempotency_key": "ops/round-1"})).await.unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
        assert!(
            committed["receipt"]["durability"]["local_commit"].is_object(),
            "{committed}"
        );
        assert!(
            committed["receipt"]["entries"]
                .get("rule.decision")
                .is_some(),
            "client-local names map to canonical ids: {committed}"
        );
        let replay = dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"local_id": "rule", "text": "ROUND-1 never retry a payment after the ledger commit succeeded", "kind": "decision"}], "idempotency_key": "ops/round-1"})).await.unwrap();
        assert_eq!(
            replay["receipt"]["durability"], committed["receipt"]["durability"],
            "replay returns the original receipt"
        );
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Orient,
            &json!({"task": "ROUND-1 retry policy", "budget": 2000}),
        )
        .await
        .unwrap();
        assert_eq!(view["profile"], "orient");
        assert!(
            matches!(view["status"].as_str(), Some("ok") | Some("partial")),
            "{view}"
        );
        let card = &view["cards"][0];
        assert_eq!(card["alias"], "m1");
        assert_eq!(card["epistemic"], "asserted");
        assert_eq!(card["applicability"], "applicable");
        assert!(card["statement"].as_str().unwrap().contains("ROUND-1"));
        assert!(
            view["coverage"]["unmet"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "open_obligations"),
            "unmet needs are named: {view}"
        );
        assert!(
            view["coverage"]["unmet"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "current_constraints"),
            "a plain decision does not satisfy current_constraints: {view}"
        );
        assert!(
            view["brief"]
                .as_str()
                .unwrap()
                .starts_with("Constraint c1:")
                || view["brief"].as_str().unwrap().starts_with("Known k1:"),
            "{view}"
        );
        let receipt = view["receipt"].as_str().unwrap().to_string();
        let expanded = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"alias": "m1", "receipt": receipt}),
        )
        .await
        .unwrap();
        assert_eq!(expanded["status"], "ok", "{expanded}");
        assert!(expanded["revision_body"]["text"]
            .as_str()
            .unwrap()
            .contains("ROUND-1"));
        let unscoped = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"alias": "m1"}),
        )
        .await
        .unwrap();
        assert_eq!(
            unscoped["status"], "invalid_request",
            "unqualified alias is rejected, not guessed: {unscoped}"
        );
        let wrong = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"alias": "m1", "receipt": "view-0-deadbeef"}),
        )
        .await
        .unwrap();
        assert_eq!(wrong["status"], "no_match");
        let other = dispatch(
            &cx,
            &state,
            Caller {
                owner_id: None,
                agent: "other",
                principal: "user:9".into(),
            },
            Operation::Expand,
            &json!({"alias": "m1", "receipt": &view["receipt"]}),
        )
        .await
        .unwrap();
        assert_eq!(other["status"], "no_match", "aliases are principal-scoped");
    });
}

#[test]
fn needs_more_budget_is_explicit_and_never_truncates_a_card() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(&cx, &state, caller(), Operation::Commit, &json!({"decision": "BUDGET-1 a fairly long constraint about retries that must not be cut in the middle of its condition clause"})).await.unwrap();
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BUDGET-1", "profile": "map", "budget": 20}),
        )
        .await
        .unwrap();
        assert_eq!(view["status"], "needs_more_budget", "{view}");
        assert!(
            view["cards"].as_array().unwrap().is_empty(),
            "no truncated card is served"
        );
        let required = view["required_plan_bytes"].as_u64().unwrap();
        assert!(required > 20);
        let ok = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BUDGET-1", "profile": "map", "budget": required}),
        )
        .await
        .unwrap();
        assert_eq!(ok["status"], "ok", "the stated plan size fits: {ok}");
        let miss = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "ZZZ-nothing-here-999", "profile": "answer"}),
        )
        .await
        .unwrap();
        assert_eq!(miss["status"], "no_match");
        assert_eq!(
            miss["coverage"]["partitions"]["cold"]["searched"], false,
            "a negative answer names what was not searched: {miss}"
        );
    });
}

#[test]
fn checkpoint_and_resolve_create_revisions() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let cp = dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "payments retry", "goal": "restore without duplicate charges", "state": {"open": ["exercise timeout-after-commit"]}})).await.unwrap();
        assert_eq!(cp["status"], "ok", "{cp}");
        assert_eq!(cp["thread"], "thread:payments-retry");
        let cp2 = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Checkpoint,
            &json!({"thread": "payments retry", "goal": "restore", "state": {"open": []}}),
        )
        .await
        .unwrap();
        assert_ne!(
            cp2["checkpoint"], cp["checkpoint"],
            "each checkpoint is a new revision"
        );
        let conn = state.db.lock(&cx).await.expect("lock");
        let heads =
            cortex_kernel::db::records::heads(&conn, "checkpoint:thread:payments-retry").unwrap();
        assert_eq!(heads.len(), 1);
        drop(conn);
        let resolved = dispatch(&cx, &state, caller(), Operation::Resolve, &json!({"record": "checkpoint:thread:payments-retry", "rationale": "owner accepted", "body": {"final": true}})).await.unwrap();
        assert_eq!(resolved["status"], "ok", "{resolved}");
        let missing = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Resolve,
            &json!({"record": "claim:none", "rationale": "x"}),
        )
        .await
        .unwrap();
        assert_eq!(missing["status"], "no_match");
    });
}

#[test]
fn mcp_advertises_eight_tools_and_legacy_store_is_real() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let list = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            None,
            None,
        )
        .await
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(names.len(), 8, "{names:?}");
        for op in [
            "cortex_capabilities",
            "cortex_orient",
            "cortex_query",
            "cortex_expand",
            "cortex_commit",
            "cortex_checkpoint",
            "cortex_resolve",
            "cortex_feedback",
        ] {
            assert!(names.contains(&op), "{op} missing from {names:?}");
        }
        let store = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"cortex_store","arguments":{"decision":"MCP-1 legacy store now really stores"}}}), None, None).await.unwrap();
        let payload = tool_text(&store);
        assert_eq!(
            payload["status"], "ok",
            "legacy cortex_store must not be a stub: {}",
            store
        );
        assert!(
            payload["receipt"]["durability"]["local_commit"].is_object(),
            "{payload}"
        );
        let query = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"cortex_query","arguments":{"need":"MCP-1","profile":"answer"}}}), None, None).await.unwrap();
        let view = tool_text(&query);
        assert_eq!(view["cards"][0]["alias"], "m1", "{view}");
        let stub = handle_mcp_message_with_caller(&cx, &state, &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"cortex_forget","arguments":{}}}), None, None).await.unwrap();
        let body_text = stub.to_string();
        assert!(
            !body_text.contains("\"ok\":true"),
            "an unimplemented legacy tool must not fabricate success: {}",
            stub
        );
        // Dead names must be unknown, not "recognised but unimplemented".
        assert!(
            stub.to_string().contains("UNKNOWN_TOOL")
                || stub["error"]["code"] == json!(-32601)
                || stub["result"]["isError"] == json!(true),
            "removed legacy tools must fail closed: {stub}"
        );
    });
}

#[test]
fn mcp_cortex_boot_routes_to_orient_and_dead_names_are_unknown() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let boot = handle_mcp_message_with_caller(
            &cx,
            &state,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"cortex_boot","arguments":{"task":"P1 boot alias","budget":2000}}}),
            None,
            None,
        )
        .await
        .unwrap();
        let body = tool_text(&boot);
        assert!(
            !body.get("error").is_some() && boot["result"]["isError"] != json!(true),
            "cortex_boot must not be a dead end: {boot}"
        );
        assert_eq!(
            body["profile"], "orient",
            "cortex_boot is the legacy name for orient: {body}"
        );
        for dead in [
            "cortex_diary",
            "cortex_reconnect",
            "cortex_boot_audit",
            "cortex_recall_policy_explain",
            "cortex_memory_decay_run",
        ] {
            let response = handle_mcp_message_with_caller(
                &cx,
                &state,
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":dead,"arguments":{}}}),
                None,
                None,
            )
            .await
            .unwrap();
            let text = response.to_string();
            assert!(
                text.contains("UNKNOWN_TOOL") || text.contains("Unknown tool"),
                "{dead} must be unknown, not a half-advertised route: {response}"
            );
            assert!(!text.contains("\"ok\":true"), "{dead} fabricated success: {response}");
        }
    });
}

#[test]
fn evidence_closure_keeps_qualifiers_and_renders_contradictions_as_contrast() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::db::records::{append_commit, heads};
        use cortex_kernel::handlers::operations::{add_relation, DependencyRole};
        let state = solo_state();
        let rule = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CLOSE-1 retry failed requests up to three times"}),
        )
        .await
        .unwrap();
        let exception = dispatch(&cx, &state,
caller(),
Operation::Commit,
&json!({"decision": "CLOSE-1 exception: never retry once the ledger commit has succeeded"}),)
.await
.unwrap();
        let rule_id = rule["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        let exception_id = exception["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            let rule_head = heads(&conn, &format!("decision:{rule_id}"))
                .unwrap()
                .remove(0);
            let exc_head = heads(&conn, &format!("decision:{exception_id}"))
                .unwrap()
                .remove(0);
            let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
            add_relation(
                &conn,
                seq,
                &rule_head,
                &exc_head,
                DependencyRole::RequiredQualifier.as_str(),
                Some("closure/1"),
            )
            .unwrap();
            let unknown = add_relation(&conn, seq, &rule_head, &exc_head, "vibes", None);
            assert!(
                unknown.unwrap_err().contains("unknown dependency role"),
                "unknown roles are never optional"
            );
        }
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CLOSE-1 retry", "profile": "answer", "budget": 4000}),
        )
        .await
        .unwrap();
        let rule_card = view["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["statement"].as_str().unwrap().contains("three times"))
            .unwrap_or_else(|| panic!("{view}"));
        let exceptions = rule_card["exceptions"].as_array().unwrap();
        assert!(
            exceptions.iter().any(|e| e
                .as_str()
                .unwrap()
                .contains("never retry once the ledger commit")),
            "the qualifier travels with the claim: {view}"
        );
        // Budget too small for statement + required exception: never served without it.
        let tight = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CLOSE-1 retry", "profile": "answer", "budget": 60}),
        )
        .await
        .unwrap();
        assert_eq!(tight["status"], "needs_more_budget", "{tight}");
        // A CONTRADICTS conflict makes the Card a contested contrast Card.
        let a = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CONTRA-9 deployment default is blue"}),
        )
        .await
        .unwrap();
        let b = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "CONTRA-9 deployment default is green"}),
        )
        .await
        .unwrap();
        let a_id = a["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        let b_id = b["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse::<i64>()
            .unwrap();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            conn.execute("INSERT INTO decision_conflicts (source_decision_id, target_decision_id, classification, similarity_jaccard, status) VALUES (?1, ?2, 'CONTRADICTS', 0.8, 'open')", [a_id, b_id]).unwrap();
        }
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "CONTRA-9 deployment default", "profile": "conflicts", "budget": 4000}),
        )
        .await
        .unwrap();
        let contested: Vec<&Value> = view["cards"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["epistemic"] == "contested")
            .collect();
        assert!(
            !contested.is_empty(),
            "open contradiction must render as contested: {view}"
        );
        assert!(
            contested[0]["exceptions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e.as_str().unwrap().starts_with("contrary head")),
            "{view}"
        );
        assert!(
            view["brief"].as_str().unwrap().contains("Conflict x1:"),
            "{view}"
        );
    });
}

/// Evidence-closure quality bar (V4 docs/05). A View never serves a claim
/// without the material exception that changes its meaning, and never
/// presents an open contrary head as asserted.
#[test]
fn evidence_closure_quality_bar_never_serves_a_naked_claim() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::db::records::{append_commit, heads};
        use cortex_kernel::handlers::operations::{add_relation, DependencyRole};
        let state = solo_state();

        // Rule + exception linked as RequiredQualifier.
        let rule = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "BAR-1 always enable TLS on the payments listener"}),
        )
        .await
        .unwrap();
        let exception = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "BAR-1 exception: localhost dev listener may stay plaintext"}),
        )
        .await
        .unwrap();
        let rule_id: i64 = rule["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let exception_id: i64 = exception["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            let rule_head = heads(&conn, &format!("decision:{rule_id}")).unwrap().remove(0);
            let exc_head = heads(&conn, &format!("decision:{exception_id}")).unwrap().remove(0);
            let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
            add_relation(
                &conn,
                seq,
                &rule_head,
                &exc_head,
                DependencyRole::RequiredQualifier.as_str(),
                Some("closure/1"),
            )
            .unwrap();
        }

        let full = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BAR-1 TLS payments listener", "profile": "answer", "budget": 8000}),
        )
        .await
        .unwrap();
        let card = full["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["statement"].as_str().unwrap().contains("always enable TLS"))
            .unwrap_or_else(|| panic!("rule card missing: {full}"));
        assert_eq!(card["epistemic"], "asserted");
        assert!(
            card["exceptions"].as_array().unwrap().iter().any(|e| e
                .as_str()
                .unwrap()
                .contains("localhost dev listener")),
            "required qualifier must travel with the claim: {full}"
        );
        // Brief is the human channel: exception is visible there too.
        assert!(
            full["brief"].as_str().unwrap().contains("localhost dev listener")
                || full["brief"].as_str().unwrap().contains("Boundary"),
            "brief must not hide the exception: {}",
            full["brief"]
        );

        // Budget that cannot hold statement+exception: no naked headline.
        let tight = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BAR-1 TLS payments listener", "profile": "answer", "budget": 40}),
        )
        .await
        .unwrap();
        assert_eq!(
            tight["status"], "needs_more_budget",
            "required bundle must not shrink below statement+exceptions: {tight}"
        );
        let cards = tight["cards"].as_array().cloned().unwrap_or_default();
        assert!(
            cards.iter().all(|c| {
                c["exceptions"]
                    .as_array()
                    .map(|e| !e.is_empty())
                    .unwrap_or(false)
                    || !c["statement"].as_str().unwrap().contains("always enable TLS")
            }),
            "no naked BAR-1 headline under insufficient budget: {tight}"
        );

        // Counterevidence is material: it rides the claim as contested-ish exception.
        let counter = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "BAR-1 counterevidence: staging TLS listener failed open last week"}),
        )
        .await
        .unwrap();
        let counter_id: i64 = counter["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            let rule_head = heads(&conn, &format!("decision:{rule_id}")).unwrap().remove(0);
            let c_head = heads(&conn, &format!("decision:{counter_id}")).unwrap().remove(0);
            let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
            add_relation(
                &conn,
                seq,
                &rule_head,
                &c_head,
                DependencyRole::Counterevidence.as_str(),
                Some("closure/1"),
            )
            .unwrap();
        }
        let with_counter = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BAR-1 TLS payments listener", "profile": "answer", "budget": 8000}),
        )
        .await
        .unwrap();
        let card = with_counter["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["statement"].as_str().unwrap().contains("always enable TLS"))
            .unwrap_or_else(|| panic!("{with_counter}"));
        assert!(
            card["exceptions"].as_array().unwrap().iter().any(|e| e
                .as_str()
                .unwrap()
                .contains("counterevidence")),
            "counterevidence must travel: {with_counter}"
        );

        // Open contrary head is never asserted.
        let other = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "BAR-2 auth service must rotate keys weekly"}),
        )
        .await
        .unwrap();
        let rival = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "BAR-2 auth service must rotate keys monthly"}),
        )
        .await
        .unwrap();
        let o_id: i64 = other["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let r_id: i64 = rival["receipt"]["entries"]["decision.decision"]["value"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            conn.execute(
                "INSERT INTO decision_conflicts (source_decision_id, target_decision_id, classification, similarity_jaccard, status) VALUES (?1, ?2, 'CONTRADICTS', 0.9, 'open')",
                [o_id, r_id],
            )
            .unwrap();
        }
        let conflict_view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BAR-2 auth service rotate keys", "profile": "answer", "budget": 8000}),
        )
        .await
        .unwrap();
        for card in conflict_view["cards"].as_array().unwrap() {
            let text = card["statement"].as_str().unwrap();
            if text.contains("weekly") || text.contains("monthly") {
                assert_eq!(
                    card["epistemic"], "contested",
                    "open CONTRADICTS head must not be asserted: {conflict_view}"
                );
                assert!(
                    card["exceptions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|e| e.as_str().unwrap().contains("contrary head")),
                    "contrary side must be attached: {conflict_view}"
                );
            }
        }

        // Omissions never mark a required bundle as a silent optional drop.
        let mid = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "BAR-1 TLS payments listener", "profile": "answer", "budget": 200}),
        )
        .await
        .unwrap();
        if let Some(omissions) = mid["coverage"]["omissions"].as_array() {
            for o in omissions {
                if o["required"] == json!(true) {
                    assert_eq!(
                        mid["status"], "needs_more_budget",
                        "required omission must escalate, not silently drop: {mid}"
                    );
                }
            }
        }
    });
}

#[test]
fn leads_are_separate_from_cards_and_absent_in_high_assurance_profiles() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "LEAD-1 billing invoices use Stripe Billing API in payments"}),
        )
        .await
        .unwrap();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "design review used stripe patterns in the UI kit"}),
        )
        .await
        .unwrap();
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "Stripe Billing API", "profile": "map", "budget": 4000}),
        )
        .await
        .unwrap();
        let cards: Vec<&str> = view["cards"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["statement"].as_str())
            .collect();
        assert!(cards.iter().any(|c| c.contains("LEAD-1")), "{view}");
        assert!(
            !cards.iter().any(|c| c.contains("UI kit")),
            "weak neighbor is not a Card: {view}"
        );
        for lead in view["leads"].as_array().unwrap() {
            assert_eq!(
                lead["supported"], false,
                "leads are labeled unsupported: {lead}"
            );
        }
        let strict = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "Stripe Billing API", "profile": "answer", "budget": 4000}),
        )
        .await
        .unwrap();
        assert!(
            strict["leads"].as_array().unwrap().is_empty(),
            "high-assurance profiles omit leads: {strict}"
        );
    });
}

#[test]
fn budgeted_selection_keeps_required_bundles_and_omits_optional_ones_explicitly() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"local_id": "c", "kind": "constraint", "text": "SEL-1 constraint: preserve the idempotency contract"}]})).await.unwrap();
        for text in [
            "SEL-1 rollout logs show the idempotency contract header in staging",
            "SEL-1 the payments gateway forwards idempotency contract keys unchanged",
            "SEL-1 dashboards chart idempotency contract retries per minute",
            "SEL-1 QA replayed the idempotency contract suite on Friday",
        ] {
            dispatch(
                &cx,
                &state,
                caller(),
                Operation::Commit,
                &json!({"decision": text}),
            )
            .await
            .unwrap();
        }
        let full = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "SEL-1 idempotency contract", "profile": "answer", "budget": 4000}),
        )
        .await
        .unwrap();
        assert_eq!(full["status"], "ok", "{full}");
        let constraint = full["cards"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["required"] == true)
            .unwrap_or_else(|| panic!("{full}"));
        assert_eq!(
            constraint["label"], "c1",
            "durable constraint is labeled as a constraint: {full}"
        );
        assert!(
            full["brief"].as_str().unwrap().contains("Constraint c1:"),
            "{full}"
        );
        let labels = full["labels"].as_array().unwrap();
        assert!(
            labels
                .iter()
                .any(|l| l["label"] == "c1" && l["alias"].as_str().unwrap().starts_with('m')),
            "labels map to aliases: {full}"
        );
        let constraint_cost = constraint["sidecar"]["bytes"].as_u64().unwrap() as usize + 6;
        let tight = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "SEL-1 idempotency contract", "profile": "answer", "budget": constraint_cost + 10})).await.unwrap();
        assert_eq!(
            tight["status"], "ok",
            "required bundle fits, optional ones are dropped: {tight}"
        );
        assert_eq!(tight["cards"].as_array().unwrap().len(), 1, "{tight}");
        assert_eq!(tight["cards"][0]["required"], true, "{tight}");
        assert!(
            !tight["coverage"]["omissions"]
                .as_array()
                .unwrap()
                .is_empty(),
            "dropped optional cards are listed, not silent: {tight}"
        );
        assert_eq!(tight["coverage"]["omissions"][0]["reason"], "budget");
        let too_small = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "SEL-1 idempotency contract", "profile": "answer", "budget": 8}),
        )
        .await
        .unwrap();
        assert_eq!(too_small["status"], "needs_more_budget", "{too_small}");
        assert!(too_small["required_plan_bytes"].as_u64().unwrap() >= constraint_cost as u64 - 6);
        let partitions = &full["coverage"]["partitions"];
        for part in ["decisions", "memories"] {
            for key in [
                "source_frontier",
                "index_frontier",
                "searched_range",
                "limits_hit",
                "archive_policy",
                "exhausted",
            ] {
                assert!(
                    partitions[part].get(key).is_some(),
                    "watermark {part}.{key} missing: {partitions}"
                );
            }
        }
        assert_eq!(partitions["cold"]["searched"], false);
        let exact = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "SEL-1 idempotency contract", "profile": "answer", "budget": 8000, "evidence": "exact"})).await.unwrap();
        assert!(
            exact["cards"][0]["exact"].as_str().is_some(),
            "exact rendering carries the captured source: {exact}"
        );
        assert!(
            exact["budget"]["used_bytes"].as_u64().unwrap()
                >= full["budget"]["used_bytes"].as_u64().unwrap(),
            "exact is measured, not assumed free"
        );
    });
}

/// Adapter equality: the same fixture through the library, the in-process
/// CLI and MCP must agree on eligible Cards (statement,
/// HTTP adapter/status claims are retired with the HTTP surface.
/// epistemic state, applicability, exceptions, labels). Receipt ids and
/// frontiers may differ; nothing else may.
#[test]
fn package_cli_and_mcp_return_the_same_qualified_bundle() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let temp = tempfile::tempdir().unwrap();
        let home_dir = temp.path();
        let home = home_dir.to_string_lossy().to_string();
        let db = home_dir.join("cortex.db").to_string_lossy().to_string();
        // Seed through the in-process CLI (no daemon running yet).
        let run_cli = |op: &str, args: &str| -> Value {
            let out = std::process::Command::new(cortex_tests::cortex_bin())
                .args(["op", op, "--args", args, "--home", &home, "--db", &db])
                .env("CORTEX_DISABLE_IPC", "1")
                .output()
                .expect("cortex op");
            let text = String::from_utf8_lossy(&out.stdout);
            serde_json::from_str(text.trim().lines().last().unwrap_or("{}")).unwrap_or_else(|e| {
                panic!(
                    "cli json {e}: {text} / {}",
                    String::from_utf8_lossy(&out.stderr)
                )
            })
        };
        let committed = run_cli(
            "commit",
            r#"{"entries":[{"local_id":"c","kind":"constraint","text":"PARITY-1 constraint: keep the idempotency contract"},{"local_id":"e","text":"PARITY-1 exception: not for non-idempotent endpoints"}]}"#,
        );
        assert_eq!(committed["status"], "ok", "{committed}");
        let cli_view = run_cli(
            "query",
            r#"{"need":"PARITY-1 idempotency","profile":"answer","budget":4000}"#,
        );
        fn shape(view: &Value) -> Vec<Value> {
            view["cards"].as_array().unwrap().iter().map(|c| json!({"statement": c["statement"], "epistemic": c["epistemic"], "applicability": c["applicability"], "exceptions": c["exceptions"], "label": c["label"], "required": c["required"], "reference": c["sidecar"]["reference"]})).collect()
        }
        let expected = shape(&cli_view);
        assert_eq!(expected.len(), 2, "{cli_view}");
        let runtime =
            cortex_kernel::runtime::CortexRuntime::open_db(std::path::Path::new(&db)).unwrap();
        let local = dispatch(
            &cx,
            runtime.state(),
            caller(),
            Operation::Query,
            &json!({"need":"PARITY-1 idempotency","profile":"answer","budget":4000}),
        )
        .await
        .unwrap();
        assert_eq!(shape(&local), expected, "library differs from CLI: {local}");
        let mcp = handle_mcp_message_with_caller(&cx, runtime.state(), &json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"cortex_query","arguments":{"need":"PARITY-1 idempotency","profile":"answer","budget":4000}}}), None, None).await.unwrap();
        let mcp_view = tool_text(&mcp);
        assert_eq!(
            shape(&mcp_view),
            expected,
            "MCP differs from CLI: {mcp_view}"
        );
        let bad = std::process::Command::new(cortex_tests::cortex_bin())
            .args([
                "op",
                "summarize",
                "--args",
                "{}",
                "--home",
                &home,
                "--db",
                &db,
            ])
            .output()
            .expect("unknown CLI operation");
        assert_eq!(bad.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&bad.stderr).contains("unknown operation"));
    });
}

#[test]
fn commit_return_view_states_its_frontier_honestly() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let out = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "RV-1 return view at frontier", "return_view": true}),
        )
        .await
        .unwrap();
        assert_eq!(out["status"], "ok", "{out}");
        assert_eq!(
            out["view"]["at_commit_frontier"], true,
            "no concurrent change: the View sits at the commit frontier: {out}"
        );
        assert!(out["view"]["cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["statement"].as_str().unwrap().contains("RV-1")));
    });
}

#[test]
fn aliases_from_a_previous_restore_epoch_are_rejected() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": "EPOCH-1 alias epoch test"}),
        )
        .await
        .unwrap();
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "EPOCH-1", "profile": "map"}),
        )
        .await
        .unwrap();
        let receipt = view["receipt"].as_str().unwrap().to_string();
        {
            let conn = state.db.lock(&cx).await.expect("lock");
            conn.execute(
                "UPDATE brain_meta SET restore_epoch = 'restore-next' WHERE singleton = 1",
                [],
            )
            .unwrap();
        }
        let expanded = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"alias": "m1", "receipt": receipt}),
        )
        .await
        .unwrap();
        assert_eq!(expanded["status"], "resnapshot_required", "{expanded}");
    });
}

/// V5 unification: query/orient surface attributed observations in a separate
/// section (never mixed into Cards); expand hydrates obs:<source_id>.
#[test]
fn query_and_expand_expose_v5_observations_as_attributed_evidence() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("worklog", "project"))
            .await
            .unwrap();
        let receipt = runtime
            .observe(
                &cx,
                "worklog",
                "g1",
                ObservationEvent {
                    event_key: "run-1".into(),
                    text: "PAY-12 ledger write uses idempotency key; observed tool report".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();

        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "PAY-12 idempotency ledger", "profile": "answer", "budget": 4000, "observation_scope": "project"}),
        )
        .await
        .unwrap();
        let observations = view
            .get("observations")
            .unwrap_or_else(|| panic!("query must surface observations section: {view}"));
        assert_eq!(observations["scope"], "project");
        let items = observations["items"].as_array().expect("items");
        assert!(
            items.iter().any(|i| i["source_id"] == json!(receipt.source_id)),
            "observation hit missing: {observations}"
        );
        let hit = items
            .iter()
            .find(|i| i["source_id"] == json!(receipt.source_id))
            .unwrap();
        assert_eq!(hit["expand"], json!(format!("obs:{}", receipt.source_id)));
        assert_eq!(hit["trust"]["kind"], "attributed_observation");
        assert_eq!(hit["trust"]["instruction"], false);
        // Never a Card: capture is not factual endorsement.
        let cards = view["cards"].as_array().cloned().unwrap_or_default();
        assert!(
            !cards.iter().any(|c| c["statement"]
                .as_str()
                .map(|s| s.contains("observed tool report"))
                .unwrap_or(false)),
            "observation text must not become an asserted Card: {view}"
        );

        let expanded = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"reference": format!("obs:{}", receipt.source_id)}),
        )
        .await
        .unwrap();
        assert_eq!(expanded["status"], "ok", "{expanded}");
        assert_eq!(expanded["representation"], "exact");
        assert_eq!(expanded["source"]["text"], "PAY-12 ledger write uses idempotency key; observed tool report");
        assert_eq!(expanded["source"]["role"], "document");
        assert_eq!(expanded["source"]["trust"]["kind"], "attributed_observation");

        // Opt-out stays clean.
        let quiet = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Orient,
            &json!({"task": "PAY-12", "budget": 2000, "observations": false}),
        )
        .await
        .unwrap();
        assert!(
            quiet.get("observations").is_none(),
            "observations=false must omit the section: {quiet}"
        );
    });
}

/// Governed promote: an explicit commit may cite V5 observations as evidence.
/// Capture is not endorsement; promotion is a Deposit the agent chose.
#[test]
fn commit_can_cite_observation_evidence_without_auto_promotion() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("worklog", "project"))
            .await
            .unwrap();
        let obs = runtime
            .observe(
                &cx,
                "worklog",
                "g1",
                ObservationEvent {
                    event_key: "run-9".into(),
                    text: "tool reported PAY-12 ledger write used idempotency key key-77".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();

        // Unknown / unauthorized evidence fails closed — no silent drop.
        let bad = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": "PAY-12 retries must reuse the original idempotency key",
                "evidence": ["obs:does-not-exist"]
            }),
        )
        .await
        .unwrap();
        assert_eq!(bad["status"], "invalid_request", "{bad}");

        let committed = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": "PAY-12 retries must reuse the original idempotency key",
                "evidence": [format!("obs:{}", obs.source_id)]
            }),
        )
        .await
        .unwrap();
        assert_eq!(committed["status"], "ok", "{committed}");
        let evidence = committed["evidence"]["linked"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            evidence.iter().any(|e| e["source_id"] == json!(obs.source_id)),
            "commit must link the cited observation: {committed}"
        );
        let hit = evidence
            .iter()
            .find(|e| e["source_id"] == json!(obs.source_id))
            .unwrap();
        assert_eq!(hit["relationship"], "promoted_from");
        assert_eq!(hit["role"], "document");

        // Expand of the decision surfaces the same exact observation.
        let decision_id = committed["legacy_entries"][0]["id"]
            .as_i64()
            .expect("decision id");
        let expanded = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"reference": format!("decision::{decision_id}")}),
        )
        .await
        .unwrap();
        let linked = expanded["evidence"]["observations"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            linked.iter().any(|e| e["source_id"] == json!(obs.source_id)),
            "expand must carry promoted evidence: {expanded}"
        );
        let hit = linked
            .iter()
            .find(|e| e["source_id"] == json!(obs.source_id))
            .unwrap();
        assert_eq!(hit["text"], "tool reported PAY-12 ledger write used idempotency key key-77");
        assert_eq!(hit["trust"]["kind"], "attributed_observation");

        // Observations alone never create a decision.
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "PAY-12 idempotency key-77", "profile": "answer", "budget": 4000}),
        )
        .await
        .unwrap();
        let cards = view["cards"].as_array().cloned().unwrap_or_default();
        assert!(
            cards
                .iter()
                .any(|c| c["statement"].as_str().map(|s| s.contains("retries must reuse")).unwrap_or(false)),
            "promoted decision is a Card: {view}"
        );
        assert!(
            view.get("observations").is_some(),
            "source observation remains available as evidence"
        );
    });
}

const OBS_REPO_A: &str = "/Users/x/repoa";
const OBS_REPO_B: &str = "/Users/x/repob";

fn observation_source_ids(view: &Value) -> Vec<String> {
    view["observations"]["items"]
        .as_array()
        .unwrap_or_else(|| panic!("observations.items missing: {view}"))
        .iter()
        .filter_map(|item| item["source_id"].as_str().map(str::to_string))
        .collect()
}

/// Path-scoped observation sources stay in that repository on query/orient.
/// The unscoped `project` bucket stays visible. Capture still does not
/// become a Card.
#[test]
fn query_and_orient_keep_path_scoped_observations_in_their_repository() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("notes-a", OBS_REPO_A))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes-b", OBS_REPO_B))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("worklog", "project"))
            .await
            .unwrap();
        let in_a = runtime
            .observe(
                &cx,
                "notes-a",
                "g1",
                ObservationEvent {
                    event_key: "a1".into(),
                    text: "PAY-OBS-1 tool reported ledger retry after crash".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let in_b = runtime
            .observe(
                &cx,
                "notes-b",
                "g1",
                ObservationEvent {
                    event_key: "b1".into(),
                    text: "PAY-OBS-1 tool reported cache warm on boot".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let unscoped = runtime
            .observe(
                &cx,
                "worklog",
                "g1",
                ObservationEvent {
                    event_key: "u1".into(),
                    text: "PAY-OBS-1 tool reported signing key rotation".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();

        let default_project = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({
                "need": "PAY-OBS-1 tool reported",
                "profile": "answer",
                "budget": 4000,
                "observation_scope": "project"
            }),
        )
        .await
        .unwrap();
        let default_ids = observation_source_ids(&default_project);
        assert!(
            default_ids.contains(&unscoped.source_id),
            "unscoped project observation must remain on the default bucket: {default_project}"
        );
        assert!(
            !default_ids.contains(&in_a.source_id),
            "a path-scoped observation must not leak into the default project pull: {default_project}"
        );
        assert!(
            !default_ids.contains(&in_b.source_id),
            "a sibling path-scoped observation must not leak into the default project pull: {default_project}"
        );

        let in_repo_a = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({
                "need": "PAY-OBS-1 tool reported",
                "profile": "answer",
                "budget": 4000,
                "paths": [OBS_REPO_A]
            }),
        )
        .await
        .unwrap();
        let ids_a = observation_source_ids(&in_repo_a);
        assert!(
            ids_a.contains(&in_a.source_id),
            "query with this repo path must return its observation: {in_repo_a}"
        );
        assert!(
            ids_a.contains(&unscoped.source_id),
            "unscoped project observations stay visible under a project path: {in_repo_a}"
        );
        assert!(
            !ids_a.contains(&in_b.source_id),
            "query with this repo path must not return a sibling repo observation: {in_repo_a}"
        );
        let cards = in_repo_a["cards"].as_array().cloned().unwrap_or_default();
        assert!(
            !cards.iter().any(|c| c["statement"]
                .as_str()
                .map(|s| s.contains("tool reported ledger retry"))
                .unwrap_or(false)),
            "path-scoped observation text must not become a Card: {in_repo_a}"
        );

        let in_repo_b = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({
                "need": "PAY-OBS-1 tool reported",
                "profile": "answer",
                "budget": 4000,
                "paths": [OBS_REPO_B]
            }),
        )
        .await
        .unwrap();
        let ids_b = observation_source_ids(&in_repo_b);
        assert!(
            ids_b.contains(&in_b.source_id),
            "query with the sibling repo must return its observation: {in_repo_b}"
        );
        assert!(
            !ids_b.contains(&in_a.source_id),
            "sibling repo query must not return this repo's observation: {in_repo_b}"
        );

        let nested = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({
                "need": "PAY-OBS-1 tool reported",
                "profile": "answer",
                "budget": 4000,
                "paths": [format!("{OBS_REPO_A}/src")]
            }),
        )
        .await
        .unwrap();
        let nested_ids = observation_source_ids(&nested);
        assert!(
            nested_ids.contains(&in_a.source_id),
            "a path under the registered root must still retrieve that observation: {nested}"
        );
        assert!(
            !nested_ids.contains(&in_b.source_id),
            "a nested path must not retrieve a sibling repo observation: {nested}"
        );

        let orient = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Orient,
            &json!({"paths": [OBS_REPO_A], "budget": 4000}),
        )
        .await
        .unwrap();
        let orient_ids = observation_source_ids(&orient);
        assert!(
            orient_ids.contains(&in_a.source_id),
            "cwd-only orient must surface this repo's observations: {orient}"
        );
        assert!(
            !orient_ids.contains(&in_b.source_id),
            "cwd-only orient must not surface a sibling repo observation: {orient}"
        );
    });
}

#[derive(Clone, Copy)]
struct CiteCase {
    source_key: &'static str,
    scope: &'static str,
    cwd: Option<&'static str>,
    expect_ok: bool,
}

#[test]
fn commit_observation_cites_stay_in_path_scope() {
    for case in [
        CiteCase {
            source_key: "notes-b",
            scope: OBS_REPO_B,
            cwd: Some(OBS_REPO_A),
            expect_ok: false,
        },
        CiteCase {
            source_key: "notes-a",
            scope: OBS_REPO_A,
            cwd: None,
            expect_ok: false,
        },
        CiteCase {
            source_key: "notes-a",
            scope: OBS_REPO_A,
            cwd: Some(OBS_REPO_A),
            expect_ok: true,
        },
        CiteCase {
            source_key: "worklog",
            scope: "project",
            cwd: Some(OBS_REPO_A),
            expect_ok: true,
        },
        CiteCase {
            source_key: "vault",
            scope: "secrets",
            cwd: Some(OBS_REPO_A),
            expect_ok: false,
        },
        CiteCase {
            source_key: "vault-global",
            scope: "secrets",
            cwd: None,
            expect_ok: false,
        },
        CiteCase {
            source_key: "notes-a",
            scope: OBS_REPO_A,
            cwd: Some("/Users/x/repoa/../repob"),
            expect_ok: false,
        },
        CiteCase {
            source_key: "notes-a",
            scope: OBS_REPO_A,
            cwd: Some("/Users/x/repoa/src/.."),
            expect_ok: true,
        },
    ] {
        cortex_tests::support::run_with_cx(|cx| async move {
            use cortex_kernel::runtime::{
                CortexRuntime,
                observation::{ObservationEvent, SourceSpec},
            };
            let state = solo_state();
            let runtime = CortexRuntime::from_state(state.clone());
            runtime
                .register_source(&cx, SourceSpec::document(case.source_key, case.scope))
                .await
                .unwrap();
            let obs = runtime
                .observe(
                    &cx,
                    case.source_key,
                    "g-cite",
                    ObservationEvent {
                        event_key: "run".into(),
                        text: format!(
                            "tool reported PAY-CITE ledger write used idempotency key {}",
                            case.source_key
                        ),
                        observed_at: None,
                    },
                )
                .await
                .unwrap();
            let mut body = json!({
                "decision": format!(
                    "PAY-CITE retries must reuse the original idempotency key ({})",
                    case.source_key
                ),
                "evidence": [format!("obs:{}", obs.source_id)]
            });
            if let Some(cwd) = case.cwd {
                body["cwd"] = json!(cwd);
            }
            let result = dispatch(&cx, &state, caller(), Operation::Commit, &body)
                .await
                .unwrap();
            if case.expect_ok {
                assert_eq!(result["status"], "ok", "{result}");
                let evidence = result["evidence"]["linked"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                assert!(
                    evidence
                        .iter()
                        .any(|item| item["source_id"] == json!(obs.source_id)),
                    "commit must link the cited observation: {result}"
                );
                return;
            }
            assert_eq!(result["status"], "invalid_request", "{result}");
            assert_eq!(result["field"], "evidence", "{result}");
            let conn = runtime.state().db_read.lock(&cx).await.unwrap();
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(n, 0, "a rejected cite must not leave a deposit: {result}");
        });
    }
}

#[test]
fn commit_cannot_cite_when_roots_span_siblings() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("notes-a", OBS_REPO_A))
            .await
            .unwrap();
        let obs = runtime
            .observe(
                &cx,
                "notes-a",
                "g-cite-union",
                ObservationEvent {
                    event_key: "run-union".into(),
                    text: "tool reported PAY-CITE-UNION ledger write used idempotency key key-u"
                        .into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let stolen = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": "PAY-CITE-UNION retries must reuse the original idempotency key",
                "cwd": OBS_REPO_B,
                "paths": [OBS_REPO_A],
                "evidence": [format!("obs:{}", obs.source_id)]
            }),
        )
        .await
        .unwrap();
        assert_eq!(stolen["status"], "invalid_request", "{stolen}");
        assert_eq!(stolen["field"], "evidence", "{stolen}");
        let conn = runtime.state().db_read.lock(&cx).await.unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            n, 0,
            "unioned sibling roots must not smuggle a cite: {stolen}"
        );
    });
}

fn cite_row_count(conn: &rusqlite::Connection, decision_id: i64, source_id: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM decision_observation_evidence WHERE decision_id = ?1 AND source_id = ?2",
        rusqlite::params![decision_id, source_id],
        |row| row.get(0),
    )
    .unwrap()
}

#[test]
fn commit_cites_attach_to_a_merged_survivor() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("worklog", "project"))
            .await
            .unwrap();
        let obs = runtime
            .observe(
                &cx,
                "worklog",
                "g-merge-cite",
                ObservationEvent {
                    event_key: "run-merge".into(),
                    text: "tool reported PAY-CITE-MERGE ledger write used idempotency key key-m"
                        .into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        const TEXT: &str = "PAY-CITE-MERGE retries must reuse the original idempotency key";
        let first = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({"decision": TEXT}),
        )
        .await
        .unwrap();
        assert_eq!(first["status"], "ok", "{first}");
        let survivor = first["legacy_entries"][0]["id"]
            .as_i64()
            .expect("first decision id");
        let second = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": TEXT,
                "evidence": [format!("obs:{}", obs.source_id)]
            }),
        )
        .await
        .unwrap();
        if second["legacy_entries"][0]["stored"] == false {
            assert_eq!(second["status"], "invalid_request", "{second}");
            assert_eq!(second["field"], "evidence", "{second}");
            assert_eq!(
                cite_row_count(
                    &runtime.state().db_read.lock(&cx).await.unwrap(),
                    survivor,
                    &obs.source_id
                ),
                0,
                "a rejected duplicate must not claim a cite: {second}"
            );
            return;
        }
        assert_eq!(second["status"], "ok", "{second}");
        assert_eq!(
            cite_row_count(
                &runtime.state().db_read.lock(&cx).await.unwrap(),
                survivor,
                &obs.source_id
            ),
            1,
            "a merged survivor must keep the cite: {second}"
        );
    });
}

#[test]
fn commit_cannot_amend_cites_on_idempotent_replay() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("notes-a", "project"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes-b", "project"))
            .await
            .unwrap();
        let obs_a = runtime
            .observe(
                &cx,
                "notes-a",
                "g-idemp",
                ObservationEvent {
                    event_key: "a".into(),
                    text: "tool reported PAY-CITE-IDEMP A ledger write used idempotency key key-ia"
                        .into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let obs_b = runtime
            .observe(
                &cx,
                "notes-b",
                "g-idemp",
                ObservationEvent {
                    event_key: "b".into(),
                    text: "tool reported PAY-CITE-IDEMP B ledger write used idempotency key key-ib"
                        .into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        const TEXT: &str = "PAY-CITE-IDEMP retries must reuse the original idempotency key";
        let first = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": TEXT,
                "evidence": [format!("obs:{}", obs_a.source_id)],
                "idempotency_key": "cite/idemp-1"
            }),
        )
        .await
        .unwrap();
        assert_eq!(first["status"], "ok", "{first}");
        let decision_id = first["legacy_entries"][0]["id"]
            .as_i64()
            .expect("decision id");
        let replay = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": TEXT,
                "evidence": [format!("obs:{}", obs_a.source_id)],
                "idempotency_key": "cite/idemp-1"
            }),
        )
        .await
        .unwrap();
        assert_eq!(replay["status"], "ok", "{replay}");
        let amend = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": TEXT,
                "evidence": [format!("obs:{}", obs_b.source_id)],
                "idempotency_key": "cite/idemp-1"
            }),
        )
        .await
        .unwrap();
        assert_eq!(amend["status"], "invalid_request", "{amend}");
        assert!(
            amend["error"]
                .as_str()
                .unwrap_or_default()
                .starts_with("idempotency_conflict"),
            "changing evidence must be a payload conflict: {amend}"
        );
        let conn = runtime.state().db_read.lock(&cx).await.unwrap();
        assert_eq!(cite_row_count(&conn, decision_id, &obs_a.source_id), 1);
        assert_eq!(cite_row_count(&conn, decision_id, &obs_b.source_id), 0);
    });
}

#[test]
fn commit_cannot_cite_after_policy_epoch_rotation() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::runtime::{
            CortexRuntime,
            observation::{ObservationEvent, SourceSpec},
        };
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("worklog", "project"))
            .await
            .unwrap();
        let obs = runtime
            .observe(
                &cx,
                "worklog",
                "g-stale-policy",
                ObservationEvent {
                    event_key: "run-stale".into(),
                    text: "tool reported PAY-STALE-POLICY ledger write used idempotency key key-sp"
                        .into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            conn.execute(
                "UPDATE brain_meta SET policy_epoch = 'rotated' WHERE singleton = 1",
                [],
            )
            .unwrap();
        }
        let result = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Commit,
            &json!({
                "decision": "PAY-STALE-POLICY retries must reuse the original idempotency key",
                "evidence": [format!("obs:{}", obs.source_id)]
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "invalid_request", "{result}");
        assert_eq!(result["field"], "evidence", "{result}");
        let conn = runtime.state().db_read.lock(&cx).await.unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "a stale-policy cite must not leave a deposit: {result}");
    });
}

#[test]
fn resolve_unknown_keep_id_fails_closed() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let state = solo_state();
        let missing = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Resolve,
            &json!({"keepId": 999_999, "action": "keep"}),
        )
        .await
        .unwrap();
        assert_eq!(missing["status"], "invalid_request", "{missing}");
        assert!(
            missing["error"]
                .as_str()
                .unwrap_or_default()
                .contains("was not found"),
            "{missing}"
        );
        let same = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Resolve,
            &json!({"keepId": 1, "supersededId": 1, "action": "keep"}),
        )
        .await
        .unwrap();
        assert_eq!(same["status"], "invalid_request", "{same}");
    });
}
