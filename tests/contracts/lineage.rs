//! Witness lineage laws: a graph hop counts once; shared ancestry is one
//! family; an exact single-source report is still retrievable; expansion
//! never mints a hard anchor; `why` answers the four questions separately.

use cortex_logic::clockwork::{
    admit_with_lineage, independent_support, ClockEvidence, Rankable, Witness, WitnessDomain,
};
use cortex_kernel::handlers::operations::{dispatch, Caller, Operation};
use cortex_kernel::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_tests::support::solo_state;
use serde_json::{json, Value};

fn rankable(hard: bool, strong: bool, ev: ClockEvidence) -> Rankable {
    Rankable {
        eligible: true,
        hard_anchor: hard,
        evidence: ev,
        strong_lexical: strong,
    }
}

#[test]
fn one_traversal_is_one_origin_however_many_counters_it_touched() {
    let hop_only = vec![Witness::derived(
        WitnessDomain::Hop,
        "hop:decision::1",
        "observed_with",
        1,
    )];
    assert_eq!(independent_support(&hop_only), 1);
    // The legacy counters say write=1, truth=1 (two nonzero clocks).
    let legacy = ClockEvidence {
        write: 1,
        truth: 1,
        task: 0,
        history: 0,
    };
    assert_eq!(
        admit_with_lineage(rankable(false, false, legacy), &hop_only),
        None,
        "hop-only rows are leads, not answers"
    );
    // Two hops from the same seed family are still one origin.
    let two_hops = vec![
        Witness::derived(WitnessDomain::Hop, "hop:decision::1", "same_path", 1),
        Witness::derived(WitnessDomain::Hop, "hop:decision::1", "observed_with", 2),
    ];
    assert_eq!(independent_support(&two_hops), 1);
    assert_eq!(
        admit_with_lineage(rankable(false, false, legacy), &two_hops),
        None
    );
    // A direct lexical channel plus a hop from an unrelated seed is a quorum.
    let mixed = vec![
        Witness::direct(WitnessDomain::Lexical, "lexical:decision::9", "retry", 1),
        Witness::derived(WitnessDomain::Hop, "hop:decision::1", "observed_with", 1),
    ];
    assert_eq!(
        admit_with_lineage(rankable(false, false, legacy), &mixed),
        Some("clock_quorum")
    );
    // Ineligible rows are never exposed whatever their support.
    let mut denied = rankable(
        true,
        true,
        ClockEvidence {
            write: 2,
            truth: 2,
            task: 2,
            history: 2,
        },
    );
    denied.eligible = false;
    assert_eq!(admit_with_lineage(denied, &mixed), None);
}

#[test]
fn exact_single_source_reports_remain_retrievable() {
    // One direct hard anchor is enough: requiring two witnesses to retrieve a
    // single exact report would destroy recall.
    let hard = vec![Witness::direct(
        WitnessDomain::Anchor,
        "anchor:decision::3",
        "PAY-77",
        3,
    )];
    assert_eq!(
        admit_with_lineage(
            rankable(
                true,
                false,
                ClockEvidence {
                    write: 0,
                    truth: 0,
                    task: 0,
                    history: 0
                }
            ),
            &hard
        ),
        Some("hard_anchor")
    );
    // A hard anchor claimed by the counters but only witnessed through a hop is not hard.
    let derived_hard = vec![Witness::derived(
        WitnessDomain::Hop,
        "hop:decision::1",
        "same_path",
        1,
    )];
    assert_eq!(
        admit_with_lineage(
            rankable(
                true,
                false,
                ClockEvidence {
                    write: 2,
                    truth: 2,
                    task: 0,
                    history: 0
                }
            ),
            &derived_hard
        ),
        None
    );
    // Strong lexical on the row itself admits alone.
    let strong = vec![Witness::direct(
        WitnessDomain::Lexical,
        "lexical:decision::4",
        "\"ledger commit\"",
        2,
    )];
    assert_eq!(
        admit_with_lineage(
            rankable(
                false,
                true,
                ClockEvidence {
                    write: 2,
                    truth: 0,
                    task: 0,
                    history: 0
                }
            ),
            &strong
        ),
        Some("strong_lexical")
    );
}

