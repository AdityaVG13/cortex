// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    use std::net::Ipv4Addr;
    #[tokio::test]
    async fn test_request_limit_allows_under_limit() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        for _ in 0..99 {
            assert!(rl.check_request(ip).await.is_ok());
        }
    }
    #[tokio::test]
    async fn test_request_limit_blocks_at_limit() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7));
        let limit = rl.request_limit_for_ip_class(ip, RequestClass::Default);
        for _ in 0..limit {
            let _ = rl.check_request(ip).await;
        }
        assert!(rl.check_request(ip).await.is_err());
    }
    #[tokio::test]
    async fn test_loopback_has_higher_request_limit_than_non_loopback() {
        let rl = RateLimiter::new();
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let remote = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
        assert!(rl.request_limit_for_ip_class(loopback, RequestClass::Default) > rl.request_limit_for_ip_class(remote, RequestClass::Default));
    }
    #[tokio::test]
    async fn test_auth_failure_blocks_after_limit() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for _ in 0..AUTH_FAIL_LIMIT {
            let _ = rl.record_auth_failure(ip).await;
        }
        assert!(rl.is_auth_blocked(&ip).await.is_some());
    }
    #[tokio::test]
    async fn test_different_ips_independent() {
        let rl = RateLimiter::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
        let limit = rl.request_limit_for_ip_class(ip1, RequestClass::Default);
        for _ in 0..limit {
            let _ = rl.check_request(ip1).await;
        }
        assert!(rl.check_request(ip1).await.is_err());
        assert!(rl.check_request(ip2).await.is_ok());
    }
    #[tokio::test]
    async fn test_route_class_buckets_are_independent() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 42));
        let store_limit = rl.request_limit_for_ip_class(ip, RequestClass::Store);
        for _ in 0..store_limit {
            let _ = rl.check_request_for_class(ip, RequestClass::Store).await.expect("store class should allow requests below class limit");
        }
        assert!(
            rl.check_request_for_class(ip, RequestClass::Store).await.is_err(),
            "store class should rate limit once its own bucket is exhausted"
        );
        assert!(
            rl.check_request_for_class(ip, RequestClass::Recall).await.is_ok(),
            "recall class should remain available after store bucket is saturated"
        );
    }
    #[tokio::test]
    async fn test_cleanup_removes_stale() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let _ = rl.check_request(ip).await;
        rl.cleanup().await;
        let map = rl.requests.lock().await;
        assert!(map.contains_key(&(ip, RequestClass::Default)));
    }
    #[test]
    fn sliding_window_try_record_prunes_expired_front_entries() {
        let mut window = SlidingWindow::new();
        let now = Instant::now();
        window.timestamps.push_back(now - Duration::from_secs(61));
        window.timestamps.push_back(now - Duration::from_secs(59));
        let remaining = window.try_record(now, 2, WINDOW).expect("expired entries should be pruned before limit check");
        assert_eq!(remaining, 0);
        assert_eq!(window.timestamps.len(), 2);
        assert!(window.timestamps.iter().all(|ts| now.duration_since(*ts) < WINDOW));
        let retry = window.try_record(now, 2, WINDOW).expect_err("window should be full at limit");
        assert_eq!(retry, 1);
        let later = now + Duration::from_secs(2);
        assert!(window.try_record(later, 2, WINDOW).is_ok(), "oldest non-expired entry should age out and free a slot");
    }
    #[tokio::test]
    async fn budget_allows_exactly_limit_then_rejects() {
        let status = BudgetConfigStatus::load_from_path(write_budget_file(
            r#"
[endpoints.recall]
limit = 2
window_seconds = 60
"#,
        ));
        let rl = RateLimiter::new_with_budget_status(status);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 88));
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall).await.unwrap().allowed);
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall).await.unwrap().allowed);
        let denied = rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall).await.unwrap();
        assert!(!denied.allowed);
        assert_eq!(denied.endpoint, BudgetEndpoint::Recall);
        assert_eq!(denied.limit, 2);
        assert_eq!(denied.window_seconds, 60);
        assert_eq!(denied.http_body_json()["source"], crate::budgets::BUDGET_SOURCE);
    }
    #[tokio::test]
    async fn budget_resets_after_window() {
        let status = BudgetConfigStatus::load_from_path(write_budget_file(
            r#"
[endpoints.store]
limit = 1
window_seconds = 1
"#,
        ));
        let rl = RateLimiter::new_with_budget_status(status);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 89));
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store).await.unwrap().allowed);
        assert!(!rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store).await.unwrap().allowed);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store).await.unwrap().allowed);
    }
    #[tokio::test]
    async fn budget_endpoint_buckets_are_independent() {
        let status = BudgetConfigStatus::load_from_path(write_budget_file(
            r#"
[endpoints.store]
limit = 1
window_seconds = 60
[endpoints.recall]
limit = 1
window_seconds = 60
"#,
        ));
        let rl = RateLimiter::new_with_budget_status(status);
        let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 90));
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store).await.unwrap().allowed);
        assert!(!rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store).await.unwrap().allowed);
        assert!(rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall).await.unwrap().allowed);
    }
    fn write_budget_file(contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("cortex-budget-rate-limit-{}.toml", uuid::Uuid::new_v4()));
        std::fs::write(&path, contents).unwrap();
        path
    }
