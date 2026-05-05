// SPDX-License-Identifier: MIT
//! Sliding-window rate limiter.
//!
//! Two independent buckets per source IP:
//!   - Auth failures: 10 per minute (brute-force protection)
//!   - Request volume: 100 per minute for non-loopback callers
//!   - Request volume: 10,000 per minute for loopback callers (desktop/plugin local workloads)
//!
//! Responses include `Retry-After`, `X-RateLimit-Remaining`, and
//! `X-RateLimit-Reset` headers when the limit is hit.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::budgets::{BudgetConfigStatus, BudgetDecision, BudgetEndpoint, EndpointBudget};

const AUTH_FAIL_LIMIT: usize = 10;
const REQUEST_LIMIT_NON_LOOPBACK: usize = 100;
const REQUEST_LIMIT_LOOPBACK: usize = 10_000;
const WINDOW: Duration = Duration::from_secs(60);
const BUDGET_DENIAL_RECENT_WINDOW: Duration = Duration::from_secs(60 * 60);
const LIMIT_MAX: usize = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RequestClass {
    Default,
    Recall,
    Store,
    Boot,
}

fn read_limit_env(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(1, LIMIT_MAX))
        .unwrap_or(default)
}

#[derive(Clone)]
struct SlidingWindow {
    timestamps: VecDeque<Instant>,
}

impl SlidingWindow {
    fn new() -> Self {
        Self {
            timestamps: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: Instant, window: Duration) {
        while let Some(oldest) = self.timestamps.front().copied() {
            if now.duration_since(oldest) < window {
                break;
            }
            self.timestamps.pop_front();
        }
    }

    fn seconds_until_slot_pruned(&self, now: Instant, limit: usize, window: Duration) -> u64 {
        if self.timestamps.len() < limit {
            return 0;
        }
        let oldest = self.timestamps.front().copied().unwrap_or(now);
        let elapsed = now.duration_since(oldest);
        window.as_secs().saturating_sub(elapsed.as_secs()).max(1)
    }

    fn try_record(&mut self, now: Instant, limit: usize, window: Duration) -> Result<usize, u64> {
        self.prune(now, window);
        let current = self.timestamps.len();
        if current >= limit {
            return Err(self.seconds_until_slot_pruned(now, limit, window));
        }
        self.timestamps.push_back(now);
        Ok(limit - current - 1)
    }

    fn record_unbounded(&mut self, now: Instant, window: Duration) {
        self.prune(now, window);
        self.timestamps.push_back(now);
    }

    fn len_after_prune(&mut self, now: Instant, window: Duration) -> usize {
        self.prune(now, window);
        self.timestamps.len()
    }
}

/// Shared rate limiter state, added to `RuntimeState`.
#[derive(Clone)]
pub struct RateLimiter {
    auth_failures: Arc<Mutex<HashMap<IpAddr, SlidingWindow>>>,
    requests: Arc<Mutex<HashMap<(IpAddr, RequestClass), SlidingWindow>>>,
    budget_requests: Arc<Mutex<HashMap<(IpAddr, BudgetEndpoint), SlidingWindow>>>,
    budget_denials: Arc<Mutex<SlidingWindow>>,
    total_budget_denials: Arc<AtomicUsize>,
    budget_config_status: Arc<BudgetConfigStatus>,
    auth_fail_limit: usize,
    request_limit_non_loopback: usize,
    request_limit_loopback: usize,
    recall_request_limit_non_loopback: usize,
    recall_request_limit_loopback: usize,
    store_request_limit_non_loopback: usize,
    store_request_limit_loopback: usize,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::new_with_budget_status(BudgetConfigStatus::missing_for_tests())
    }

