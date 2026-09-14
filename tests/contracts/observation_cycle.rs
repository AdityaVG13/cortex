use cortex_kernel::runtime::{
    CortexRuntime,
    cycle::NeedSpec,
    observation::{ObservationEvent, SourceSpec},
};
use cortex_tests::support::run_with_cx;

fn need() -> NeedSpec {
    NeedSpec {
        id: "active".into(),
        scope: "repo".into(),
        cues: vec!["retry".into()],
        exclude_cues: vec![],
        max_results: 128,
        max_bytes: 65536,
        ttl_seconds: 3600,
        learned: false,
    }
}
#[test]
fn ready_literal_pull_does_not_wait_for_an_unrelated_writer() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("brain.db");
        let runtime = CortexRuntime::open_db(&path).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        let receipt = runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "one".into(),
                    text: "retry evidence".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        runtime
            .state()
            .db
            .lock(&cx)
            .await
            .unwrap()
            .busy_timeout(std::time::Duration::ZERO)
            .unwrap();
        let mut other = rusqlite::Connection::open(&path).unwrap();
        let writer = other
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let pull = runtime
            .query_observations(&cx, "repo", "retry", 128, 65536, false)
            .await
            .unwrap();
        assert_eq!(pull.status, "ready");
        assert_eq!(pull.source_refs, vec![receipt.source_id]);
        writer.rollback().unwrap();
    });
}
#[test]
fn ready_literal_pull_does_not_write_subscription_state() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "one".into(),
                    text: "retry requires idempotency".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let active = runtime.subscribe_observations(&cx, need()).await.unwrap();
        let changes = {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            conn.execute_batch("CREATE TEMP TRIGGER reject_pull_registration BEFORE INSERT ON observation_needs BEGIN SELECT RAISE(ABORT,'pull attempted subscription write'); END;").unwrap();
            conn.total_changes()
        };
        let pull = runtime
            .query_observations(&cx, "repo", "retry", 128, 65536, false)
            .await
            .unwrap();
        assert_eq!(pull.status, "ready");
        assert_eq!(pull.source_refs, active.source_refs);
        assert_eq!(pull.payload, active.payload);
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        assert_eq!(
            conn.total_changes(),
            changes,
            "ready literal pull must not write or churn subscriptions"
        );
        let needs: i64 = conn
            .query_row("SELECT count(*) FROM observation_needs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(needs, 1, "the existing subscription must remain intact");
    });
}

#[test]
fn capture_updates_reverse_need_index_in_its_transaction() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime.subscribe_observations(&cx, need()).await.unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "one".into(),
                    text: "retry requires idempotency".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM observation_matches WHERE need_id='active'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "capture must maintain already active reverse subscriptions before acknowledgement"
        );
    });
}

#[test]
fn delta_views_equal_full_recomputation_and_presence_never_survives_changes() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("foreign", "elsewhere"))
            .await
            .unwrap();
        runtime.subscribe_observations(&cx, need()).await.unwrap();
        let mut expected = std::collections::BTreeSet::new();
        let mut sources = Vec::new();
        for i in 0..80 {
            let text = if i % 3 == 0 {
                "retry exception"
            } else {
                "unrelated artifact"
            };
            let receipt = runtime
                .observe(
                    &cx,
                    "notes",
                    "g",
                    ObservationEvent {
                        event_key: i.to_string(),
                        text: text.into(),
                        observed_at: None,
                    },
                )
                .await
                .unwrap();
            if i % 3 == 0 {
                expected.insert(receipt.source_id.clone());
            }
            sources.push(receipt.source_id);
            if i % 7 == 0 {
                let id = &sources[i / 2];
                runtime
                    .retract_observation(&cx, id, "withdrawn")
                    .await
                    .unwrap();
                expected.remove(id);
            }
            let view = runtime
                .prepare_observations(&cx, "active", "context-1", None)
                .await
                .unwrap();
            assert_eq!(view.status, "ready");
            assert_eq!(
                view.evidence
                    .iter()
                    .map(|e| e.source_id.clone())
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
        }
        let foreign = runtime
            .observe(
                &cx,
                "foreign",
                "g",
                ObservationEvent {
                    event_key: "f".into(),
                    text: "retry foreign secret".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let initial = runtime
            .prepare_observations(&cx, "active", "context-1", None)
            .await
            .unwrap();
        assert!(
            initial
                .evidence
                .iter()
                .all(|e| e.source_id != foreign.source_id)
        );
        let present = runtime
            .prepare_observations(&cx, "active", "context-1", initial.delivery_id.as_deref())
            .await
            .unwrap();
        assert_eq!(present.payload_bytes, 0);
        let compacted = runtime
            .prepare_observations(&cx, "active", "context-2", initial.delivery_id.as_deref())
            .await
            .unwrap();
        assert!(compacted.payload_bytes > 0);
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "new-exception".into(),
                    text: "retry must stop on revoked permit".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let changed = runtime
            .prepare_observations(&cx, "active", "context-1", initial.delivery_id.as_deref())
            .await
            .unwrap();
        assert!(changed.payload.contains("revoked permit"));
        assert_ne!(initial.fingerprint, changed.fingerprint);
        runtime
            .set_source_enabled(&cx, "notes", false)
            .await
            .unwrap();
        let revoked = runtime
            .prepare_observations(&cx, "active", "context-1", changed.delivery_id.as_deref())
            .await
            .unwrap();
        assert!(revoked.evidence.is_empty());
        assert_eq!(revoked.payload_bytes, 0);
    });
}

