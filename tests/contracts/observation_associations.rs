//! Mechanism contracts, not held-out retrieval-quality evidence.
use cortex_kernel::runtime::{
    CortexRuntime,
    observation::{ObservationEvent, ObservationRole, SourceSpec},
};
use cortex_tests::support::run_with_cx;

fn event(key: &str, text: &str) -> ObservationEvent {
    ObservationEvent {
        event_key: key.into(),
        text: text.into(),
        observed_at: None,
    }
}

#[test]
fn learned_routes_are_scoped_explainable_reversible_and_permission_gated() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        for key in ["support-a", "support-b", "target"] {
            runtime
                .register_source(&cx, SourceSpec::document(key, "repo-a"))
                .await
                .unwrap();
        }
        let a = runtime
            .observe(
                &cx,
                "support-a",
                "g1",
                event("one", "zephyr codename first"),
            )
            .await
            .unwrap();
        let b = runtime
            .observe(
                &cx,
                "support-b",
                "g1",
                event("one", "zephyr codename second"),
            )
            .await
            .unwrap();
        let target = runtime
            .observe(
                &cx,
                "target",
                "g1",
                event("one", "codename deployment instructions"),
            )
            .await
            .unwrap();
        let cues = vec!["zephyr".to_owned()];
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        let premature = runtime
            .query_observations(&cx, "repo-a", "zephyr", 32, 65536, true)
            .await
            .unwrap();
        assert!(
            premature
                .evidence
                .iter()
                .all(|e| e.route != "learned_local_association"),
            "learned=true must not refresh associations before rebuild_associations"
        );
        assert_eq!(
            runtime.rebuild_associations(&cx, "repo-a").await.unwrap(),
            3
        );
        let routes = runtime
            .explain_associations(&cx, "repo-a", &cues, 64)
            .await
            .unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].source_id, target.source_id);
        assert_eq!(routes[0].route, "learned_local_association");
        assert_eq!(routes[0].alias, "codename");
        let mut support = vec![a.source_id.clone(), b.source_id.clone()];
        support.sort();
        assert_eq!(routes[0].support_sources, support);
        use cortex_kernel::runtime::associations::AssociationAssessment;
        let baseline = routes[0].score;
        assert!(
            runtime
                .record_association_feedback(
                    &cx,
                    "repo-a",
                    "outcome-1",
                    &target.source_id,
                    AssociationAssessment::Useful
                )
                .await
                .unwrap()
        );
        assert!(
            !runtime
                .record_association_feedback(
                    &cx,
                    "repo-a",
                    "outcome-1",
                    &target.source_id,
                    AssociationAssessment::Useful
                )
                .await
                .unwrap()
        );
        assert!(
            runtime
                .record_association_feedback(
                    &cx,
                    "repo-a",
                    "outcome-1",
                    &target.source_id,
                    AssociationAssessment::Harmful
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()[0]
                .score
                > baseline
        );
        let literal = runtime
            .query_observations(&cx, "repo-a", "zephyr", 32, 65536, false)
            .await
            .unwrap();
        assert!(
            literal
                .evidence
                .iter()
                .all(|e| e.source_id != target.source_id)
        );
        let learned = runtime
            .query_observations(&cx, "repo-a", "zephyr", 32, 65536, true)
            .await
            .unwrap();
        assert!(
            learned
                .evidence
                .iter()
                .any(|e| e.source_id == target.source_id && e.route == "learned_local_association")
        );
        runtime
            .retract_association_feedback(&cx, "repo-a", "outcome-1")
            .await
            .unwrap();
        assert_eq!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()[0]
                .score,
            baseline
        );
        assert!(
            runtime
                .explain_associations(&cx, "repo-b", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 0)
                .await
                .unwrap()
                .is_empty()
        );
        runtime
            .set_source_enabled(&cx, "support-a", false)
            .await
            .unwrap();
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        runtime
            .set_source_enabled(&cx, "support-a", true)
            .await
            .unwrap();
        assert_eq!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap(),
            routes
        );
        runtime
            .set_source_enabled(&cx, "target", false)
            .await
            .unwrap();
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        runtime
            .set_source_enabled(&cx, "target", true)
            .await
            .unwrap();
        runtime.reset_associations(&cx, "repo-a").await.unwrap();
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            runtime
                .read_observation(&cx, &target.source_id)
                .await
                .unwrap()
                .text,
            "codename deployment instructions"
        );
        runtime.rebuild_associations(&cx, "repo-a").await.unwrap();
        assert_eq!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap(),
            routes
        );
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS observation_retractions(source_id TEXT PRIMARY KEY,principal TEXT NOT NULL,reason TEXT NOT NULL);").unwrap();
        conn.execute(
            "INSERT INTO observation_retractions VALUES(?1,'local','correction')",
            [&a.source_id],
        )
        .unwrap();
        drop(conn);
        assert!(
            runtime
                .explain_associations(&cx, "repo-a", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
    });
}