#[test]
fn why_answers_the_four_questions_and_expansion_is_never_hard() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let caller = Caller {
            owner_id: None,
            agent: "lineage",
            principal: "solo".into(),
        };
        dispatch(
            cx,
            &state,
            Caller { ..caller },
            Operation::Commit,
            &json!({"decision": "LIN-1 outage postmortem filed under src/lin/retry.rs"}),
        )
        .await
        .unwrap();
        dispatch(
            cx,
            &state,
            Caller {
                owner_id: None,
                agent: "lineage",
                principal: "solo".into(),
            },
            Operation::Commit,
            &json!({"decision": "src/lin/retry.rs owns idempotent replay for gateway timeouts"}),
        )
        .await
        .unwrap();
        let payload = execute_unified_recall(
            cx,
            &state,
            "LIN-1",
            320,
            8,
            "lineage",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        let results = payload["results"].as_array().unwrap();
        let seed = results
            .iter()
            .find(|r| r["excerpt"].as_str().unwrap().contains("postmortem"))
            .expect("seed admitted");
        let q = &seed["why"]["questions"];
        assert!(
            q["discovery"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a == "anchor" || a == "lexical"),
            "{q}"
        );
        assert_eq!(q["relevance"], "hard_anchor");
        assert_eq!(q["epistemic"], "asserted");
        assert!(q["coverage"].as_str().unwrap().contains("validAt=current"));
        assert!(q["independentOrigins"].as_u64().unwrap() >= 1);
        let witnesses = seed["why"]["witnesses"].as_array().unwrap();
        assert!(
            witnesses.iter().all(|w| w["derived"] == false),
            "seed is directly witnessed: {witnesses:?}"
        );
        assert!(!results.iter().any(|r| r["excerpt"].as_str().unwrap().contains("idempotent replay")), "the neighbor reached only through the shared path / sibling expansion is not an answer: {payload}");
        // The neighbor may appear as a lead in a non-high-assurance View.
        let view = dispatch(
            cx,
            &state,
            Caller {
                owner_id: None,
                agent: "lineage",
                principal: "solo".into(),
            },
            Operation::Query,
            &json!({"need": "LIN-1", "profile": "map"}),
        )
        .await
        .unwrap();
        let statements: Vec<&str> = view["cards"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["statement"].as_str())
            .collect();
        assert!(
            !statements.iter().any(|s| s.contains("idempotent replay")),
            "{view}"
        );
        let _: &Value = &view;
    });
}

