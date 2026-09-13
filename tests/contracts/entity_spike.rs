use cortex_kernel::runtime::{CortexRuntime, LensInput};
use cortex_kernel::graph::resolve_query;
use cortex_tests::support::{run_with_cx, solo_state};

async fn resolve(cx: &asupersync::Cx, runtime: &CortexRuntime, query: &str) -> Vec<(i64, String, String)> {
    let conn = runtime.state().db.lock(cx).await.unwrap();
    resolve_query(&conn, query).into_iter().map(|id| {
        conn.query_row("SELECT id, qualifier, kind FROM entities WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).expect("resolved entity")
    }).collect()
}

// HTTP entities/recall status and URL encoding retired; graph identity and CQR attribution remain.
#[test]
fn aliases_resolve_to_one_entity_without_an_llm() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        for text in [
            "The auth service issues OAuth2 bearer credentials for the dashboard",
            "OAuth microservice rotates signing keys weekly without downtime",
            "The payments service handles refunds through the ledger queue",
        ] {
            assert!(runtime.deposit(&cx, text, text, "entity-spike", None).await.unwrap().target_id.is_some());
        }
        let auth = resolve(&cx, &runtime, "auth service").await;
        assert_eq!(auth.len(), 1);
        let auth_id = auth[0].0;
        assert_eq!(auth[0].1, "auth");
        assert_eq!(auth[0].2, "service");
        for alias in ["OAuth microservice", "login system"] {
            let resolved = resolve(&cx, &runtime, alias).await;
            assert_eq!(resolved.len(), 1);
            assert_eq!(resolved[0].0, auth_id);
        }
        let payments = resolve(&cx, &runtime, "payments service").await;
        assert_eq!(payments.len(), 1);
        assert_ne!(payments[0].0, auth_id);
        assert_eq!(payments[0].1, "payments");
        for _ in 0..2 {
            assert!(resolve(&cx, &runtime, "warp drive assembly").await.is_empty(), "queries must not create entities");
        }
        let filler = ["the"; 40].join(" ");
        let late = resolve(&cx, &runtime, &format!("{filler} payments")).await;
        assert_eq!(late.len(), 1, "stop-word prefix must not consume the query token cap");
        assert_eq!(late[0].1, "payments");
    });
}

#[test]
fn entity_arm_recalls_alias_row_with_zero_keyword_overlap() {
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::from_state(solo_state());
        const DECISION: &str = "The auth service migrated to argon2 hashing yesterday";
        runtime.deposit(&cx, "alias", DECISION, "entity-spike", None).await.unwrap();
        let recall = runtime.lens(&cx, LensInput { query: "login system".into(), k: 5, budget: 600, ..Default::default() }).await.unwrap();
        let hit = recall["results"].as_array().unwrap().iter().find(|r| r["excerpt"] == DECISION).expect("exact alias row recalled");
        assert_eq!(hit["method"], "clock-quorum");
        assert_eq!(hit["why"]["hardAnchor"], true);
        assert!(hit["why"]["anchors"].as_array().unwrap().iter().any(|a| a["kind"] == "entity"), "{hit}");
    });
}