#[test]
fn copied_text_same_lineage_and_delivery_do_not_reinforce() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        for key in ["original", "copy", "target"] {
            runtime
                .register_source(&cx, SourceSpec::document(key, "repo"))
                .await
                .unwrap();
        }
        for (key, role) in [
            ("delivery", ObservationRole::DeliveryOnly),
            ("agent", ObservationRole::AgentAssertion),
            ("tool", ObservationRole::ToolReport),
        ] {
            runtime
                .register_source(
                    &cx,
                    SourceSpec {
                        key: key.into(),
                        scope: "repo".into(),
                        role,
                        max_bytes: 65536,
                    },
                )
                .await
                .unwrap();
            runtime
                .observe(
                    &cx,
                    key,
                    "g1",
                    event("one", &format!("zephyr codename {key}")),
                )
                .await
                .unwrap();
        }
        runtime
            .observe(&cx, "original", "g1", event("one", "zephyr codename first"))
            .await
            .unwrap();
        runtime
            .observe(&cx, "copy", "g1", event("one", "zephyr codename first"))
            .await
            .unwrap();
        runtime
            .observe(&cx, "target", "g1", event("one", "codename instructions"))
            .await
            .unwrap();
        runtime.rebuild_associations(&cx, "repo").await.unwrap();
        let cues = vec!["zephyr".to_owned()];
        assert!(
            runtime
                .explain_associations(&cx, "repo", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        runtime
            .set_source_enabled(&cx, "copy", false)
            .await
            .unwrap();
        runtime
            .observe(
                &cx,
                "original",
                "g2",
                event("two", "zephyr codename revised"),
            )
            .await
            .unwrap();
        runtime.rebuild_associations(&cx, "repo").await.unwrap();
        assert!(
            runtime
                .explain_associations(&cx, "repo", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
    });
}

#[test]
fn stale_policy_and_erased_support_invalidate_without_rebuild() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        for (key, text) in [
            ("a", "zephyr codename first"),
            ("b", "zephyr codename second"),
            ("c", "codename instructions"),
        ] {
            runtime
                .register_source(&cx, SourceSpec::document(key, "repo"))
                .await
                .unwrap();
            runtime
                .observe(&cx, key, "g1", event("one", text))
                .await
                .unwrap();
        }
        runtime.rebuild_associations(&cx, "repo").await.unwrap();
        let cues = vec!["zephyr".to_owned()];
        assert_eq!(
            runtime
                .explain_associations(&cx, "repo", &cues, 64)
                .await
                .unwrap()
                .len(),
            1
        );
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        conn.execute(
            "UPDATE sources SET availability='erased',inline_payload=NULL WHERE origin_id='a'",
            [],
        )
        .unwrap();
        drop(conn);
        assert!(
            runtime
                .explain_associations(&cx, "repo", &cues, 64)
                .await
                .unwrap()
                .is_empty()
        );
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        conn.execute(
            "UPDATE brain_meta SET policy_epoch='changed' WHERE singleton=1",
            [],
        )
        .unwrap();
        drop(conn);
        assert_eq!(runtime.rebuild_associations(&cx, "repo").await.unwrap(), 0);
    });
}
