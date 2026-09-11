//! Assemblies are exact membership over existing revisions. Ranking never
//! changes epistemic status. Routes stay off until an explicit rebuild.
use cortex_kernel::{
    assembly::{canonical_json, expand_records, factor_records, rank_routes, route_edges, FactorCodec, LearningEvent, LearningKind, MembershipRole},
    handlers::operations::{dispatch, Caller, Operation},
    runtime::{
        assembly::{AssemblyMemberSpec, AssemblySpec},
        cycle::NeedSpec,
        observation::{ObservationEvent, SourceSpec},
        CortexRuntime,
    },
    LensInput,
};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::{json, Value};

fn caller() -> Caller<'static> {
    Caller {
        owner_id: None,
        agent: "ops-agent",
        principal: "solo".into(),
    }
}

fn event(key: &str, text: &str) -> ObservationEvent {
    ObservationEvent {
        event_key: key.into(),
        text: text.into(),
        observed_at: None,
    }
}

fn need(learned: bool) -> NeedSpec {
    NeedSpec {
        id: "active".into(),
        scope: "repo".into(),
        cues: vec!["retry".into()],
        exclude_cues: vec![],
        max_results: 32,
        max_bytes: 65536,
        ttl_seconds: 3600,
        learned,
    }
}

fn learn(
    origin: &str,
    event_id: &str,
    target: &str,
    training_unit: &str,
    reward: i8,
    cues: &[&str],
    sources: &[&str],
) -> LearningEvent {
    LearningEvent {
        origin: origin.into(),
        origin_event_id: event_id.into(),
        principal: "local".into(),
        scope: "repo".into(),
        training_unit: training_unit.into(),
        target: target.into(),
        kind: LearningKind::Explicit,
        reward,
        cues: cues.iter().map(|cue| (*cue).to_string()).collect(),
        sources: sources.iter().map(|source| (*source).to_string()).collect(),
        observed_at: 1,
        receipt_ref: format!("receipt:{event_id}"),
    }
}

#[test]
fn factor_roundtrip_keeps_typed_json_and_falls_back_to_raw() {
    let rows = vec![
        json!({"common":"pad-pad-pad-pad-pad-pad","flag":true}),
        json!({"common":"pad-pad-pad-pad-pad-pad","flag":1}),
        json!({"common":"pad-pad-pad-pad-pad-pad","flag":"1"}),
        json!({"common":"pad-pad-pad-pad-pad-pad","flag":Value::Null}),
        json!({"common":"pad-pad-pad-pad-pad-pad"}),
        json!({"common":"pad-pad-pad-pad-pad-pad","flag":false}),
    ];
    let pack = factor_records(&rows);
    assert_eq!(pack["codec"], FactorCodec::TemplateResidual.as_str());
    let expanded = expand_records(&pack).unwrap();
    assert_eq!(expanded.len(), rows.len());
    for (got, want) in expanded.iter().zip(&rows) {
        assert_eq!(canonical_json(got), canonical_json(want), "{got} vs {want}");
    }
    assert_ne!(canonical_json(&rows[0]["flag"]), canonical_json(&rows[1]["flag"]));
    assert_ne!(canonical_json(&rows[1]["flag"]), canonical_json(&rows[2]["flag"]));
    assert_ne!(canonical_json(&rows[3]), canonical_json(&rows[4]));
    assert_ne!(canonical_json(&rows[3]["flag"]), canonical_json(&rows[5]["flag"]));

    let sparse = vec![json!({"a":1}), json!({"b":2})];
    let raw = factor_records(&sparse);
    assert_eq!(raw["codec"], FactorCodec::Raw.as_str());
    assert_eq!(expand_records(&raw).unwrap(), sparse);
}