#[test]
fn bounded_views_disclose_incomplete_results_instead_of_delivering_prefixes() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "a".into(),
                    text: "retry long evidence must remain exact".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let mut spec = need();
        spec.max_bytes = 5;
        let view = runtime.subscribe_observations(&cx, spec).await.unwrap();
        assert_eq!(view.status, "quota_blocked");
        assert_eq!(view.payload_bytes, 0);
        assert!(view.evidence.is_empty());
        assert!(
            view.source_refs.is_empty(),
            "quota_blocked must not name overflowing sources for later expand: {:?}",
            view.source_refs
        );
    });
}

#[test]
fn required_child_blocks_ready_until_it_is_available() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        let parent = runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "rule".into(),
                    text: "retry unless revoked".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let child = runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "exception".into(),
                    text: "revoked permit exception".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        runtime
            .require_observation(&cx, &parent.source_id, &child.source_id)
            .await
            .unwrap();
        let ready = runtime.subscribe_observations(&cx, need()).await.unwrap();
        assert_eq!(ready.status, "ready");
        assert!(ready.evidence.iter().any(|item| item.route == "required"));
        runtime
            .retract_observation(&cx, &child.source_id, "withdrawn")
            .await
            .unwrap();
        let blocked = runtime
            .prepare_observations(&cx, "active", "context-1", None)
            .await
            .unwrap();
        assert_eq!(blocked.status, "qualification_unavailable");
        assert!(blocked.payload.is_empty());
    });
}

#[test]
fn exclude_cues_remove_candidates_without_deleting_sources() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes", "repo"))
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "keep".into(),
                    text: "retry ledger".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let foreign = runtime
            .observe(
                &cx,
                "notes",
                "g",
                ObservationEvent {
                    event_key: "drop".into(),
                    text: "retry foreign secret".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();
        let mut spec = need();
        spec.exclude_cues = vec!["foreign".into()];
        let view = runtime.subscribe_observations(&cx, spec).await.unwrap();
        assert_eq!(view.status, "ready");
        assert!(
            view.evidence
                .iter()
                .all(|item| item.source_id != foreign.source_id)
        );
        assert_eq!(
            runtime
                .read_observation(&cx, &foreign.source_id)
                .await
                .unwrap()
                .text,
            "retry foreign secret"
        );
    });
}

const REPO_A: &str = "/Users/x/repoa";
const REPO_B: &str = "/Users/x/repob";

#[test]
fn path_scoped_pull_stays_in_repository_and_default_project_does_not_leak() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes-a", REPO_A))
            .await
            .unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("notes-b", REPO_B))
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
                "g",
                ObservationEvent {
                    event_key: "a".into(),
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
                "g",
                ObservationEvent {
                    event_key: "b".into(),
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
                "g",
                ObservationEvent {
                    event_key: "u".into(),
                    text: "PAY-OBS-1 tool reported signing key rotation".into(),
                    observed_at: None,
                },
            )
            .await
            .unwrap();

        let project = runtime
            .query_observations(&cx, "project", "PAY-OBS-1 tool reported", 32, 65536, false)
            .await
            .unwrap();
        assert!(
            project.source_refs.contains(&unscoped.source_id),
            "unscoped project observation must remain on the exact project pull: {project:?}"
        );
        assert!(
            !project.source_refs.contains(&in_a.source_id),
            "path-scoped observation must not leak into an exact project pull: {project:?}"
        );

        let paths_a = vec![REPO_A.to_string()];
        let scoped = runtime
            .query_observations_for_paths(
                &cx,
                "PAY-OBS-1 tool reported",
                &paths_a,
                None,
                32,
                65536,
                false,
            )
            .await
            .unwrap();
        assert!(
            scoped.source_refs.contains(&in_a.source_id),
            "path pull must return this repo: {scoped:?}"
        );
        assert!(
            scoped.source_refs.contains(&unscoped.source_id),
            "unscoped project observations stay visible under a named root: {scoped:?}"
        );
        assert!(
            !scoped.source_refs.contains(&in_b.source_id),
            "path pull must not return a sibling repo: {scoped:?}"
        );

        let nested = runtime
            .query_observations_for_paths(
                &cx,
                "PAY-OBS-1 tool reported",
                &[format!("{REPO_A}/src")],
                None,
                32,
                65536,
                false,
            )
            .await
            .unwrap();
        assert!(
            nested.source_refs.contains(&in_a.source_id),
            "a nested path under the registered root must still retrieve it: {nested:?}"
        );

        let recent = runtime
            .recent_observations_for_paths(&cx, &paths_a, None, 8, 16 * 1024)
            .await
            .unwrap();
        assert!(
            recent.source_refs.contains(&in_a.source_id),
            "recent-in-scope must surface this repo without cue overlap: {recent:?}"
        );
        assert!(
            !recent.source_refs.contains(&in_b.source_id),
            "recent-in-scope must not surface a sibling repo: {recent:?}"
        );
    });
}