    pub fn new_with_budget_status(budget_config_status: BudgetConfigStatus) -> Self {
        let auth_fail_limit =
            read_limit_env("CORTEX_RATE_LIMIT_AUTH_FAILS_PER_MIN", AUTH_FAIL_LIMIT);
        let request_limit_non_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_REQUESTS_PER_MIN",
            REQUEST_LIMIT_NON_LOOPBACK,
        );
        let request_limit_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_LOOPBACK_REQUESTS_PER_MIN",
            REQUEST_LIMIT_LOOPBACK,
        );
        let recall_request_limit_non_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_RECALL_REQUESTS_PER_MIN",
            request_limit_non_loopback,
        );
        let recall_request_limit_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_RECALL_LOOPBACK_REQUESTS_PER_MIN",
            request_limit_loopback,
        );
        let store_request_limit_non_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_STORE_REQUESTS_PER_MIN",
            request_limit_non_loopback,
        );
        let store_request_limit_loopback = read_limit_env(
            "CORTEX_RATE_LIMIT_STORE_LOOPBACK_REQUESTS_PER_MIN",
            request_limit_loopback,
        );
        if auth_fail_limit != AUTH_FAIL_LIMIT
            || request_limit_non_loopback != REQUEST_LIMIT_NON_LOOPBACK
            || request_limit_loopback != REQUEST_LIMIT_LOOPBACK
            || recall_request_limit_non_loopback != request_limit_non_loopback
            || recall_request_limit_loopback != request_limit_loopback
            || store_request_limit_non_loopback != request_limit_non_loopback
            || store_request_limit_loopback != request_limit_loopback
        {
            eprintln!(
                "[cortex] Rate limiter configured: auth_fails/min={auth_fail_limit}, default_requests/min(non-loopback)={request_limit_non_loopback}, default_requests/min(loopback)={request_limit_loopback}, recall_requests/min(non-loopback)={recall_request_limit_non_loopback}, recall_requests/min(loopback)={recall_request_limit_loopback}, store_requests/min(non-loopback)={store_request_limit_non_loopback}, store_requests/min(loopback)={store_request_limit_loopback}"
            );
        }
        Self {
            auth_failures: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
            budget_requests: Arc::new(Mutex::new(HashMap::new())),
            budget_denials: Arc::new(Mutex::new(SlidingWindow::new())),
            total_budget_denials: Arc::new(AtomicUsize::new(0)),
            budget_config_status: Arc::new(budget_config_status),
            auth_fail_limit,
            request_limit_non_loopback,
            request_limit_loopback,
            recall_request_limit_non_loopback,
            recall_request_limit_loopback,
            store_request_limit_non_loopback,
            store_request_limit_loopback,
        }
    }

    fn request_limit_for_ip_class(&self, ip: IpAddr, class: RequestClass) -> usize {
        let loopback = ip.is_loopback();
        match class {
            RequestClass::Default | RequestClass::Boot => {
                if loopback {
                    self.request_limit_loopback
                } else {
                    self.request_limit_non_loopback
                }
            }
            RequestClass::Recall => {
                if loopback {
                    self.recall_request_limit_loopback
                } else {
                    self.recall_request_limit_non_loopback
                }
            }
            RequestClass::Store => {
                if loopback {
                    self.store_request_limit_loopback
                } else {
                    self.store_request_limit_non_loopback
                }
            }
        }
    }

    /// Record a failed auth attempt. Returns `Err(retry_after_secs)` if blocked.
    pub async fn record_auth_failure(&self, ip: IpAddr) -> Result<(), u64> {
        let mut map = self.auth_failures.lock().await;
        let window = map.entry(ip).or_insert_with(SlidingWindow::new);
        let now = Instant::now();
        window
            .try_record(now, self.auth_fail_limit, WINDOW)
            .map(|_| ())
    }

    /// Check if an IP is currently blocked due to auth failures.
    pub async fn is_auth_blocked(&self, ip: &IpAddr) -> Option<u64> {
        let mut map = self.auth_failures.lock().await;
        if let Some(window) = map.get_mut(ip) {
            let now = Instant::now();
            window.prune(now, WINDOW);
            if window.timestamps.len() >= self.auth_fail_limit {
                return Some(window.seconds_until_slot_pruned(now, self.auth_fail_limit, WINDOW));
            }
        }
        None
    }

    /// Check and record a request. Returns `Ok(remaining)` or `Err(retry_after)`.
    pub async fn check_request(&self, ip: IpAddr) -> Result<usize, u64> {
        self.check_request_for_class(ip, RequestClass::Default)
            .await
    }

    /// Check and record a request for a route class.
    /// Returns `Ok(remaining)` or `Err(retry_after)`.
    pub async fn check_request_for_class(
        &self,
        ip: IpAddr,
        class: RequestClass,
    ) -> Result<usize, u64> {
        let mut map = self.requests.lock().await;
        let window = map.entry((ip, class)).or_insert_with(SlidingWindow::new);
        let request_limit = self.request_limit_for_ip_class(ip, class);
        let now = Instant::now();
        window.try_record(now, request_limit, WINDOW)
    }

    pub fn budget_status(&self) -> BudgetConfigStatus {
        (*self.budget_config_status).clone()
    }

    pub fn budget_for_endpoint(&self, endpoint: BudgetEndpoint) -> Option<EndpointBudget> {
        self.budget_config_status.budget_for(endpoint)
    }

    pub async fn check_budget_for_endpoint(
        &self,
        ip: IpAddr,
        endpoint: BudgetEndpoint,
    ) -> Option<BudgetDecision> {
        let budget = self.budget_for_endpoint(endpoint)?;
        let window_duration = Duration::from_secs(budget.window_seconds);
        let mut map = self.budget_requests.lock().await;
        let window = map.entry((ip, endpoint)).or_insert_with(SlidingWindow::new);
        let now = Instant::now();
        match window.try_record(now, budget.limit, window_duration) {
            Ok(remaining) => Some(BudgetDecision::allowed(endpoint, budget, remaining)),
            Err(retry_after) => {
                drop(map);
                self.record_budget_denial().await;
                Some(BudgetDecision::denied(endpoint, budget, retry_after))
            }
        }
    }

    async fn record_budget_denial(&self) {
        self.total_budget_denials.fetch_add(1, Ordering::Relaxed);
        let mut denials = self.budget_denials.lock().await;
        denials.record_unbounded(Instant::now(), BUDGET_DENIAL_RECENT_WINDOW);
    }

    pub async fn recent_budget_denials(&self) -> usize {
        let mut denials = self.budget_denials.lock().await;
        denials.len_after_prune(Instant::now(), BUDGET_DENIAL_RECENT_WINDOW)
    }

    #[allow(dead_code)]
    pub fn total_budget_denials(&self) -> usize {
        self.total_budget_denials.load(Ordering::Relaxed)
    }

    /// Periodic cleanup of stale entries (call from background task).
    pub async fn cleanup(&self) {
        let now = Instant::now();
        {
            let mut map = self.auth_failures.lock().await;
            map.retain(|_, w| {
                w.prune(now, WINDOW);
                !w.timestamps.is_empty()
            });
        }
        {
            let mut map = self.requests.lock().await;
            map.retain(|_, w| {
                w.prune(now, WINDOW);
                !w.timestamps.is_empty()
            });
        }
        {
            let budget_status = self.budget_status();
            let mut map = self.budget_requests.lock().await;
            map.retain(|(_, endpoint), w| {
                let window = budget_status
                    .budget_for(*endpoint)
                    .map(|budget| Duration::from_secs(budget.window_seconds))
                    .unwrap_or(WINDOW);
                w.prune(now, window);
                !w.timestamps.is_empty()
            });
        }
    }
}