#[test]
fn assembly_membership_is_exact_and_expand_returns_original_bodies() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("counter", "repo"))
            .await
            .unwrap();
        let noted = runtime
            .observe(&cx, "notes", "g", event("one", "retry requires idempotency"))
            .await
            .unwrap();
        let countered = runtime
            .observe(&cx, "counter", "g", event("one", "retry is unsafe here"))
            .await
            .unwrap();
        runtime
            .deposit(
                &cx,
                "req-asm-1",
                "ASM-1 ledger writes must stay idempotent",
                "outfit",
                None,
            )
            .await
            .unwrap();
        let fact = {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            conn.query_row(
                "SELECT revision_id FROM revisions WHERE revision_id LIKE 'decision:%' ORDER BY recorded_sequence DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        let stored = runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
                    kind: "rule".into(),
                    members: vec![
                        AssemblyMemberSpec {
                            revision_id: noted.revision_id.clone(),
                            role: MembershipRole::Observation,
                        },
                        AssemblyMemberSpec {
                            revision_id: countered.revision_id.clone(),
                            role: MembershipRole::Contradiction,
                        },
                        AssemblyMemberSpec {
                            revision_id: fact.clone(),
                            role: MembershipRole::Support,
                        },
                    ],
                    guards: vec![],
                },
            )
            .await
            .unwrap();
        assert_eq!(stored.members.len(), 3);
        assert_eq!(stored.members[1].role, MembershipRole::Contradiction);
        let expanded = runtime.expand_assembly(&cx, "bundle").await.unwrap();
        let bodies = {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            vec![
                cortex_kernel::db::records::revision_body(&conn, &noted.revision_id)
                    .unwrap()
                    .unwrap(),
                cortex_kernel::db::records::revision_body(&conn, &countered.revision_id)
                    .unwrap()
                    .unwrap(),
                cortex_kernel::db::records::revision_body(&conn, &fact).unwrap().unwrap(),
            ]
        };
        assert_eq!(expanded, bodies);
        let again = runtime.get_assembly(&cx, "bundle").await.unwrap();
        assert_eq!(again.revision_id, stored.revision_id);
        assert_eq!(again.members, stored.members);
    });
}

#[test]
fn learning_is_idempotent_retractable_and_source_erasable() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        let noted = runtime
            .observe(&cx, "notes", "g", event("one", "retry requires idempotency"))
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
                    kind: "rule".into(),
                    members: vec![AssemblyMemberSpec {
                        revision_id: noted.revision_id.clone(),
                        role: MembershipRole::Observation,
                    }],
                    guards: vec![],
                },
            )
            .await
            .unwrap();
        let first = learn("host", "e1", "bundle", "unit-a", 1, &["retry"], &[&noted.source_id]);
        assert!(runtime.record_learning_event(&cx, first.clone()).await.unwrap());
        assert!(!runtime.record_learning_event(&cx, first.clone()).await.unwrap());
        let mut conflicted = first.clone();
        conflicted.reward = -1;
        assert_eq!(
            runtime.record_learning_event(&cx, conflicted).await.unwrap_err(),
            "feedback_identity_conflict"
        );
        assert_eq!(runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap(), 1);
        runtime
            .retract_learning_event(&cx, "host", "e1", "withdrawn")
            .await
            .unwrap();
        assert!(!runtime.record_learning_event(&cx, first).await.unwrap());
        assert_eq!(runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap(), 0);
        assert!(
            runtime
                .explain_assembly_routes(&cx, "repo", &["retry".into()], 8)
                .await
                .unwrap()
                .is_empty()
        );

        let second = learn("host", "e2", "bundle", "unit-b", 1, &["retry"], &[&noted.source_id]);
        assert!(runtime.record_learning_event(&cx, second.clone()).await.unwrap());
        assert_eq!(
            runtime.erase_learning_source(&cx, &noted.source_id).await.unwrap(),
            2,
            "erasure must retract every event that named the source, including a prior retraction"
        );
        assert!(!runtime.record_learning_event(&cx, second).await.unwrap());
        let later = learn("host", "e3", "bundle", "unit-c", 1, &["retry"], &[&noted.source_id]);
        assert!(!runtime.record_learning_event(&cx, later).await.unwrap());
        assert_eq!(runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap(), 0);
    });
}

#[test]
fn training_units_do_not_amplify_or_vote_across_conflict() {
    let copied = vec![
        learn("host", "a1", "bundle", "copied", 1, &["retry"], &[]),
        learn("host", "a2", "bundle", "copied", 1, &["retry"], &[]),
    ];
    let edges = route_edges(&copied, 10).unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].positive, 1.0);
    assert_eq!(edges[0].negative, 0.0);
    let ranked = rank_routes(&edges, &["retry".into()], &["bundle".into()], 0.5);
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].mass, 1.0);

    let conflicted = vec![
        learn("host", "b1", "bundle", "split", 1, &["retry"], &[]),
        learn("host", "b2", "bundle", "split", -1, &["retry"], &[]),
    ];
    assert!(route_edges(&conflicted, 10).unwrap().is_empty());
}

