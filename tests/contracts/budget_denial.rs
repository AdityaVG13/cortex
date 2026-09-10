//! Budget admission is exercised directly with explicit, per-test configuration.
//! Retired: HTTP 429, Retry-After/Cache-Control headers, and the removed router's
//! JSON-RPC -32029 mapping. The MCP dispatcher does not enforce that router budget.
use cortex_daemon::budgets::{BudgetConfigStatus, BudgetEndpoint};
use cortex_daemon::rate_limit::RateLimiter;
use cortex_tests::support::run_with_cx;

#[test]
fn configured_store_and_mcp_budgets_deny_persistently() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("budgets.toml"), "[defaults]\nenabled = true\n[endpoints.store]\nlimit = 2\nwindow_seconds = 3600\n[endpoints.mcp]\nlimit = 2\nwindow_seconds = 3600\n").unwrap();
        let status = BudgetConfigStatus::load_from_home(home.path());
        assert!(status.config_loaded && status.error.is_none());
        let limiter = RateLimiter::new_with_budget_status(status);
        let ip = "127.0.0.1".parse().unwrap();
        for endpoint in [BudgetEndpoint::Store, BudgetEndpoint::Mcp] {
            for remaining in [1, 0] {
                let decision = limiter
                    .check_budget_for_endpoint(&cx, ip, endpoint)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(decision.allowed);
                assert_eq!(decision.remaining, Some(remaining));
                assert_eq!(decision.retry_after_seconds, 0);
            }
            for _ in 0..2 {
                let decision = limiter
                    .check_budget_for_endpoint(&cx, ip, endpoint)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(!decision.allowed);
                assert_eq!(decision.endpoint, endpoint);
                assert_eq!(decision.limit, 2);
                assert_eq!(decision.window_seconds, 3600);
                assert_eq!(decision.remaining, Some(0));
                assert!([3600, 3599, 3598].contains(&decision.retry_after_seconds));
            }
        }
        assert_eq!(limiter.total_budget_denials(), 4);
        assert_eq!(limiter.recent_budget_denials(&cx).await.unwrap(), 4);
        assert!(limiter
            .check_budget_for_endpoint(&cx, ip, BudgetEndpoint::Recall)
            .await
            .unwrap()
            .is_none());
    });
}

#[test]
fn missing_configuration_does_not_invent_a_budget() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let status = BudgetConfigStatus::load_from_home(home.path());
        assert!(!status.config_loaded);
        let limiter = RateLimiter::new_with_budget_status(status);
        for endpoint in BudgetEndpoint::all() {
            assert!(limiter
                .check_budget_for_endpoint(&cx, "127.0.0.1".parse().unwrap(), *endpoint)
                .await
                .unwrap()
                .is_none());
        }
        assert_eq!(limiter.total_budget_denials(), 0);
    });
}