/// Route quotas and the versioned rank tuple: a constraint-kind row outranks
/// lexical noise regardless of recency; lexical crowding is bounded per
/// family and reported as exhaustion; ordering is deterministic across runs.
#[test]
fn constraints_outrank_lexical_noise_and_quota_exhaustion_is_visible() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let caller = || Caller {
            owner_id: None,
            agent: "quota",
            principal: "solo".into(),
        };
        dispatch(cx, &state, caller(), Operation::Commit, &json!({"entries": [{"kind": "constraint", "text": "QUOTA-1 constraint: never ship the gateway without the idempotency header"}]})).await.unwrap();
        // Thirty lexical near-neighbors inserted directly (the store's refine
        // policy would otherwise supersede them into one row); FTS triggers
        // index them, so the lexical family alone sees 31 candidates.
        {
            let conn = state.db.lock(cx).await.unwrap();
            for i in 0..30 {
                conn.execute("INSERT INTO decisions (decision, type, source_agent, status) VALUES (?1, 'decision', 'quota', 'active')", [format!("QUOTA-1 pipeline {i} forwards the idempotency header downstream {i}")]).unwrap();
            }
        }
        let payload = execute_unified_recall(
            cx,
            &state,
            "QUOTA-1 idempotency header",
            4000,
            40,
            "quota",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        let results = payload["results"].as_array().unwrap();
        assert!(
            results[0]["excerpt"]
                .as_str()
                .unwrap()
                .contains("constraint:"),
            "required role ranks first, not the newest log line: {}",
            results[0]
        );
        let routes = &payload["routes"];
        assert_eq!(routes["rank_tuple"], "rank/2");
        assert!(
            routes["collected"]["lexical"].as_u64().unwrap() >= 1,
            "{routes}"
        );
        assert!(
            routes["exhausted"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "lexical" || f == "anchor"),
            "31 near-duplicates must exhaust a family quota, visibly: {routes}"
        );
        let again = execute_unified_recall(
            cx,
            &state,
            "QUOTA-1 idempotency header",
            4000,
            40,
            "quota",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        let order_a: Vec<&str> = results
            .iter()
            .filter_map(|r| r["source"].as_str())
            .collect();
        let order_b: Vec<&str> = again["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|r| r["source"].as_str())
            .collect();
        assert_eq!(order_a, order_b, "deterministic ordering across runs");
        let view = dispatch(
            cx,
            &state,
            caller(),
            Operation::Query,
            &json!({"need": "QUOTA-1 idempotency header", "profile": "map", "budget": 8000}),
        )
        .await
        .unwrap();
        assert!(
            view["coverage"]["partitions"]["routes"]["exhausted"]
                .as_array()
                .unwrap()
                .len()
                >= 1,
            "exhaustion travels into the View watermark: {}",
            view["coverage"]
        );
        assert_eq!(
            view["coverage"]["partitions"]["decisions"]["limits_hit"],
            true
        );
    });
}

/// Strong matches are qualified by namespace: a relative path is identity
/// only inside the task's repository root; a row whose own paths live under
/// a foreign root is a lead, not a hard anchor. Source/author facts are never
/// minted from text inside a memory.
#[test]
fn path_identity_is_scoped_to_the_repository_root() {
    cortex_tests::support::run_with_cx(|cx| async move {
        let cx = &cx;
        let state = solo_state();
        let caller = || Caller {
            owner_id: None,
            agent: "scope",
            principal: "solo".into(),
        };
        dispatch(cx, &state, caller(), Operation::Commit, &json!({"decision": "Fixed retry loop in /Users/x/repoa/src/lib.rs (src/lib.rs) for gateway timeouts"})).await.unwrap();
        dispatch(cx, &state, caller(), Operation::Commit, &json!({"decision": "Fixed cache bug in /Users/x/repob/src/lib.rs (src/lib.rs) for ledger reads"})).await.unwrap();
        let mut ctx = RecallContext::solo();
        ctx.paths = vec!["/Users/x/repoa/src".into(), "/Users/x/repoa/tests".into()];
        let payload = execute_unified_recall(cx, &state, "src/lib.rs", 400, 8, "scope", &ctx, None)
            .await
            .unwrap();
        let results = payload["results"].as_array().unwrap();
        let local = results
            .iter()
            .find(|r| r["excerpt"].as_str().unwrap().contains("repoa"))
            .expect("same-root row admitted: {payload}");
        assert_eq!(
            local["why"]["questions"]["relevance"], "hard_anchor",
            "{payload}"
        );
        if let Some(foreign) = results
            .iter()
            .find(|r| r["excerpt"].as_str().unwrap().contains("repob"))
        {
            assert_ne!(
                foreign["why"]["questions"]["relevance"], "hard_anchor",
                "foreign-root path is not identity: {payload}"
            );
            let witnesses = foreign["why"]["witnesses"].as_array().unwrap();
            assert!(
                witnesses.iter().any(|w| w["matchedKey"]
                    .as_str()
                    .or(w["matched_key"].as_str())
                    .unwrap_or("")
                    .starts_with("demoted:path_outside_root")),
                "{witnesses:?}"
            );
        }
        // Namespace ignorance never demotes: without task paths both rows stay hard.
        let payload = execute_unified_recall(
            cx,
            &state,
            "src/lib.rs",
            400,
            8,
            "scope",
            &RecallContext::solo(),
            None,
        )
        .await
        .unwrap();
        for r in payload["results"].as_array().unwrap() {
            assert_eq!(
                r["why"]["questions"]["relevance"], "hard_anchor",
                "{payload}"
            );
        }
        // Text claiming a source/author never becomes a Source anchor.
        let anchors = cortex_logic::clockwork::extract_anchors(
            "source_agent: claude-opus model: gpt-5 wrote memory::12",
            &[],
            32,
        );
        assert!(
            anchors
                .iter()
                .all(|a| a.kind != cortex_logic::clockwork::AnchorKind::Source),
            "{anchors:?}"
        );
    });
}

