//! Recall truth is a kernel contract; URL encoding and HTTP status checks retired.
use cortex_kernel::handlers::store::store_decision_with_ttl;
use cortex_kernel::{runtime::LensInput, CortexRuntime};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::json;

const AGENT: &str = "recall-truth-agent";
const DECISION_A: &str = "We are using Redis for caching";
const DECISION_B: &str =
    "We are not using Redis for caching, we moved off Redis to rediska last sprint";
const QUERY: &str = "what do we use for caching";

#[test]
fn recall_truth_supersede_and_why() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        {
            let mut conn = runtime.state().db.lock(&cx).await.unwrap();
            for (decision, confidence) in [(DECISION_A, 0.85), (DECISION_B, 0.95)] {
                let (entry, _) = store_decision_with_ttl(
                    &mut conn,
                    decision,
                    None,
                    Some("decision".into()),
                    AGENT.into(),
                    Some(confidence),
                    None,
                    None,
                )
                .unwrap();
                assert_eq!(entry["action"], "inserted");
            }
            // Explicit fixture state: this proves recall filtering, not automatic supersession.
            assert_eq!(conn.execute("UPDATE decisions SET status = 'superseded', updated_at = datetime('now') WHERE decision = ?1 AND status = 'active'", [DECISION_A]).unwrap(), 1);
        }
        let recalled = runtime
            .lens(
                &cx,
                LensInput {
                    query: QUERY.into(),
                    budget: 320,
                    k: 10,
                    agent: AGENT.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let results = recalled["results"].as_array().expect("results");
        assert!(!results.is_empty(), "{recalled}");
        assert!(
            results.iter().any(|item| item["excerpt"] == DECISION_B),
            "{results:?}"
        );
        for item in results {
            assert_ne!(item["excerpt"], DECISION_A);
            assert!(item["why"].is_object());
        }
        let redis = runtime
            .lens(
                &cx,
                LensInput {
                    query: "Redis caching".into(),
                    budget: 320,
                    k: 10,
                    agent: AGENT.into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for item in redis["results"].as_array().unwrap() {
            assert_ne!(item["excerpt"], DECISION_A);
        }
        let why = &results[0]["why"];
        let mut keys: Vec<_> = why
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "admittedBy",
                "anchors",
                "clockVotes",
                "engine",
                "filters",
                "hardAnchor",
                "links",
                "questions",
                "tieBreak",
                "witnesses"
            ]
        );
        assert_eq!(why["engine"], json!("clock-quorum"));
        assert!(why["admittedBy"].is_string());
        assert!(why["clockVotes"].is_object());
        let mut filters: Vec<_> = why["filters"]["statusFilters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        filters.sort();
        assert_eq!(filters, ["archived", "superseded"]);
    });
}