#[test]
fn routes_default_off_and_rank_does_not_change_epistemic_status() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("extra", "repo"))
            .await
            .unwrap();
        let noted = runtime
            .observe(&cx, "notes", "g", event("one", "retry requires idempotency"))
            .await
            .unwrap();
        let extra = runtime
            .observe(&cx, "extra", "g", event("one", "widget assembly only"))
            .await
            .unwrap();
        runtime
            .deposit(
                &cx,
                "req-asm-2",
                "ASM-2 clock handle stays a fact",
                "outfit",
                None,
            )
            .await
            .unwrap();
        let before = runtime
            .lens(
                &cx,
                LensInput {
                    query: "ASM-2".into(),
                    agent: "outfit".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
                    kind: "rule".into(),
                    members: vec![
                        AssemblyMemberSpec {
                            revision_id: noted.revision_id.clone(),
                            role: MembershipRole::Observation,
                        },
                        AssemblyMemberSpec {
                            revision_id: extra.revision_id.clone(),
                            role: MembershipRole::Contradiction,
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
                learn("host", "e1", "bundle", "unit-a", 1, &["retry"], &[&noted.source_id]),
            )
            .await
            .unwrap();
        let stored = runtime.get_assembly(&cx, "bundle").await.unwrap();
        assert_eq!(stored.members[1].role, MembershipRole::Contradiction);
        assert!(
            runtime
                .explain_assembly_routes(&cx, "repo", &["retry".into()], 8)
                .await
                .unwrap()
                .is_empty()
        );
        let literal = runtime
            .query_observations(&cx, "repo", "retry", 32, 65536, false)
            .await
            .unwrap();
        assert_eq!(literal.status, "ready");
        assert!(literal.evidence.iter().all(|item| item.source_id != extra.source_id));
        assert!(literal.evidence.iter().all(|item| item.route == "literal"));
        let after = runtime
            .lens(
                &cx,
                LensInput {
                    query: "ASM-2".into(),
                    agent: "outfit".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(before["results"], after["results"]);
        assert_eq!(
            runtime.expand_assembly(&cx, "bundle").await.unwrap().len(),
            2
        );
    });
}

#[test]
fn prepare_labels_assembly_routes_only_after_rebuild() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("extra", "repo"))
            .await
            .unwrap();
        let noted = runtime
            .observe(&cx, "notes", "g", event("one", "retry requires idempotency"))
            .await
            .unwrap();
        let extra = runtime
            .observe(&cx, "extra", "g", event("one", "widget assembly only"))
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
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
                learn("host", "e1", "bundle", "unit-a", 1, &["retry"], &[&noted.source_id]),
            )
            .await
            .unwrap();
        runtime.reset_associations(&cx, "repo").await.unwrap();
        let before = runtime.subscribe_observations(&cx, need(true)).await.unwrap();
        assert!(before.evidence.iter().all(|item| item.source_id != extra.source_id));
        assert_eq!(runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap(), 1);
        let why = runtime
            .explain_assembly_routes(&cx, "repo", &["retry".into()], 8)
            .await
            .unwrap();
        assert_eq!(why.len(), 1);
        assert_eq!(why[0].assembly_id, "bundle");
        assert_eq!(why[0].cues, vec!["retry".to_string()]);
        assert_eq!(why[0].training_units, vec!["unit-a".to_string()]);
        let prepared = runtime
            .prepare_observations(&cx, "active", "context-1", None)
            .await
            .unwrap();
        assert_eq!(prepared.status, "ready");
        assert!(
            prepared
                .evidence
                .iter()
                .any(|item| item.source_id == extra.source_id && item.route == "assembly_exception")
        );
        assert!(
            prepared
                .evidence
                .iter()
                .any(|item| item.source_id == noted.source_id && item.route == "literal")
        );
        let literal = runtime
            .query_observations(&cx, "repo", "retry", 32, 65536, false)
            .await
            .unwrap();
        assert!(literal.evidence.iter().all(|item| item.source_id != extra.source_id));
        runtime
            .retract_observation(&cx, &extra.source_id, "withdrawn")
            .await
            .unwrap();
        let blocked = runtime
            .prepare_observations(&cx, "active", "context-2", None)
            .await
            .unwrap();
        assert_eq!(blocked.status, "qualification_unavailable");
        assert!(blocked.evidence.is_empty());
        runtime.reset_assembly_routes(&cx, "repo").await.unwrap();
        runtime
            .observe(&cx, "extra", "g2", event("two", "widget assembly restored"))
            .await
            .unwrap();
        let off = runtime
            .query_observations(&cx, "repo", "retry", 32, 65536, true)
            .await
            .unwrap();
        assert!(off.evidence.iter().all(|item| item.route != "learned_assembly_route"));
        assert!(off.evidence.iter().all(|item| item.route != "assembly_exception"));
    });
}

