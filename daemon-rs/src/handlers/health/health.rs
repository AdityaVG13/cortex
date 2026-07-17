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
    let embedding_model = crate::embeddings::selected_model_selection();
    let now_unix_secs = Utc::now().timestamp();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let (memories, decisions, embeddings_count, events, db_freelist_pages, sqlite_vec_status) = {
        let conn = state.db_read.lock().await;
        let m: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap_or(0);
        let d: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
            .unwrap_or(0);
        let e: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .unwrap_or(0);
        let ev: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap_or(0);
        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .unwrap_or(0);
        let sqlite_vec_status = crate::db::sqlite_vec_status(&conn);
        (m, d, e, ev, freelist, sqlite_vec_status)
    };
    let (
        embedding_inventory,
        storage_bytes,
        backup_count,
        log_bytes,
        heavy_metrics_source,
        cache_age_secs,
    ) = {
        let cached = match health_heavy_metrics_cache().lock() {
            Ok(guard) => *guard,
            Err(poisoned) => *poisoned.into_inner(),
        };
        if let Some(snapshot) = cache_snapshot_if_fresh(cached, now_unix_secs) {
            (
                snapshot.embedding_inventory,
                snapshot.storage_bytes,
                snapshot.backup_count,
                snapshot.log_bytes,
                "cache",
                snapshot.cache_age_secs(now_unix_secs),
            )
        } else if app_managed_warmup_active(daemon_owner.as_deref()) {
            let fallback = cached.unwrap_or(HealthHeavyMetricsSnapshot {
                computed_at_unix_secs: now_unix_secs,
                embedding_inventory: EmbeddingInventoryMetrics::default(),
                storage_bytes: 0,
                backup_count: 0,
                log_bytes: 0,
            });
            (
                fallback.embedding_inventory,
                fallback.storage_bytes,
                fallback.backup_count,
                fallback.log_bytes,
                "warmup-deferred",
                fallback.cache_age_secs(now_unix_secs),
            )
        } else {
            let embedding_inventory = {
                let conn = state.db_read.lock().await;
                collect_embedding_inventory(&conn, embedding_model.key)
            };
            let (storage_bytes, backup_count, log_bytes) = collect_storage_metrics(&state.home);
            let snapshot = HealthHeavyMetricsSnapshot {
                computed_at_unix_secs: now_unix_secs,
                embedding_inventory,
                storage_bytes,
                backup_count,
                log_bytes,
            };
            match health_heavy_metrics_cache().lock() {
                Ok(mut guard) => *guard = Some(snapshot),
                Err(poisoned) => *poisoned.into_inner() = Some(snapshot),
            }
            (
                embedding_inventory,
                storage_bytes,
                backup_count,
                log_bytes,
                "live",
                0,
            )
        }
    };
    let db_size_bytes = std::fs::metadata(&state.db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let db_soft_limit_bytes = crate::compaction::STORAGE_SOFT_LIMIT_BYTES.max(1) as u64;
    let db_hard_limit_bytes = crate::compaction::STORAGE_HARD_LIMIT_BYTES.max(1) as u64;
    let db_pressure = crate::compaction::classify_storage_pressure(db_size_bytes as i64);
    let db_soft_utilization = ((db_size_bytes as f64) / (db_soft_limit_bytes as f64)).min(10.0);
    let active_model_ratio = if embeddings_count > 0 {
        (embedding_inventory.active_model_embeddings as f64) / (embeddings_count as f64)
    } else {
        0.0
    };
    let reembed_backlog_total =
        embedding_inventory.backlog_memories + embedding_inventory.backlog_decisions;
    let degraded = state
        .degraded_mode
        .load(std::sync::atomic::Ordering::Relaxed);
    let reranker_model = crate::rerank::selected_reranker_selection();
    let db_corrupted = state
        .db_corrupted
        .load(std::sync::atomic::Ordering::Relaxed);
    let embedding_status = if degraded {
        "degraded"
    } else if state.embedding_engine.is_some() {
        "available"
    } else {
        "unavailable"
    };
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let ipc_endpoint = std::env::var("CORTEX_IPC_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let ipc_kind = if ipc_endpoint.is_some() {
        Some(if cfg!(windows) {
            "named-pipe"
        } else {
            "unix-socket"
        })
    } else {
        None
    };
    let ready = state.readiness.load(std::sync::atomic::Ordering::Relaxed);
    let budgets = state
        .rate_limiter
        .budget_status()
        .to_health_json(state.rate_limiter.recent_budget_denials().await);
    let mut payload = json!({"status":if degraded||db_corrupted{"degraded"}else{"ok"},
"ready":ready,"degraded":degraded||db_corrupted,"db_corrupted":db_corrupted,"budgets":budgets,"embedding_status":embedding_status,
"vector_search":{"backend":if matches!(state.sqlite_vec_canary.effective_route_mode(),crate::state::SqliteVecRouteMode::Primary){
"sqlite_vec_primary"}else{"blob_scan"},"embedding_model":{"key":embedding_model.key,"display_name":embedding_model.display_name,
"dimension":embedding_model.dimension,"max_input_tokens":embedding_model.max_input_tokens,"pooling":embedding_model.pooling,
"model_file":embedding_model.model_file,"tokenizer_file":embedding_model.tokenizer_file},"routing":{"mode":state.sqlite_vec_canary
.route_mode.as_str(),"effective_mode":state.sqlite_vec_canary.effective_route_mode().as_str(),"trial_percent":state.
sqlite_vec_canary.trial_percent,"force_off":state.sqlite_vec_canary.force_off},"reranker":{"mode":state.rerank_config.mode.as_str(
),"available":state.reranker.is_some(),"model":{"key":reranker_model.key,"display_name":reranker_model.display_name,
"model_size_mb":reranker_model.model_size_mb,"max_input_tokens":reranker_model.max_input_tokens,"model_file":reranker_model.
model_file,"tokenizer_file":reranker_model.tokenizer_file},"top_n":state.rerank_config.top_n,"fusion_alpha":state.rerank_config.
fusion_alpha},"embedding_inventory":{"active_model_key":embedding_model.key,"active_model_embeddings":embedding_inventory.
active_model_embeddings,"other_model_embeddings":embedding_inventory.other_model_embeddings,"unknown_model_embeddings":
embedding_inventory.unknown_model_embeddings,"active_model_ratio":active_model_ratio,"reembed_backlog":{"memories":
embedding_inventory.backlog_memories,"decisions":embedding_inventory.backlog_decisions,"total":reembed_backlog_total}},
"sqlite_vec":{"available":sqlite_vec_status.available,"version":sqlite_vec_status.version,"error":sqlite_vec_status.error},
"health_heavy_metrics":{"source":heavy_metrics_source,"cache_ttl_secs":HEALTH_HEAVY_CACHE_TTL_SECS,"cache_age_secs":cache_age_secs
},},"team_mode":state.team_mode,"db_freelist_pages":db_freelist_pages,"db_size_bytes":db_size_bytes,"db_soft_limit_bytes":
db_soft_limit_bytes,"db_hard_limit_bytes":db_hard_limit_bytes,"db_pressure":db_pressure,"db_soft_utilization":db_soft_utilization,
"storage_bytes":storage_bytes,"backup_count":backup_count,"log_bytes":log_bytes,"stats":{"memories":memories,"decisions":decisions
,"embeddings":embeddings_count,"events":events,"home":state.home.display().to_string()},"runtime":{"version":env!(
"CARGO_PKG_VERSION"),"mode":if state.team_mode{"team"}else{"solo"},"port":state.port,"db_path":state.db_path.display().to_string()
,"token_path":state.token_path.display().to_string(),"pid_path":state.pid_path.display().to_string(),"ipc_endpoint":ipc_endpoint,
"ipc_kind":ipc_kind,"executable":executable,"owner":daemon_owner}});
    if !include_private_runtime {
        redact_private_runtime_details(&mut payload);
    }
    payload
}
pub async fn handle_health(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    let include_private_runtime = include_private_runtime_details(&headers);
    json_response(
        StatusCode::OK,
        build_health_payload(&state, include_private_runtime).await,
    )
}
pub async fn build_readiness_payload(state: &RuntimeState, include_private_runtime: bool) -> Value {
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let ipc_endpoint = std::env::var("CORTEX_IPC_ENDPOINT")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let ipc_kind = if ipc_endpoint.is_some() {
        Some(if cfg!(windows) {
            "named-pipe"
        } else {
            "unix-socket"
        })
    } else {
        None
    };
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
    let ready = payload
        .get("ready")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    json_response(status, payload)
}
