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

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const AUTH_FAIL_LIMIT: usize = 10;
const REQUEST_LIMIT_NON_LOOPBACK: usize = 100;
const REQUEST_LIMIT_LOOPBACK: usize = 10_000;
const WINDOW: Duration = Duration::from_secs(60);
const LIMIT_MAX: usize = 1_000_000;

fn read_limit_env(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(1, LIMIT_MAX))
        .unwrap_or(default)
}

#[derive(Clone)]
struct SlidingWindow {
    timestamps: Vec<Instant>,
}

impl SlidingWindow {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        self.timestamps
            .retain(|ts| now.duration_since(*ts) < WINDOW);
    }

    fn count(&mut self, now: Instant) -> usize {
        self.prune(now);
        self.timestamps.len()
    }

    fn record(&mut self, now: Instant) {
        self.prune(now);
        self.timestamps.push(now);
    }

    fn seconds_until_slot(&mut self, now: Instant, limit: usize) -> u64 {
        self.prune(now);
        if self.timestamps.len() < limit {
            return 0;
        }
        let oldest = self.timestamps[0];
        let elapsed = now.duration_since(oldest);
        WINDOW.as_secs().saturating_sub(elapsed.as_secs()).max(1)
    }
}

/// Shared rate limiter state, added to `RuntimeState`.
#[derive(Clone)]
pub struct RateLimiter {
    auth_failures: Arc<Mutex<HashMap<IpAddr, SlidingWindow>>>,
    requests: Arc<Mutex<HashMap<IpAddr, SlidingWindow>>>,
    auth_fail_limit: usize,
    request_limit_non_loopback: usize,
    request_limit_loopback: usize,
}

impl RateLimiter {
    pub fn new() -> Self {
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
        if auth_fail_limit != AUTH_FAIL_LIMIT
            || request_limit_non_loopback != REQUEST_LIMIT_NON_LOOPBACK
            || request_limit_loopback != REQUEST_LIMIT_LOOPBACK
        {
            eprintln!(
                "[cortex] Rate limiter configured: auth_fails/min={auth_fail_limit}, requests/min(non-loopback)={request_limit_non_loopback}, requests/min(loopback)={request_limit_loopback}"
            );
        }
        Self {
            auth_failures: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
            auth_fail_limit,
            request_limit_non_loopback,
            request_limit_loopback,
        }
    }

    fn request_limit_for_ip(&self, ip: IpAddr) -> usize {
        if ip.is_loopback() {
            self.request_limit_loopback
        } else {
            self.request_limit_non_loopback
        }
    }

    /// Record a failed auth attempt. Returns `Err(retry_after_secs)` if blocked.
    pub async fn record_auth_failure(&self, ip: IpAddr) -> Result<(), u64> {
        let mut map = self.auth_failures.lock().await;
        let window = map.entry(ip).or_insert_with(SlidingWindow::new);
        let now = Instant::now();
        if window.count(now) >= self.auth_fail_limit {
            let retry = window.seconds_until_slot(now, self.auth_fail_limit);
            return Err(retry);
        }
        window.record(now);
        Ok(())
    }

    /// Check if an IP is currently blocked due to auth failures.
    pub async fn is_auth_blocked(&self, ip: &IpAddr) -> Option<u64> {
        let mut map = self.auth_failures.lock().await;
        if let Some(window) = map.get_mut(ip) {
            let now = Instant::now();
            if window.count(now) >= self.auth_fail_limit {
                return Some(window.seconds_until_slot(now, self.auth_fail_limit));
            }
        }
        None
    }

    /// Check and record a request. Returns `Ok(remaining)` or `Err(retry_after)`.
    pub async fn check_request(&self, ip: IpAddr) -> Result<usize, u64> {
        let mut map = self.requests.lock().await;
        let window = map.entry(ip).or_insert_with(SlidingWindow::new);
        let request_limit = self.request_limit_for_ip(ip);
        let now = Instant::now();
        let current = window.count(now);
        if current >= request_limit {
            let retry = window.seconds_until_slot(now, request_limit);
            return Err(retry);
        }
        window.record(now);
        Ok(request_limit - current - 1)
    }

    /// Periodic cleanup of stale entries (call from background task).
    pub async fn cleanup(&self) {
        let now = Instant::now();
        {
            let mut map = self.auth_failures.lock().await;
            map.retain(|_, w| {
                w.prune(now);
                !w.timestamps.is_empty()
            });
        }
        {
            let mut map = self.requests.lock().await;
            map.retain(|_, w| {
                w.prune(now);
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
        let limit = rl.request_limit_for_ip(ip);
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
        assert!(rl.request_limit_for_ip(loopback) > rl.request_limit_for_ip(remote));
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
        let limit = rl.request_limit_for_ip(ip1);
        for _ in 0..limit {
            let _ = rl.check_request(ip1).await;
        }
        assert!(rl.check_request(ip1).await.is_err());
        assert!(rl.check_request(ip2).await.is_ok());
    }

    #[tokio::test]
    async fn test_cleanup_removes_stale() {
        let rl = RateLimiter::new();
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let _ = rl.check_request(ip).await;
        rl.cleanup().await;
        // Entry still there (not expired)
        let map = rl.requests.lock().await;
        assert!(map.contains_key(&ip));
    }
}