fn qa(kind: cortex_logic::clockwork::AnchorKind, v: &str) -> cortex_logic::clockwork::QueryAnchor {
    cortex_logic::clockwork::QueryAnchor {
        kind,
        value: v.into(),
        specificity: 3,
    }
}

#[test]
fn repo_root_is_the_shared_prefix_of_task_paths() {
    use cortex_logic::clockwork::AnchorScope;
    let s = AnchorScope::from_query(
        &[
            "/Users/x/repoa/src/lib.rs".into(),
            "/Users/x/repoa/tests/a.rs".into(),
        ],
        &[],
    );
    assert_eq!(s.repo_root.as_deref(), Some("users/x/repoa"));
    let single = AnchorScope::from_query(&["/Users/x/repoa/src".into()], &[]);
    assert_eq!(single.repo_root.as_deref(), Some("users/x/repoa"));
    assert_eq!(
        AnchorScope::from_query(&["src".into()], &[]).repo_root,
        None
    );
}

#[test]
fn relative_path_inside_a_foreign_root_is_not_identity() {
    use cortex_logic::clockwork::{qualify_matches, AnchorKind, AnchorScope, RowNamespace};
    let scope = AnchorScope::from_query(&["/Users/x/repoa/src".into()], &[]);
    let foreign = RowNamespace {
        paths: vec!["users/x/repob/src/lib.rs".into(), "src/lib.rs".into()],
        hosts: vec![],
    };
    let (q, why) = qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &foreign, &scope);
    assert_eq!(q[0].specificity, 2, "{why:?}");
    let local = RowNamespace {
        paths: vec!["users/x/repoa/src/lib.rs".into(), "src/lib.rs".into()],
        hosts: vec![],
    };
    assert_eq!(
        qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &local, &scope).0[0].specificity,
        3
    );
    let unknown = RowNamespace {
        paths: vec!["src/lib.rs".into()],
        hosts: vec![],
    };
    assert_eq!(
        qualify_matches(&[qa(AnchorKind::Path, "src/lib.rs")], &unknown, &scope).0[0].specificity,
        3,
        "ignorance never demotes"
    );
}

#[test]
fn bare_aliases_and_foreign_issuers_are_demoted() {
    use cortex_logic::clockwork::{qualify_matches, AnchorKind, AnchorScope, RowNamespace};
    let scope = AnchorScope {
        repo_root: None,
        issuer_hosts: ["jira.example.com".to_string()].into_iter().collect(),
    };
    let row = RowNamespace {
        paths: vec![],
        hosts: vec!["linear.app".into()],
    };
    let (q, why) = qualify_matches(
        &[
            qa(AnchorKind::Path, "main"),
            qa(AnchorKind::Ticket, "pay-77"),
        ],
        &row,
        &scope,
    );
    assert_eq!(q[0].specificity, 2);
    assert_eq!(q[1].specificity, 2);
    assert_eq!(why.len(), 2);
    let same = RowNamespace {
        paths: vec![],
        hosts: vec!["jira.example.com".into()],
    };
    assert_eq!(
        qualify_matches(&[qa(AnchorKind::Ticket, "pay-77")], &same, &scope).0[0].specificity,
        3
    );
}
