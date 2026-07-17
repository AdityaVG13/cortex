// SPDX-License-Identifier: MIT
use rusqlite::params;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
pub(crate) const STORAGE_LOG_FILES: &[&str] = &["daemon.log", "daemon.err.log", "daemon.out.log", "mcp-crash.log", "rust-daemon.err.log"];
pub(crate) const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
pub(crate) const HEALTH_HEAVY_CACHE_TTL_SECS: i64 = 30;
pub(crate) const HEALTH_HEAVY_WARMUP_DELAY_SECS: u64 = 90;
pub(crate) const SAVINGS_CACHE_TTL_SECS: i64 = 20;
pub(crate) const SAVINGS_HISTORY_DAYS: i64 = 30;
static HEALTH_BOOT_INSTANT: OnceLock<Instant> = OnceLock::new();
static HEALTH_HEAVY_METRICS_CACHE: OnceLock<Mutex<Option<HealthHeavyMetricsSnapshot>>> = OnceLock::new();
static SAVINGS_PAYLOAD_CACHE: OnceLock<Mutex<Option<SavingsPayloadSnapshot>>> = OnceLock::new();
pub(crate) fn directory_size_bytes(path: &std::path::Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| directory_size_bytes(&entry.path())).sum())
            .unwrap_or(0),
        _ => 0,
    }
}
pub(crate) fn collect_storage_metrics(home: &std::path::Path) -> (u64, usize, u64) {
    let storage_bytes = directory_size_bytes(home);
    let backup_count = std::fs::read_dir(home.join("backups"))
        .map(|entries| entries.filter_map(|entry| entry.ok()).filter(|entry| entry.file_name().to_string_lossy().ends_with(".db")).count())
        .unwrap_or(0);
    let log_bytes = STORAGE_LOG_FILES
        .iter()
        .flat_map(|name| [home.join(name), home.join(format!("{name}.1"))])
        .map(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum();
    (storage_bytes, backup_count, log_bytes)
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EmbeddingInventoryMetrics {
    pub(crate) active_model_embeddings: i64,
    pub(crate) other_model_embeddings: i64,
    pub(crate) unknown_model_embeddings: i64,
    pub(crate) backlog_memories: i64,
    pub(crate) backlog_decisions: i64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct HealthHeavyMetricsSnapshot {
    pub(crate) computed_at_unix_secs: i64,
    pub(crate) embedding_inventory: EmbeddingInventoryMetrics,
    pub(crate) storage_bytes: u64,
    pub(crate) backup_count: usize,
    pub(crate) log_bytes: u64,
}
impl HealthHeavyMetricsSnapshot {
    pub(crate) fn cache_age_secs(self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}
#[derive(Clone, Debug)]
pub(crate) struct SavingsPayloadSnapshot {
    pub(crate) computed_at_unix_secs: i64,
    pub(crate) payload: Value,
}
impl SavingsPayloadSnapshot {
    pub(crate) fn cache_age_secs(&self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}
pub(crate) fn is_control_center_owner(owner_tag: Option<&str>) -> bool {
    owner_tag.map(|owner| owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG)).unwrap_or(false)
}
pub(crate) fn health_heavy_metrics_cache() -> &'static Mutex<Option<HealthHeavyMetricsSnapshot>> {
    HEALTH_HEAVY_METRICS_CACHE.get_or_init(|| Mutex::new(None))
}
pub(crate) fn savings_payload_cache() -> &'static Mutex<Option<SavingsPayloadSnapshot>> {
    SAVINGS_PAYLOAD_CACHE.get_or_init(|| Mutex::new(None))
}
pub(crate) fn app_managed_warmup_active(daemon_owner: Option<&str>) -> bool {
    if !is_control_center_owner(daemon_owner) {
        return false;
    }
    let started = HEALTH_BOOT_INSTANT.get_or_init(Instant::now);
    started.elapsed() < Duration::from_secs(HEALTH_HEAVY_WARMUP_DELAY_SECS)
}
pub(crate) fn cache_snapshot_if_fresh(snapshot: Option<HealthHeavyMetricsSnapshot>, now_unix_secs: i64) -> Option<HealthHeavyMetricsSnapshot> {
    snapshot.and_then(|entry| if entry.cache_age_secs(now_unix_secs) <= HEALTH_HEAVY_CACHE_TTL_SECS { Some(entry) } else { None })
}
pub(crate) fn savings_payload_cache_if_fresh(snapshot: Option<SavingsPayloadSnapshot>, now_unix_secs: i64) -> Option<SavingsPayloadSnapshot> {
    snapshot.and_then(|entry| if entry.cache_age_secs(now_unix_secs) <= SAVINGS_CACHE_TTL_SECS { Some(entry) } else { None })
}
pub(crate) fn weekday_name_from_sqlite(weekday: i64) -> &'static str {
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
pub(crate) fn collect_embedding_inventory(conn: &rusqlite::Connection, active_model_key: &str) -> EmbeddingInventoryMetrics {
    let total_embeddings: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0)).unwrap_or(0);
    let active_model_embeddings: i64 = conn
        .query_row("SELECT COUNT(*) FROM embeddings WHERE LOWER(COALESCE(model, '')) = ?1", params![active_model_key], |r| r.get(0))
        .unwrap_or(0);
    let unknown_model_embeddings: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings WHERE model IS NULL OR TRIM(model) = ''", [], |r| r.get(0)).unwrap_or(0);
    let other_model_embeddings = (total_embeddings - active_model_embeddings - unknown_model_embeddings).max(0);
    let backlog_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories m \
             WHERE m.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'memory' \
                     AND e.target_id = m.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            params![active_model_key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let backlog_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions d \
             WHERE d.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'decision' \
                     AND e.target_id = d.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            params![active_model_key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    EmbeddingInventoryMetrics {
        active_model_embeddings,
        other_model_embeddings,
        unknown_model_embeddings,
        backlog_memories,
        backlog_decisions,
    }
}
