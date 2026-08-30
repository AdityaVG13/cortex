use super::*;
use crate::handlers::{client_ip, ensure_ssrf_protection, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use serde_json::{json, Value};
pub(crate) fn include_private_runtime_details(headers: &HeaderMap) -> bool {
    ensure_ssrf_protection(headers).is_ok() && client_ip(headers).is_loopback()
}
pub(crate) fn redact_private_runtime_details(payload: &mut Value) {
    if let Some(runtime) = payload.get_mut("runtime").and_then(Value::as_object_mut) {
        runtime.remove("db_path");
        runtime.remove("token_path");
        runtime.remove("pid_path");
        runtime.remove("ipc_endpoint");
        runtime.remove("ipc_kind");
        runtime.remove("executable");
        runtime.remove("owner");
    }
    if let Some(stats) = payload.get_mut("stats").and_then(Value::as_object_mut) {
        stats.remove("home");
    }
}
pub async fn build_health_payload(state: &RuntimeState, include_private_runtime: bool) -> Value {
    let now_unix_secs = Utc::now().timestamp();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let (memories, decisions, embeddings_count, events, db_freelist_pages, retrieval) = {
        let conn = state.db_read.lock().await;
        let m: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0)).unwrap_or(0);
        let d: i64 = conn.query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0)).unwrap_or(0);
        let e: i64 = conn.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0)).unwrap_or(0);
        let ev: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap_or(0);
        let freelist: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0)).unwrap_or(0);
        let retrieval = crate::handlers::recall::clock_health_payload(&conn);
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
    let ipc_endpoint = std::env::var("CORTEX_IPC_ENDPOINT").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let ipc_kind = if ipc_endpoint.is_some() { Some(if cfg!(windows) { "named-pipe" } else { "unix-socket" }) } else { None };
    let ready = state.readiness.load(std::sync::atomic::Ordering::Relaxed);
    let budgets = state.rate_limiter.budget_status().to_health_json(state.rate_limiter.recent_budget_denials().await);
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
            "port": state.port,
            "db_path": state.db_path.display().to_string(),
            "token_path": state.token_path.display().to_string(),
            "pid_path": state.pid_path.display().to_string(),
            "ipc_endpoint": ipc_endpoint,
            "ipc_kind": ipc_kind,
            "executable": executable,
            "owner": daemon_owner
        }
    });
    if !include_private_runtime {
        redact_private_runtime_details(&mut payload);
    }
    payload
}
pub async fn handle_health(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    let include_private_runtime = include_private_runtime_details(&headers);
    json_response(StatusCode::OK, build_health_payload(&state, include_private_runtime).await)
}
pub async fn build_readiness_payload(state: &RuntimeState, include_private_runtime: bool) -> Value {
    let executable = std::env::current_exe().ok().map(|path| path.display().to_string()).unwrap_or_default();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let ipc_endpoint = std::env::var("CORTEX_IPC_ENDPOINT").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let ipc_kind = if ipc_endpoint.is_some() { Some(if cfg!(windows) { "named-pipe" } else { "unix-socket" }) } else { None };
    let ready = state.readiness.load(std::sync::atomic::Ordering::Relaxed);
    let mut payload = json!({"status":if ready{"ready"}else{"starting"},"ready":ready,"runtime":{"version":env!(
"CARGO_PKG_VERSION"),"mode":if state.team_mode{"team"}else{"solo"},"port":state.port,"db_path":state.db_path.display().to_string()
,"token_path":state.token_path.display().to_string(),"pid_path":state.pid_path.display().to_string(),"ipc_endpoint":ipc_endpoint,
"ipc_kind":ipc_kind,"executable":executable,"owner":daemon_owner},"stats":{"home":state.home.display().to_string()}});
    if !include_private_runtime {
        redact_private_runtime_details(&mut payload);
    }
    payload
}
pub async fn handle_readiness(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    let include_private_runtime = include_private_runtime_details(&headers);
    let payload = build_readiness_payload(&state, include_private_runtime).await;
    let ready = payload.get("ready").and_then(|value| value.as_bool()).unwrap_or(false);
    let status = if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    json_response(status, payload)
}