#[cfg(test)]
mod tests {
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
        assert!(
            rl.request_limit_for_ip_class(loopback, RequestClass::Default)
                > rl.request_limit_for_ip_class(remote, RequestClass::Default)
        );
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
            let _ = rl
                .check_request_for_class(ip, RequestClass::Store)
                .await
                .expect("store class should allow requests below class limit");
        }
        assert!(
            rl.check_request_for_class(ip, RequestClass::Store)
                .await
                .is_err(),
            "store class should rate limit once its own bucket is exhausted"
        );
        assert!(
            rl.check_request_for_class(ip, RequestClass::Recall)
                .await
                .is_ok(),
            "recall class should remain available after store bucket is saturated"
        );
    }

    #[tokio::test]
    async fn test_cleanup_removes_stale() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let _ = rl.check_request(ip).await;
        rl.cleanup().await;
        // Entry still there (not expired)
        let map = rl.requests.lock().await;
        assert!(map.contains_key(&(ip, RequestClass::Default)));
    }

    #[test]
    fn sliding_window_try_record_prunes_expired_front_entries() {
        let mut window = SlidingWindow::new();
        let now = Instant::now();
        window.timestamps.push_back(now - Duration::from_secs(61));
        window.timestamps.push_back(now - Duration::from_secs(59));

        let remaining = window
            .try_record(now, 2, WINDOW)
            .expect("expired entries should be pruned before limit check");
        assert_eq!(remaining, 0);
        assert_eq!(window.timestamps.len(), 2);
        assert!(window
            .timestamps
            .iter()
            .all(|ts| now.duration_since(*ts) < WINDOW));

        let retry = window
            .try_record(now, 2, WINDOW)
            .expect_err("window should be full at limit");
        assert_eq!(retry, 1);

        let later = now + Duration::from_secs(2);
        assert!(
            window.try_record(later, 2, WINDOW).is_ok(),
            "oldest non-expired entry should age out and free a slot"
        );
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

        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall)
                .await
                .unwrap()
                .allowed
        );
        let denied = rl
            .check_budget_for_endpoint(ip, BudgetEndpoint::Recall)
            .await
            .unwrap();
        assert!(!denied.allowed);
        assert_eq!(denied.endpoint, BudgetEndpoint::Recall);
        assert_eq!(denied.limit, 2);
        assert_eq!(denied.window_seconds, 60);
        assert_eq!(
            denied.http_body_json()["source"],
            crate::budgets::BUDGET_SOURCE
        );
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

        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store)
                .await
                .unwrap()
                .allowed
        );
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store)
                .await
                .unwrap()
                .allowed
        );
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

        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            !rl.check_budget_for_endpoint(ip, BudgetEndpoint::Store)
                .await
                .unwrap()
                .allowed
        );
        assert!(
            rl.check_budget_for_endpoint(ip, BudgetEndpoint::Recall)
                .await
                .unwrap()
                .allowed
        );
    }

    fn write_budget_file(contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cortex-budget-rate-limit-{}.toml",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }
}
