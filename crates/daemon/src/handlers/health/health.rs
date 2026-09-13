use super::*;
use crate::state::RuntimeState;
use chrono::Utc;
use serde_json::{json, Value};
pub(crate) fn redact_private_runtime_details(payload: &mut Value) {
    if let Some(runtime) = payload.get_mut("runtime").and_then(Value::as_object_mut) {
        runtime.remove("db_path");
        runtime.remove("token_path");
        runtime.remove("pid_path");
        runtime.remove("executable");
        runtime.remove("owner");
    }
    if let Some(stats) = payload.get_mut("stats").and_then(Value::as_object_mut) {
        stats.remove("home");
    }
}
/// Bounded semantic health from the read pool.
async fn brain_health_snapshot(cx: &asupersync::Cx, state: &RuntimeState) -> Result<Value, String> {
    let conn = state.db_read.lock(cx).await.map_err(|err| err.to_string())?;
    if !crate::db::table_exists(&conn, "outbox") {
        return Ok(json!({"status": "schema_pending"}));
    }
    Ok(crate::db::outbox::brain_health(&conn, &state.home))
}
pub async fn build_health_payload(cx: &asupersync::Cx, state: &RuntimeState, include_private_runtime: bool) -> Result<Value, String> {
    let now_unix_secs = Utc::now().timestamp();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let (memories, decisions, embeddings_count, events, db_freelist_pages, retrieval) = {
        let conn = state.db_read.lock(cx).await.map_err(|err| err.to_string())?;
        let m: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let d: i64 = conn.query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let e: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let ev: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let freelist: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let retrieval = cortex_kernel::handlers::recall::clock_health_payload(&conn);
        (m, d, e, ev, freelist, retrieval)
    };
    let (storage_bytes, backup_count, log_bytes, heavy_metrics_source, cache_age_secs) = {
        let cached = match health_heavy_metrics_cache().lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };
        if let Some(snapshot) = cache_snapshot_if_fresh(cached, now_unix_secs) {
            (snapshot.storage_bytes, snapshot.backup_count, snapshot.log_bytes, "cache", snapshot.cache_age_secs(now_unix_secs))
        } else if app_managed_warmup_active(daemon_owner.as_deref()) {
            let fallback = cached.unwrap_or(HealthHeavyMetricsSnapshot {
                computed_at_unix_secs: now_unix_secs,
                embedding_inventory: EmbeddingInventoryMetrics::default(),
                storage_bytes: 0,
                backup_count: 0,
                log_bytes: 0,
            });
            (fallback.storage_bytes, fallback.backup_count, fallback.log_bytes, "warmup-deferred", fallback.cache_age_secs(now_unix_secs))
        } else {
            let (storage_bytes, backup_count, log_bytes) = collect_storage_metrics(&state.home);
            let snapshot = HealthHeavyMetricsSnapshot {
                computed_at_unix_secs: now_unix_secs,
                embedding_inventory: EmbeddingInventoryMetrics::default(),
                storage_bytes,
                backup_count,
                log_bytes,
            };
            match health_heavy_metrics_cache().lock() {
                Ok(mut guard) => *guard = Some(snapshot),
                Err(poisoned) => *poisoned.into_inner() = Some(snapshot),
            }
            (storage_bytes, backup_count, log_bytes, "live", 0)
        }
    };
    let db_size_bytes = std::fs::metadata(&state.db_path).map(|meta| meta.len()).unwrap_or(0);
    let db_soft_limit_bytes = crate::compaction::STORAGE_SOFT_LIMIT_BYTES.max(1) as u64;
    let db_hard_limit_bytes = crate::compaction::STORAGE_HARD_LIMIT_BYTES.max(1) as u64;
    let db_pressure = crate::compaction::classify_storage_pressure(db_size_bytes as i64);
    let db_soft_utilization = ((db_size_bytes as f64) / (db_soft_limit_bytes as f64)).min(10.0);
    let degraded = state.degraded_mode.load(std::sync::atomic::Ordering::Relaxed);
    let db_corrupted = state.db_corrupted.load(std::sync::atomic::Ordering::Relaxed);
    let executable = std::env::current_exe().ok().map(|path| path.display().to_string()).unwrap_or_default();
    let ready = state.readiness.load(std::sync::atomic::Ordering::Relaxed);
    let budgets = state
        .rate_limiter
        .budget_status()
        .to_health_json(state.rate_limiter.recent_budget_denials(cx).await.map_err(|err| err.to_string())?);
    let mut payload = json!({
        "status": if degraded || db_corrupted { "degraded" } else { "ok" },
        "ready": ready,
        "degraded": degraded || db_corrupted,
        "db_corrupted": db_corrupted,
        "budgets": budgets,
        "retrieval": retrieval,
        "team_mode": state.team_mode,
        "db_freelist_pages": db_freelist_pages,
        "db_size_bytes": db_size_bytes,
        "db_soft_limit_bytes": db_soft_limit_bytes,
        "db_hard_limit_bytes": db_hard_limit_bytes,
        "db_pressure": db_pressure,
        "db_soft_utilization": db_soft_utilization,
        "storage_bytes": storage_bytes,
        "backup_count": backup_count,
        "last_verified_restore": crate::db::backup::last_verified_restore(&state.home),
        "brain": brain_health_snapshot(cx, state).await?,
        "durability_profile": crate::db::DurabilityProfile::from_env().as_str(),
        "sqlite_version": crate::db::sqlite_version(),
        "log_bytes": log_bytes,
        "health_heavy_metrics": {
            "source": heavy_metrics_source,
            "cache_ttl_secs": HEALTH_HEAVY_CACHE_TTL_SECS,
            "cache_age_secs": cache_age_secs
        },
        "stats": {
            "memories": memories,
            "decisions": decisions,
            "embeddings": embeddings_count,
            "events": events,
            "home": state.home.display().to_string()
        },
        "runtime": {
            "version": env!("CARGO_PKG_VERSION"),
            "mode": if state.team_mode { "team" } else { "solo" },
            "db_path": state.db_path.display().to_string(),
            "token_path": state.token_path.display().to_string(),
            "pid_path": state.pid_path.display().to_string(),
            "executable": executable,
            "owner": daemon_owner
        }
    });
    if !include_private_runtime {
        redact_private_runtime_details(&mut payload);
    }
    Ok(payload)
}
pub fn build_readiness_payload(state: &RuntimeState, include_private_runtime: bool) -> Value {
    let executable = std::env::current_exe().ok().map(|path| path.display().to_string()).unwrap_or_default();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let ready = state.readiness.load(std::sync::atomic::Ordering::Relaxed);
    let mut payload = json!({
        "status": if ready { "ready" } else { "starting" },
        "ready": ready,
        "runtime": {
            "version": env!("CARGO_PKG_VERSION"),
            "mode": if state.team_mode { "team" } else { "solo" },
            "db_path": state.db_path.display().to_string(),
            "token_path": state.token_path.display().to_string(),
            "pid_path": state.pid_path.display().to_string(),
            "executable": executable,
            "owner": daemon_owner
        },
        "stats": { "home": state.home.display().to_string() }
    });
    if !include_private_runtime {
        redact_private_runtime_details(&mut payload);
    }
    payload
}
