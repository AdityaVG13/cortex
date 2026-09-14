//! Recipe laws ported from the design pack's oracle plus the compiled-read
//! invalidation laws: bounded interpreter (limits, cycles, unknown ops),
//! new in-scope exception invalidates without touching the rule, unrelated
//! scope write preserves reuse, environment change invalidates, unsupported
//! fingerprint never means unchanged.

use cortex_kernel::db::compiled::{bump_guard, guard_epoch, run_compiled};
use cortex_kernel::db::records::{append_commit, append_revision, NewRevision};
use cortex_kernel::handlers::operations::recipes;
use cortex_logic::recipe::{evaluate, Fact, Limits, Predicate, RecipeError, Snapshot, Step};
use cortex_tests::support::test_conn;
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn fact(id: &str, relation: &str, fields: Value) -> Fact {
    Fact {
        id: id.into(),
        scope: "default".into(),
        relation: relation.into(),
        fields: fields
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        valid_from: None,
        valid_until: None,
        known_seq: 1,
    }
}

fn retry_recipe() -> (Vec<Step>, Vec<String>) {
    (
        vec![
            Step::Select {
                id: "rules".into(),
                relation: "constraint".into(),
            },
            Step::Filter {
                id: "retry_rules".into(),
                input: "rules".into(),
                field: "subject".into(),
                predicate: Predicate::Eq {
                    value: json!("retry"),
                },
            },
            Step::Select {
                id: "exceptions".into(),
                relation: "exception".into(),
            },
            Step::Exists {
                id: "has_exceptions".into(),
                input: "exceptions".into(),
            },
            Step::Project {
                id: "out".into(),
                input: "retry_rules".into(),
                fields: vec!["text".into()],
            },
        ],
        vec!["out".into(), "has_exceptions".into()],
    )
}

#[test]
fn current_constraints_template_includes_policy_kind() {
    let mut snap = Snapshot {
        brain_epoch: "b1".into(),
        policy_epoch: "p1".into(),
        environment: "e1".into(),
        ..Snapshot::default()
    };
    snap.put(fact(
        "p1",
        "policy",
        json!({"subject": "tls", "text": "require TLS 1.3 on the payments webhook"}),
    ));
    snap.put(fact(
        "d1",
        "decision",
        json!({"subject": "other", "text": "a plain decision is not a constraint"}),
    ));
    let (steps, outputs) = recipes::template("current_constraints", &json!({})).unwrap();
    let ok = evaluate(&snap, "default", &steps, &outputs, Limits::default()).unwrap();
    let out = ok.values["out"].as_array().expect("out rows");
    assert_eq!(out.len(), 1, "plain decisions must not appear: {ok:?}");
    assert_eq!(
        out[0]["fields"]["text"],
        "require TLS 1.3 on the payments webhook",
        "{ok:?}"
    );
    assert!(
        ok.guards.contains_key(&("default".into(), "policy".into())),
        "compiled reads must watch the policy guard epoch: {ok:?}"
    );
}

