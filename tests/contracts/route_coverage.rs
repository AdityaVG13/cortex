//! Retired: HTTP route registration/status/header/query contracts, feed/import,
//! focus lifecycle, conductor minted-UUID/no-effect facades, and removed route
//! aliases. Local permissions retain their actual owner-scoped database effects;
//! semantic recall retains model-free retrieval and token-budget accounting.
use cortex_daemon::handlers::{mutate::{grant_permission, list_permissions, revoke_permission}, operations::{dispatch, Caller, Operation}, recall::{execute_semantic_recall, RecallContext}};
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::json;

#[test]
fn local_permissions_roundtrip_is_owner_scoped() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let conn = state.db.lock(&cx).await.unwrap();
        assert!(list_permissions(&conn, 1).unwrap().is_empty());
        grant_permission(&conn, 1, "rcov-client", "read", "memory:*", "rcov-admin").unwrap();
        let grants = list_permissions(&conn, 1).unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["client"], "rcov-client");
        assert_eq!(grants[0]["permission"], "read");
        assert_eq!(grants[0]["scope"], "memory:*");
        assert_eq!(grants[0]["grantedBy"], "rcov-admin");
        assert!(grants[0]["grantedAt"].is_string());
        assert!(list_permissions(&conn, 2).unwrap().is_empty());
        assert_eq!(revoke_permission(&conn, 2, "rcov-client", "read", "memory:*").unwrap(), 0);
        assert_eq!(list_permissions(&conn, 1).unwrap().len(), 1);
        assert_eq!(revoke_permission(&conn, 1, "rcov-client", "read", "memory:*").unwrap(), 1);
        assert_eq!(revoke_permission(&conn, 1, "rcov-client", "read", "memory:*").unwrap(), 0);
        assert!(list_permissions(&conn, 1).unwrap().is_empty());
    });
}

#[test]
fn local_semantic_recall_preserves_model_free_engine_and_budget() {
    run_with_cx(|cx| async move {
        let state = solo_state();
        let text = "RCOV_ROUTE_COVERAGE_MARKER_SEMANTIC deterministic route-coverage probe decision";
        dispatch(&cx, &state, Caller { owner_id: None, agent: "rcov", principal: "solo".into() }, Operation::Commit, &json!({"decision":text})).await.unwrap();
        let context = RecallContext::from_caller(None, &state);
        let result = execute_semantic_recall(&cx, &state, "RCOV_ROUTE_COVERAGE_MARKER_SEMANTIC", 200, 10, "rcov", &context, None).await.unwrap();
        assert_eq!(result["semanticRoute"], json!({"engine":"clock-quorum","modelFree":true}));
        assert_eq!(result["budget"], 200);
        let hit = result["results"].as_array().unwrap().iter().find(|item| item["excerpt"] == text).expect("stored decision recalled");
        assert_eq!(hit["method"], "clock-quorum");
        let spent = result["spent"].as_u64().unwrap();
        let saved = result["saved"].as_i64().unwrap();
        assert_eq!(saved, 200 - spent as i64);
        assert_eq!(result["tokenUsageLine"], format!("Cortex recall used {spent} tokens and saved {saved} of 200 budget."));
    });
}
