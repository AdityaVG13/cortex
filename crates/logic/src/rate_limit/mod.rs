#![allow(dead_code)]
use crate::budgets::{BudgetConfigStatus, BudgetDecision, BudgetEndpoint, EndpointBudget};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
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
    pub async fn record_auth_failure(&self, ip: IpAddr) -> Result<(), u64> {
        let mut map = self.auth_failures.lock().await;
        let window = map.entry(ip).or_insert_with(SlidingWindow::new);
        let now = Instant::now();
        window
            .try_record(now, self.auth_fail_limit, WINDOW)
            .map(|_| ())
    }
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
    pub async fn check_request(&self, ip: IpAddr) -> Result<usize, u64> {
        self.check_request_for_class(ip, RequestClass::Default)
            .await
    }
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