#[test]
fn interpreter_is_bounded_and_rejects_cycles_unknown_inputs_and_overwork() {
    let mut snap = Snapshot {
        brain_epoch: "b1".into(),
        policy_epoch: "p1".into(),
        environment: "e1".into(),
        ..Snapshot::default()
    };
    snap.put(fact(
        "r1",
        "constraint",
        json!({"subject": "retry", "text": "retry at most three times"}),
    ));
    let (steps, outputs) = retry_recipe();
    let ok = evaluate(&snap, "default", &steps, &outputs, Limits::default()).unwrap();
    assert_eq!(ok.values["has_exceptions"], false);
    assert_eq!(
        ok.values["out"][0]["fields"]["text"],
        "retry at most three times"
    );
    assert!(
        ok.guards
            .contains_key(&("default".into(), "exception".into())),
        "an absence query is a negative dependency"
    );
    assert!(ok.positive.contains("r1"));
    let cyc = vec![Step::Filter {
        id: "a".into(),
        input: "a".into(),
        field: "x".into(),
        predicate: Predicate::Eq { value: json!(1) },
    }];
    assert_eq!(
        evaluate(&snap, "default", &cyc, &["a".into()], Limits::default()).unwrap_err(),
        RecipeError::Cycle
    );
    let unknown = vec![Step::Count {
        id: "c".into(),
        input: "nope".into(),
    }];
    assert_eq!(
        evaluate(&snap, "default", &unknown, &["c".into()], Limits::default()).unwrap_err(),
        RecipeError::UnknownInput("nope".into())
    );
    assert_eq!(
        evaluate(
            &snap,
            "default",
            &steps,
            &outputs,
            Limits {
                max_work: 1,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        RecipeError::WorkLimit
    );
    assert_eq!(
        evaluate(
            &snap,
            "default",
            &steps,
            &outputs,
            Limits {
                max_steps: 2,
                ..Limits::default()
            }
        )
        .unwrap_err(),
        RecipeError::StepLimit
    );
    assert_eq!(
        evaluate(
            &snap,
            "default",
            &steps,
            &["missing".into()],
            Limits::default()
        )
        .unwrap_err(),
        RecipeError::UnknownOutput("missing".into())
    );
    snap.put(fact("ok", "item", json!({"status": "ok"})));
    snap.put(fact("nameless", "item", json!({"name": "x"})));
    snap.put(fact("failed", "item", json!({"status": "failed"})));
    let ne = vec![
        Step::Select {
            id: "all".into(),
            relation: "item".into(),
        },
        Step::Filter {
            id: "not_failed".into(),
            input: "all".into(),
            field: "status".into(),
            predicate: Predicate::Ne {
                value: json!("failed"),
            },
        },
        Step::Count {
            id: "n".into(),
            input: "not_failed".into(),
        },
    ];
    let counted = evaluate(&snap, "default", &ne, &["n".into()], Limits::default()).unwrap();
    assert_eq!(
        counted.values["n"],
        json!(1),
        "Ne must not treat a missing field as not-equal: {counted:?}"
    );
    let dup = vec![
        Step::Select {
            id: "x".into(),
            relation: "constraint".into(),
        },
        Step::Select {
            id: "x".into(),
            relation: "exception".into(),
        },
    ];
    assert_eq!(
        evaluate(&snap, "default", &dup, &[], Limits::default()).unwrap_err(),
        RecipeError::DuplicateStepId
    );
    let render = vec![
        Step::Select {
            id: "r".into(),
            relation: "constraint".into(),
        },
        Step::Render {
            id: "t".into(),
            input: "r".into(),
            template: "{{shell}}".into(),
        },
    ];
    assert!(
        matches!(
            evaluate(&snap, "default", &render, &["t".into()], Limits::default()).unwrap_err(),
            RecipeError::UnsupportedFeature(_)
        ),
        "render cannot evaluate arbitrary templates"
    );
}

#[test]
fn open_work_and_handoff_keep_observed_complete() {
    // `observed_complete` is unfinished: the checker has not verified it.
    // Dropping it from open_work hid work the inspection view still lists.
    let mut snap = Snapshot {
        brain_epoch: "b1".into(),
        policy_epoch: "p1".into(),
        environment: "e1".into(),
        ..Snapshot::default()
    };
    snap.put(fact(
        "oc",
        "obligation",
        json!({"state": "observed_complete", "title": "awaiting checker", "record": "oc"}),
    ));
    snap.put(fact(
        "done",
        "obligation",
        json!({"state": "verified_complete", "title": "shipped", "record": "done"}),
    ));
    snap.put(fact(
        "nope",
        "obligation",
        json!({"state": "cancelled", "title": "dropped", "record": "nope"}),
    ));
    let (steps, outputs) = recipes::template("open_work", &json!({})).unwrap();
    let ok = evaluate(&snap, "default", &steps, &outputs, Limits::default()).unwrap();
    let out = ok.values["out"].as_array().expect("out");
    assert_eq!(out.len(), 1, "{ok:?}");
    assert_eq!(out[0]["fields"]["title"], "awaiting checker");
    assert_eq!(ok.values["count"], 1);

    let (h_steps, h_outputs) = recipes::template("handoff", &json!({})).unwrap();
    let handoff = evaluate(&snap, "default", &h_steps, &h_outputs, Limits::default()).unwrap();
    let open = handoff.values["open_work"].as_array().expect("open_work");
    assert_eq!(open.len(), 1, "{handoff:?}");
    assert_eq!(open[0]["fields"]["record"], "oc");
}

#[test]
fn new_exception_invalidates_without_touching_the_rule_and_unrelated_scope_preserves_reuse() {
    let mut snap = Snapshot {
        brain_epoch: "b1".into(),
        policy_epoch: "p1".into(),
        environment: "e1".into(),
        ..Snapshot::default()
    };
    snap.put(fact(
        "r1",
        "constraint",
        json!({"subject": "retry", "text": "retry at most three times"}),
    ));
    let (steps, outputs) = retry_recipe();
    let result = evaluate(&snap, "default", &steps, &outputs, Limits::default()).unwrap();
    assert!(result.reusable(&snap, "default").is_ok());
    // Unrelated relation in the same scope: reusable (range not searched).
    snap.put(fact("n1", "note", json!({"text": "palette changed"})));
    assert!(
        result.reusable(&snap, "default").is_ok(),
        "an unrelated write must not invalidate"
    );
    // New exception: the rule r1 is untouched, yet the absence query moved.
    snap.put(fact(
        "e1",
        "exception",
        json!({"subject": "retry", "text": "never after ledger commit"}),
    ));
    let err = result.reusable(&snap, "default").unwrap_err();
    assert!(err.contains("exception"), "{err}");
    // Recompute: has_exceptions flips.
    let fresh = evaluate(&snap, "default", &steps, &outputs, Limits::default()).unwrap();
    assert_eq!(fresh.values["has_exceptions"], true);
    // Different scope, environment or epoch are never reusable.
    assert!(fresh.reusable(&snap, "other-scope").is_err());
    let mut env = snap.clone();
    env.environment = "e2".into();
    assert!(fresh
        .reusable(&env, "default")
        .unwrap_err()
        .contains("environment"));
    let mut epoch = snap.clone();
    epoch.policy_epoch = "p2".into();
    assert!(fresh.reusable(&epoch, "default").is_err());
    // Removing a positive dependency invalidates too.
    let mut removed = snap.clone();
    removed.remove("r1");
    let err = fresh.reusable(&removed, "default").unwrap_err();
    assert!(err.contains("r1") || err.contains("constraint"), "{err}");
}

#[test]
fn temporal_slice_conflict_join_and_compare_are_typed() {
    let mut snap = Snapshot {
        brain_epoch: "b".into(),
        policy_epoch: "p".into(),
        environment: "e".into(),
        ..Snapshot::default()
    };
    snap.put(Fact {
        valid_from: Some(100),
        valid_until: Some(200),
        known_seq: 5,
        ..fact(
            "d1",
            "decision",
            json!({"key": "deploy", "value": "blue", "text": "blue"}),
        )
    });
    snap.put(Fact {
        valid_from: Some(150),
        valid_until: None,
        known_seq: 9,
        ..fact(
            "d2",
            "decision",
            json!({"key": "deploy", "value": "green", "text": "green"}),
        )
    });
    snap.put(fact("d3", "decision", json!({"text": "orphan-a"})));
    snap.put(fact("d4", "decision", json!({"text": "orphan-b"})));
    let steps = vec![
        Step::Select {
            id: "d".into(),
            relation: "decision".into(),
        },
        Step::TemporalSlice {
            id: "then".into(),
            input: "d".into(),
            valid_at: 120,
            known_seq: 6,
        },
        Step::Count {
            id: "known_then".into(),
            input: "then".into(),
        },
        Step::TemporalSlice {
            id: "now".into(),
            input: "d".into(),
            valid_at: 160,
            known_seq: 100,
        },
        Step::Conflict {
            id: "x".into(),
            input: "now".into(),
            key: "key".into(),
            value_field: "value".into(),
        },
        Step::Filter {
            id: "green".into(),
            input: "now".into(),
            field: "value".into(),
            predicate: Predicate::Eq {
                value: json!("green"),
            },
        },
        Step::Compare {
            id: "cmp".into(),
            left: "then".into(),
            right: "green".into(),
            fields: vec!["value".into()],
        },
    ];
    let r = evaluate(
        &snap,
        "default",
        &steps,
        &["known_then".into(), "x".into(), "cmp".into()],
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        r.values["known_then"], 1,
        "as-known at seq 6 excludes the later fact"
    );
    assert_eq!(
        r.values["x"].as_array().map(Vec::len),
        Some(1),
        "facts missing the key field are not a second conflict group: {}",
        r.values["x"]
    );
    assert_eq!(
        r.values["x"][0]["key"], "\"deploy\"",
        "two applicable heads with different values are a conflict, not an average"
    );
    assert_eq!(r.values["cmp"][0]["field"], "value");
}

#[test]
fn what_changed_lists_only_decisions_recorded_after_since_seq() {
    let conn = test_conn();
    let seq1 = append_commit(&conn, "solo", None, "process_crash").unwrap();
    append_revision(
        &conn,
        seq1,
        NewRevision {
            record_id: "decision:old",
            kind: "decision",
            retention: "durable",
            body: json!({"text": "before-cursor"}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "t",
        },
    )
    .unwrap();
    let seq2 = append_commit(&conn, "solo", None, "process_crash").unwrap();
    append_revision(
        &conn,
        seq2,
        NewRevision {
            record_id: "decision:new",
            kind: "decision",
            retention: "durable",
            body: json!({"text": "after-cursor"}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "t",
        },
    )
    .unwrap();
    // JSON-RPC often sends numbers as floats; `as_i64()` alone would default
    // `since` to 0 and list every decision.
    let params = json!({"since_seq": seq1 as f64});
    let (steps, outputs) = recipes::template("what_changed", &params).expect("template");
    let (value, _, _) = run_compiled(
        &conn,
        "solo",
        "what_changed",
        &steps,
        &outputs,
        &params,
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    let out = value["values"]["out"].as_array().expect("out");
    assert_eq!(out.len(), 1, "{value}");
    assert_eq!(out[0]["fields"]["text"], "after-cursor", "{value}");
    assert_eq!(value["values"]["known_before"], 1, "{value}");
    assert_eq!(value["values"]["known_now"], 2, "{value}");
}

#[test]
fn compiled_reads_reuse_until_a_guard_moves_in_the_database() {
    let conn = test_conn();
    let seq = append_commit(&conn, "solo", None, "process_crash").unwrap();
    append_revision(
        &conn,
        seq,
        NewRevision {
            record_id: "constraint:retry",
            kind: "constraint",
            retention: "durable",
            body: json!({"subject": "retry", "text": "retry at most three times"}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "t",
        },
    )
    .unwrap();
    let (steps, outputs) = retry_recipe();
    let (first, cached, _) = run_compiled(
        &conn,
        "solo",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    assert!(!cached);
    assert_eq!(first["values"]["has_exceptions"], false);
    let (again, cached, _) = run_compiled(
        &conn,
        "solo",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    assert!(cached, "identical guards → reuse");
    assert_eq!(again, first);
    // Another principal or environment never sees this cache entry.
    let (_, cached_other, _) = run_compiled(
        &conn,
        "user:9",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    assert!(!cached_other, "cache identity includes the principal");
    let (_, cached_env, _) = run_compiled(
        &conn,
        "solo",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:feature",
        Limits::default(),
    )
    .unwrap();
    assert!(
        !cached_env,
        "cache identity includes the environment fingerprint"
    );
    // A new exception revision bumps the guard inside the write path.
    let before = guard_epoch(&conn, "default", "exception");
    let seq2 = append_commit(&conn, "solo", None, "process_crash").unwrap();
    append_revision(
        &conn,
        seq2,
        NewRevision {
            record_id: "exception:1",
            kind: "exception",
            retention: "durable",
            body: json!({"subject": "retry", "text": "never after commit"}),
            epistemic_status: "asserted",
            parents: &[],
            replace_parents: true,
            representation_version: "t",
        },
    )
    .unwrap();
    assert!(
        guard_epoch(&conn, "default", "exception") > before,
        "mutation advances the searched range's epoch"
    );
    let (third, cached, _) = run_compiled(
        &conn,
        "solo",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    assert!(
        !cached,
        "a new in-scope exception invalidates the cached answer"
    );
    assert_eq!(third["values"]["has_exceptions"], true);
    // Coarse fallback: an explicit scope-wide bump invalidates everything.
    bump_guard(&conn, "default", "*").unwrap();
    let (_, cached, _) = run_compiled(
        &conn,
        "solo",
        "retry-policy",
        &steps,
        &outputs,
        &json!({}),
        "branch:main",
        Limits::default(),
    )
    .unwrap();
    assert!(cached, "the scope-wide epoch is not a searched guard of this recipe; only searched domains invalidate");
}

#[test]
fn query_runs_named_templates_and_proposed_plans_through_compiled_reads() {
    cortex_tests::support::run_with_cx(|cx| async move {
        use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
        use cortex_tests::support::solo_state;
        let state = solo_state();
        let caller = || Caller {
            owner_id: None,
            agent: "recipe-agent",
            principal: "solo".into(),
        };
        dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "constraint", "text": "RCP-1 retries stop at the ledger commit"}]})).await.unwrap();
        dispatch(&cx, &state, caller(), Operation::Checkpoint, &json!({"thread": "rcp", "action": "attempt", "attempt": {"exit_status": 1, "failure": "duplicate charge", "text": "RCP-1 raised retries"}})).await.unwrap();
        let first = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "constraints", "needs": ["recipe:current_constraints"], "environment": "branch:main"})).await.unwrap();
        assert_eq!(first["status"], "ok", "{first}");
        assert_eq!(first["recipe"]["cached"], false);
        assert_eq!(first["values"]["has_exceptions"], false);
        assert!(
            first["values"]["out"][0]["fields"]["text"]
                .as_str()
                .unwrap()
                .contains("RCP-1"),
            "{first}"
        );
        let second = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "constraints", "needs": ["recipe:current_constraints"], "environment": "branch:main"})).await.unwrap();
        assert_eq!(
            second["recipe"]["cached"], true,
            "unchanged guards reuse the compiled read: {second}"
        );
        dispatch(&cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "exception", "text": "RCP-1 exception: health checks may retry after commit"}]})).await.unwrap();
        let third = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "constraints", "needs": ["recipe:current_constraints"], "environment": "branch:main"})).await.unwrap();
        assert_eq!(
            third["recipe"]["cached"], false,
            "a new exception invalidates: {third}"
        );
        assert_eq!(third["values"]["has_exceptions"], true);
        let failed = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "x", "recipe": "failed_attempts"}),
        )
        .await
        .unwrap();
        assert_eq!(
            failed["values"]["out"][0]["fields"]["failure"], "duplicate charge",
            "{failed}"
        );
        let unknown = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "RCP-1", "recipe": "summarize_everything"}),
        )
        .await
        .unwrap();
        assert_eq!(
            unknown["recipe"]["status"], "unknown_recipe_fell_back_to_recall",
            "{unknown}"
        );
        assert!(unknown["cards"].is_array());
        let proposed = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "x", "recipe": "proposed", "plan": {"steps": [{"op": "Select", "id": "c", "relation": "constraint"}, {"op": "Count", "id": "n", "input": "c"}], "outputs": ["n"]}})).await.unwrap();
        assert_eq!(proposed["status"], "ok", "{proposed}");
        assert_eq!(proposed["values"]["n"], 1);
        assert_eq!(proposed["recipe"]["source"]["kind"], "agent_proposed");
        assert!(
            proposed["recipe"]["source"]["annotation_bytes"]
                .as_u64()
                .unwrap()
                > 0,
            "agent annotation is counted"
        );
        let bad = dispatch(&cx, &state, caller(), Operation::Query, &json!({"need": "x", "recipe": "proposed", "plan": {"steps": [{"op": "Shell", "id": "s", "cmd": "rm"}], "outputs": ["s"]}})).await.unwrap();
        assert_eq!(
            bad["status"], "invalid_request",
            "unknown operators are rejected at parse time: {bad}"
        );
    });
}