#[test]
fn orient_compiles_closed_assemblies_only_after_rebuild() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let runtime = CortexRuntime::from_state(state.clone());
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("extra", "repo"))
            .await
            .unwrap();
        let noted = runtime
            .observe(&cx, "notes", "g", event("one", "retry requires idempotency"))
            .await
            .unwrap();
        let extra = runtime
            .observe(&cx, "extra", "g", event("one", "widget assembly only"))
            .await
            .unwrap();
        runtime
            .deposit(
                &cx,
                "req-asm-3",
                "ASM-3 clock handle stays a fact",
                "outfit",
                None,
            )
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
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
                learn("host", "e1", "bundle", "unit-a", 1, &["retry"], &[&noted.source_id]),
            )
            .await
            .unwrap();
        let quiet = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "retry", "observation_scope": "repo", "budget": 4000}),
        )
        .await
        .unwrap();
        assert!(
            quiet.get("assemblies").is_none(),
            "routes off must omit assemblies: {quiet}"
        );
        assert!(
            quiet["cards"]
                .as_array()
                .unwrap()
                .iter()
                .all(|card| card["trust"]["kind"] == "recalled_claim"),
            "{quiet}"
        );
        runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap();
        let view = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "retry", "observation_scope": "repo", "budget": 4000}),
        )
        .await
        .unwrap();
        let assemblies = view.get("assemblies").unwrap_or_else(|| panic!("missing assemblies: {view}"));
        assert_eq!(assemblies["status"], "ready");
        assert_eq!(assemblies["scope"], "repo");
        let bundles = assemblies["bundles"].as_array().unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0]["id"], "bundle");
        assert_eq!(bundles[0]["status"], "ready");
        assert_eq!(bundles[0]["route"], "learned_assembly_route");
        let members = bundles[0]["members"].as_array().unwrap();
        assert!(members.iter().any(|m| m["role"] == "observation" && m["expand"] == format!("obs:{}", noted.source_id)));
        assert!(members.iter().any(|m| m["role"] == "exception" && m["required"] == true));
        assert!(assemblies["brief"].as_str().unwrap().contains("Assembly bundle"));
        assert!(assemblies["brief"].as_str().unwrap().contains("Exception:"));
        let lens = runtime
            .lens(
                &cx,
                LensInput {
                    query: "ASM-3".into(),
                    agent: "outfit".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(lens.to_string().contains("ASM-3"));
        let opted = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "retry", "observation_scope": "repo", "assemblies": false}),
        )
        .await
        .unwrap();
        assert!(opted.get("assemblies").is_none(), "{opted}");
        let expanded = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Expand,
            &json!({"reference": "asm:bundle"}),
        )
        .await
        .unwrap();
        assert_eq!(expanded["status"], "ok");
        assert_eq!(expanded["assembly"]["id"], "bundle");
        assert_eq!(expanded["assembly"]["members"].as_array().unwrap().len(), 2);
        runtime
            .retract_observation(&cx, &extra.source_id, "withdrawn")
            .await
            .unwrap();
        let blocked = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "retry", "observation_scope": "repo"}),
        )
        .await
        .unwrap();
        assert_eq!(blocked["assemblies"]["status"], "qualification_unavailable");
        assert!(blocked["assemblies"]["bundles"][0]["members"].as_array().unwrap().is_empty());
        assert_eq!(blocked["assemblies"]["brief"], "");
        let restored = runtime
            .observe(&cx, "extra", "g2", event("two", "widget assembly restored"))
            .await
            .unwrap();
        runtime
            .put_assembly(
                &cx,
                AssemblySpec {
                    id: "bundle".into(),
                    scope: "repo".into(),
                    kind: "rule".into(),
                    members: vec![
                        AssemblyMemberSpec {
                            revision_id: noted.revision_id.clone(),
                            role: MembershipRole::Observation,
                        },
                        AssemblyMemberSpec {
                            revision_id: restored.revision_id.clone(),
                            role: MembershipRole::Exception,
                        },
                    ],
                    guards: vec![],
                },
            )
            .await
            .unwrap();
        let (_, restore, policy) = {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            cortex_kernel::db::records::brain_epochs(&conn)
        };
        let stored = runtime.get_assembly(&cx, "bundle").await.unwrap();
        runtime.rebuild_assembly_routes(&cx, "repo").await.unwrap();
        let present = dispatch(
            &cx,
            &state,
            caller(),
            Operation::Query,
            &json!({
                "need": "retry",
                "observation_scope": "repo",
                "context_epoch": "ctx-1",
                "context_presence": {
                    "context_epoch": "ctx-1",
                    "invocation": "inv-1",
                    "brain_epoch": restore,
                    "policy_epoch": policy,
                    "present": [{
                        "revision": {"namespace": "revision", "value": stored.revision_id},
                        "representation": "assembly_brief"
                    }]
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(present["assemblies"]["bundles"][0]["present"], true);
        assert!(present["assemblies"]["bundles"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m.get("preview").is_none()));
        assert!(present["assemblies"]["brief"].as_str().unwrap().contains("already present"));
    });
}
