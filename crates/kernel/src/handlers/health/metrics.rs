use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
pub const STORAGE_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];
pub const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
pub const HEALTH_HEAVY_CACHE_TTL_SECS: i64 = 30;
pub const HEALTH_HEAVY_WARMUP_DELAY_SECS: u64 = 90;
pub const SAVINGS_CACHE_TTL_SECS: i64 = 20;
pub const SAVINGS_HISTORY_DAYS: i64 = 30;
static HEALTH_BOOT_INSTANT: OnceLock<Instant> = OnceLock::new();
static HEALTH_HEAVY_METRICS_CACHE: OnceLock<Mutex<Option<HealthHeavyMetricsSnapshot>>> =
    OnceLock::new();
static SAVINGS_PAYLOAD_CACHE: OnceLock<Mutex<Option<SavingsPayloadSnapshot>>> = OnceLock::new();
pub fn directory_size_bytes(path: &std::path::Path) -> u64 {
    directory_size_bytes_inner(path, true)
}

/// `follow_root` lets `CORTEX_HOME` itself be a symlink. Child entries are
/// measured with `lstat` so a planted symlink under home cannot walk `/`.
fn directory_size_bytes_inner(path: &std::path::Path, follow_root: bool) -> u64 {
    let meta = if follow_root {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    };
    match meta {
        Ok(meta) if !follow_root && meta.file_type().is_symlink() => 0,
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| directory_size_bytes_inner(&entry.path(), false))
                    .sum()
            })
            .unwrap_or(0),
        _ => 0,
    }
}
pub fn collect_storage_metrics(home: &std::path::Path) -> (u64, usize, u64) {
    let storage_bytes = directory_size_bytes(home);
    let backup_count = std::fs::read_dir(home.join("backups"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".db"))
                .count()
        })
        .unwrap_or(0);
    let log_bytes = STORAGE_LOG_FILES
        .iter()
        .flat_map(|name| [home.join(name), home.join(format!("{name}.1"))])
        .map(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum();
    (storage_bytes, backup_count, log_bytes)
}
#[derive(Clone, Copy, Debug, Default)]
pub struct EmbeddingInventoryMetrics {
    pub active_model_embeddings: i64,
    pub other_model_embeddings: i64,
    pub unknown_model_embeddings: i64,
    pub backlog_memories: i64,
    pub backlog_decisions: i64,
}
#[derive(Clone, Copy, Debug)]
pub struct HealthHeavyMetricsSnapshot {
    pub computed_at_unix_secs: i64,
    pub embedding_inventory: EmbeddingInventoryMetrics,
    pub storage_bytes: u64,
    pub backup_count: usize,
    pub log_bytes: u64,
}
impl HealthHeavyMetricsSnapshot {
    pub fn cache_age_secs(self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}
#[derive(Clone, Debug)]
pub struct SavingsPayloadSnapshot {
    pub computed_at_unix_secs: i64,
    pub payload: Value,
}
impl SavingsPayloadSnapshot {
    pub fn cache_age_secs(&self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}
pub fn is_control_center_owner(owner_tag: Option<&str>) -> bool {
    owner_tag
        .map(|owner| owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG))
        .unwrap_or(false)
}
pub fn health_heavy_metrics_cache() -> &'static Mutex<Option<HealthHeavyMetricsSnapshot>> {
    HEALTH_HEAVY_METRICS_CACHE.get_or_init(|| Mutex::new(None))
}
pub fn savings_payload_cache() -> &'static Mutex<Option<SavingsPayloadSnapshot>> {
    SAVINGS_PAYLOAD_CACHE.get_or_init(|| Mutex::new(None))
}
pub fn app_managed_warmup_active(daemon_owner: Option<&str>) -> bool {
    if !is_control_center_owner(daemon_owner) {
        return false;
    }
    let started = HEALTH_BOOT_INSTANT.get_or_init(Instant::now);
    started.elapsed() < Duration::from_secs(HEALTH_HEAVY_WARMUP_DELAY_SECS)
}
pub fn cache_snapshot_if_fresh(
    snapshot: Option<HealthHeavyMetricsSnapshot>,
    now_unix_secs: i64,
) -> Option<HealthHeavyMetricsSnapshot> {
    snapshot.filter(|entry| entry.cache_age_secs(now_unix_secs) <= HEALTH_HEAVY_CACHE_TTL_SECS)
}
pub fn savings_payload_cache_if_fresh(
    snapshot: Option<SavingsPayloadSnapshot>,
    now_unix_secs: i64,
) -> Option<SavingsPayloadSnapshot> {
    snapshot.filter(|entry| entry.cache_age_secs(now_unix_secs) <= SAVINGS_CACHE_TTL_SECS)
}
pub fn weekday_name_from_sqlite(weekday: i64) -> &'static str {
    match weekday {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "Unknown",
    }
}
pub fn collect_embedding_inventory(
    _conn: &rusqlite::Connection,
    _active_model_key: &str,
) -> EmbeddingInventoryMetrics {
    EmbeddingInventoryMetrics::default()
}
