// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{ensure_auth_rated, json_response, truncate_chars};
use crate::state::RuntimeState;

const STORAGE_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];
const CONTROL_CENTER_OWNER_TAG: &str = "control-center";
const HEALTH_HEAVY_CACHE_TTL_SECS: i64 = 30;
const HEALTH_HEAVY_WARMUP_DELAY_SECS: u64 = 90;
const SAVINGS_CACHE_TTL_SECS: i64 = 20;
static HEALTH_BOOT_INSTANT: OnceLock<Instant> = OnceLock::new();
static HEALTH_HEAVY_METRICS_CACHE: OnceLock<Mutex<Option<HealthHeavyMetricsSnapshot>>> =
    OnceLock::new();
static SAVINGS_PAYLOAD_CACHE: OnceLock<Mutex<Option<SavingsPayloadSnapshot>>> = OnceLock::new();

fn directory_size_bytes(path: &std::path::Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.len(),
        Ok(meta) if meta.is_dir() => std::fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| directory_size_bytes(&entry.path()))
                    .sum()
            })
            .unwrap_or(0),
        _ => 0,
    }
}

fn collect_storage_metrics(home: &std::path::Path) -> (u64, usize, u64) {
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
struct EmbeddingInventoryMetrics {
    active_model_embeddings: i64,
    other_model_embeddings: i64,
    unknown_model_embeddings: i64,
    backlog_memories: i64,
    backlog_decisions: i64,
}

#[derive(Clone, Copy, Debug)]
struct HealthHeavyMetricsSnapshot {
    computed_at_unix_secs: i64,
    embedding_inventory: EmbeddingInventoryMetrics,
    storage_bytes: u64,
    backup_count: usize,
    log_bytes: u64,
}

impl HealthHeavyMetricsSnapshot {
    fn cache_age_secs(self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}

#[derive(Clone, Debug)]
struct SavingsPayloadSnapshot {
    computed_at_unix_secs: i64,
    payload: Value,
}

impl SavingsPayloadSnapshot {
    fn cache_age_secs(&self, now_unix_secs: i64) -> i64 {
        (now_unix_secs - self.computed_at_unix_secs).max(0)
    }
}

fn is_control_center_owner(owner_tag: Option<&str>) -> bool {
    owner_tag
        .map(|owner| owner.eq_ignore_ascii_case(CONTROL_CENTER_OWNER_TAG))
        .unwrap_or(false)
}

fn health_heavy_metrics_cache() -> &'static Mutex<Option<HealthHeavyMetricsSnapshot>> {
    HEALTH_HEAVY_METRICS_CACHE.get_or_init(|| Mutex::new(None))
}

fn savings_payload_cache() -> &'static Mutex<Option<SavingsPayloadSnapshot>> {
    SAVINGS_PAYLOAD_CACHE.get_or_init(|| Mutex::new(None))
}

fn app_managed_warmup_active(daemon_owner: Option<&str>) -> bool {
    if !is_control_center_owner(daemon_owner) {
        return false;
    }
    let started = HEALTH_BOOT_INSTANT.get_or_init(Instant::now);
    started.elapsed() < Duration::from_secs(HEALTH_HEAVY_WARMUP_DELAY_SECS)
}

fn cache_snapshot_if_fresh(
    snapshot: Option<HealthHeavyMetricsSnapshot>,
    now_unix_secs: i64,
) -> Option<HealthHeavyMetricsSnapshot> {
    snapshot.and_then(|entry| {
        if entry.cache_age_secs(now_unix_secs) <= HEALTH_HEAVY_CACHE_TTL_SECS {
            Some(entry)
        } else {
            None
        }
    })
}

fn savings_payload_cache_if_fresh(
    snapshot: Option<SavingsPayloadSnapshot>,
    now_unix_secs: i64,
) -> Option<SavingsPayloadSnapshot> {
    snapshot.and_then(|entry| {
        if entry.cache_age_secs(now_unix_secs) <= SAVINGS_CACHE_TTL_SECS {
            Some(entry)
        } else {
            None
        }
    })
}

fn weekday_name_from_sqlite(weekday: i64) -> &'static str {
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

fn collect_embedding_inventory(
    conn: &rusqlite::Connection,
    active_model_key: &str,
) -> EmbeddingInventoryMetrics {
    let total_embeddings: i64 = conn
        .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
        .unwrap_or(0);
    let active_model_embeddings: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embeddings WHERE LOWER(COALESCE(model, '')) = ?1",
            params![active_model_key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let unknown_model_embeddings: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM embeddings WHERE model IS NULL OR TRIM(model) = ''",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let other_model_embeddings =
        (total_embeddings - active_model_embeddings - unknown_model_embeddings).max(0);
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

// ─── GET /health ─────────────────────────────────────────────────────────────

pub async fn build_health_payload(state: &RuntimeState) -> Value {
    let embedding_model = crate::embeddings::selected_model_selection();
    let now_unix_secs = Utc::now().timestamp();
    let daemon_owner = std::env::var("CORTEX_DAEMON_OWNER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    // Read DB stats in a short lock, then drop it before the network call.
    let (
        memories,
        decisions,
        embeddings_count,
        events,
        db_freelist_pages,
        sqlite_vec_status,
    ) = {
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
    }; // DB lock released here.

    let (embedding_inventory, storage_bytes, backup_count, log_bytes, heavy_metrics_source, cache_age_secs) = {
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

    json!({
        "status": if degraded || db_corrupted { "degraded" } else { "ok" },
        "ready": ready,
        "degraded": degraded || db_corrupted,
        "db_corrupted": db_corrupted,
        "embedding_status": embedding_status,
        "vector_search": {
            "backend": "blob_scan",
            "embedding_model": {
                "key": embedding_model.key,
                "display_name": embedding_model.display_name,
                "dimension": embedding_model.dimension,
                "model_file": embedding_model.model_file,
                "tokenizer_file": embedding_model.tokenizer_file
            },
            "embedding_inventory": {
                "active_model_key": embedding_model.key,
                "active_model_embeddings": embedding_inventory.active_model_embeddings,
                "other_model_embeddings": embedding_inventory.other_model_embeddings,
                "unknown_model_embeddings": embedding_inventory.unknown_model_embeddings,
                "active_model_ratio": active_model_ratio,
                "reembed_backlog": {
                    "memories": embedding_inventory.backlog_memories,
                    "decisions": embedding_inventory.backlog_decisions,
                    "total": reembed_backlog_total
                }
            },
            "sqlite_vec": {
                "available": sqlite_vec_status.available,
                "version": sqlite_vec_status.version,
                "error": sqlite_vec_status.error
            },
            "health_heavy_metrics": {
                "source": heavy_metrics_source,
                "cache_ttl_secs": HEALTH_HEAVY_CACHE_TTL_SECS,
                "cache_age_secs": cache_age_secs
            },
        },
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
    })
}

pub async fn handle_health(State(state): State<RuntimeState>) -> Response {
    json_response(StatusCode::OK, build_health_payload(&state).await)
}

pub async fn build_readiness_payload(state: &RuntimeState) -> Value {
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

    json!({
        "status": if ready { "ready" } else { "starting" },
        "ready": ready,
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
        },
        "stats": {
            "home": state.home.display().to_string()
        }
    })
}

pub async fn handle_readiness(State(state): State<RuntimeState>) -> Response {
    let payload = build_readiness_payload(&state).await;
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

// ─── GET /digest ─────────────────────────────────────────────────────────────

pub async fn handle_digest(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    match build_digest(&conn) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Digest failed: {err}") }),
        ),
    }
}

pub fn build_digest(conn: &rusqlite::Connection) -> Result<Value, String> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let today_like = format!("{today}%");

    let total_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'disputed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let new_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE created_at LIKE ?1",
            params![today_like.clone()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let new_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE created_at LIKE ?1",
            params![today_like.clone()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let stores_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'decision_stored' AND created_at LIKE ?1",
            params![today_like.clone()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let conflicts_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'decision_conflict' AND created_at LIKE ?1",
            params![today_like.clone()],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let decayed_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active' AND score < 0.5 AND pinned = 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let decayed_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active' AND score < 0.5 AND pinned = 0",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Top recalled memories
    let mut top_stmt = conn
        .prepare(
            "SELECT source, text, retrievals FROM memories \
             WHERE status = 'active' AND retrievals > 0 \
             ORDER BY retrievals DESC LIMIT 5",
        )
        .map_err(|e| e.to_string())?;
    let top_rows = top_stmt
        .query_map([], |row| {
            Ok(json!({
                "source": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
                "text": truncate_chars(&row.get::<_, String>(1)?, 80),
                "retrievals": row.get::<_, i64>(2)?
            }))
        })
        .map_err(|e| e.to_string())?;
    let top_recalled: Vec<Value> = top_rows.filter_map(|r| r.ok()).collect();

    // Agent boots today
    let mut boots_stmt = conn
        .prepare(
            "SELECT source_agent, COUNT(*) as cnt FROM events \
             WHERE type = 'agent_boot' AND created_at LIKE ?1 \
             GROUP BY source_agent",
        )
        .map_err(|e| e.to_string())?;
    let boots_rows = boots_stmt
        .query_map(params![today_like.clone()], |row| {
            Ok(json!({
                "source_agent": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
                "cnt": row.get::<_, i64>(1)?
            }))
        })
        .map_err(|e| e.to_string())?;
    let agent_boots: Vec<Value> = boots_rows.filter_map(|r| r.ok()).collect();

    // Token savings
    let mut total_saved = 0_i64;
    let mut total_served = 0_i64;
    let mut boot_count = 0_i64;
    let mut today_saved = 0_i64;
    let mut today_served = 0_i64;
    let mut today_boots = 0_i64;

    let mut savings_stmt = conn
        .prepare("SELECT data, created_at FROM events WHERE type = 'boot_savings'")
        .map_err(|e| e.to_string())?;
    let savings_rows = savings_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in savings_rows.flatten() {
        if let (Some(data), created_at) = row {
            if let Ok(parsed) = serde_json::from_str::<Value>(&data) {
                total_saved += parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
                total_served += parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
                boot_count += 1;
                if created_at
                    .as_deref()
                    .map(|v| v.starts_with(&today))
                    .unwrap_or(false)
                {
                    today_saved += parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
                    today_served += parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
                    today_boots += 1;
                }
            }
        }
    }

    // Build oneliner
    let agent_str = if agent_boots.is_empty() {
        "none".to_string()
    } else {
        agent_boots
            .iter()
            .map(|row| {
                format!(
                    "{} ({})",
                    row.get("source_agent")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown"),
                    row.get("cnt").and_then(|v| v.as_i64()).unwrap_or(0)
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    let savings_str = if total_saved > 0 {
        format!(" | Saved: {} tokens ({} boots)", total_saved, boot_count)
    } else {
        String::new()
    };
    let oneliner = format!(
        "Cortex Daily — {today} | Mem: {total_memories} (+{new_memories}) | Dec: {total_decisions} (+{new_decisions}) | Conflicts: {total_conflicts} | Decaying: {} | Agents: {}{savings_str}",
        decayed_memories + decayed_decisions,
        agent_str,
    );

    Ok(json!({
        "date": today,
        "totals": { "memories": total_memories, "decisions": total_decisions, "conflicts": total_conflicts },
        "today": { "newMemories": new_memories, "newDecisions": new_decisions, "stores": stores_today, "conflictsDetected": conflicts_today },
        "tokenSavings": {
            "allTime": { "saved": total_saved, "served": total_served, "boots": boot_count },
            "today": { "saved": today_saved, "served": today_served, "boots": today_boots }
        },
        "topRecalled": top_recalled,
        "decay": { "memories": decayed_memories, "decisions": decayed_decisions },
        "agentBoots": agent_boots,
        "oneliner": oneliner
    }))
}

// ─── GET /savings ────────────────────────────────────────────────────────────

fn value_i64_any(payload: &Value, keys: &[&str]) -> i64 {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(|v| v.as_i64()))
        .unwrap_or(0)
}

fn method_count(payload: &Value, method: &str) -> i64 {
    payload
        .get("method_breakdown")
        .and_then(|value| value.get(method))
        .and_then(|value| value.as_i64())
        .unwrap_or(0)
}

fn classify_recall_tier_from_payload(payload: &Value) -> String {
    if let Some(tier) = payload.get("tier").and_then(|value| value.as_str()) {
        if !tier.trim().is_empty() {
            return tier.to_string();
        }
    }

    if payload
        .get("cached")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        return "cache_hit".to_string();
    }

    let mode = payload
        .get("mode")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if mode == "headlines" {
        return "headlines".to_string();
    }
    if mode == "semantic" {
        return "semantic_only".to_string();
    }

    let keyword = method_count(payload, "keyword");
    let semantic = method_count(payload, "semantic");
    let hybrid = method_count(payload, "hybrid");
    let crystal = method_count(payload, "crystal");

    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        if crystal > 0 {
            return "hybrid_crystal".to_string();
        }
        return "hybrid_fusion".to_string();
    }
    if keyword > 0 {
        if crystal > 0 {
            return "keyword_crystal".to_string();
        }
        return "keyword_only".to_string();
    }
    if semantic > 0 {
        if crystal > 0 {
            return "semantic_crystal".to_string();
        }
        return "semantic_only".to_string();
    }
    if crystal > 0 {
        return "crystal_only".to_string();
    }

    "unknown".to_string()
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn normalize_shadow_status(status: &str) -> &'static str {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "ok" => "ok",
        "unavailable" => "unavailable",
        "error" => "error",
        "skipped" => "skipped",
        _ => "unknown",
    }
}

const SHADOW_GATE_MIN_PROBED_EVENTS: i64 = 25;
const SHADOW_GATE_MIN_OK_SAMPLES: i64 = 15;
const SHADOW_GATE_MAX_UNAVAILABLE_RATE: f64 = 0.35;
const SHADOW_GATE_MAX_ERROR_RATE: f64 = 0.05;
const SHADOW_GATE_MIN_OK_OVERLAP_RATIO: f64 = 0.60;
const SHADOW_GATE_MIN_OK_JACCARD: f64 = 0.45;
const SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA: f64 = 1.25;
const SHADOW_GATE_MIN_TOP1_MATCH_RATE: f64 = 0.60;

struct ShadowOkMetricSamples {
    overlap_ratio: i64,
    jaccard: i64,
    mean_abs_rank_delta: i64,
    top1_match: i64,
}

struct ShadowOkMetricAverages {
    overlap_ratio: Option<f64>,
    jaccard: Option<f64>,
    mean_abs_rank_delta: Option<f64>,
    top1_match_rate: Option<f64>,
}

fn build_shadow_semantic_gate(
    shadow_status_counts: &BTreeMap<String, i64>,
    ok_samples: i64,
    ok_metric_samples: &ShadowOkMetricSamples,
    ok_metric_averages: &ShadowOkMetricAverages,
) -> Value {
    let ok_count = *shadow_status_counts.get("ok").unwrap_or(&0);
    let unavailable_count = *shadow_status_counts.get("unavailable").unwrap_or(&0);
    let error_count = *shadow_status_counts.get("error").unwrap_or(&0);
    let unknown_count = *shadow_status_counts.get("unknown").unwrap_or(&0);
    let skipped_count = *shadow_status_counts.get("skipped").unwrap_or(&0);

    // "Probed" excludes cache-hit skips, because no shadow query was attempted.
    let probed_events = ok_count + unavailable_count + error_count + unknown_count;
    let unavailable_rate = if probed_events > 0 {
        round4(unavailable_count as f64 / probed_events as f64)
    } else {
        0.0
    };
    let error_rate = if probed_events > 0 {
        round4(error_count as f64 / probed_events as f64)
    } else {
        0.0
    };

    let mut blockers: Vec<String> = Vec::new();
    if probed_events < SHADOW_GATE_MIN_PROBED_EVENTS {
        blockers.push("insufficient_shadow_samples".to_string());
    }
    if ok_samples < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_ok_samples".to_string());
    }
    if unavailable_rate > SHADOW_GATE_MAX_UNAVAILABLE_RATE {
        blockers.push("unavailable_rate_above_gate".to_string());
    }
    if error_rate > SHADOW_GATE_MAX_ERROR_RATE {
        blockers.push("error_rate_above_gate".to_string());
    }
    if ok_metric_samples.overlap_ratio > 0
        && ok_metric_samples.overlap_ratio < SHADOW_GATE_MIN_OK_SAMPLES
    {
        blockers.push("insufficient_overlap_ratio_samples".to_string());
    }
    match ok_metric_averages.overlap_ratio {
        Some(value) if value < SHADOW_GATE_MIN_OK_OVERLAP_RATIO => {
            blockers.push("overlap_ratio_below_gate".to_string());
        }
        None => blockers.push("missing_overlap_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.jaccard > 0 && ok_metric_samples.jaccard < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_jaccard_samples".to_string());
    }
    match ok_metric_averages.jaccard {
        Some(value) if value < SHADOW_GATE_MIN_OK_JACCARD => {
            blockers.push("jaccard_below_gate".to_string());
        }
        None => blockers.push("missing_jaccard_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.mean_abs_rank_delta > 0
        && ok_metric_samples.mean_abs_rank_delta < SHADOW_GATE_MIN_OK_SAMPLES
    {
        blockers.push("insufficient_rank_delta_samples".to_string());
    }
    match ok_metric_averages.mean_abs_rank_delta {
        Some(value) if value > SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA => {
            blockers.push("mean_abs_rank_delta_above_gate".to_string());
        }
        None => blockers.push("missing_rank_delta_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.top1_match > 0 && ok_metric_samples.top1_match < SHADOW_GATE_MIN_OK_SAMPLES
    {
        blockers.push("insufficient_top1_match_samples".to_string());
    }
    match ok_metric_averages.top1_match_rate {
        Some(value) if value < SHADOW_GATE_MIN_TOP1_MATCH_RATE => {
            blockers.push("top1_match_rate_below_gate".to_string());
        }
        None => blockers.push("missing_top1_match_signal".to_string()),
        _ => {}
    }

    let ready = blockers.is_empty();
    json!({
        "ready": ready,
        "decision": if ready { "ready_for_vec0_trial" } else { "hold" },
        "target": "sqlite_vec_production_routing",
        "blockers": blockers,
        "metrics": {
            "probed_events": probed_events,
            "ok_count": ok_count,
            "unavailable_count": unavailable_count,
            "error_count": error_count,
            "unknown_count": unknown_count,
            "skipped_count": skipped_count,
            "ok_samples": ok_samples,
            "ok_overlap_samples": ok_metric_samples.overlap_ratio,
            "ok_jaccard_samples": ok_metric_samples.jaccard,
            "ok_rank_delta_samples": ok_metric_samples.mean_abs_rank_delta,
            "ok_top1_match_samples": ok_metric_samples.top1_match,
            "ok_overlap_ratio_avg": ok_metric_averages.overlap_ratio,
            "ok_jaccard_avg": ok_metric_averages.jaccard,
            "ok_mean_abs_rank_delta_avg": ok_metric_averages.mean_abs_rank_delta,
            "ok_top1_match_rate": ok_metric_averages.top1_match_rate,
            "unavailable_rate": unavailable_rate,
            "error_rate": error_rate
        },
        "thresholds": {
            "min_probed_events": SHADOW_GATE_MIN_PROBED_EVENTS,
            "min_ok_samples": SHADOW_GATE_MIN_OK_SAMPLES,
            "max_unavailable_rate": SHADOW_GATE_MAX_UNAVAILABLE_RATE,
            "max_error_rate": SHADOW_GATE_MAX_ERROR_RATE,
            "min_ok_overlap_ratio": SHADOW_GATE_MIN_OK_OVERLAP_RATIO,
            "min_ok_jaccard": SHADOW_GATE_MIN_OK_JACCARD,
            "max_mean_abs_rank_delta": SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA,
            "min_top1_match_rate": SHADOW_GATE_MIN_TOP1_MATCH_RATE
        }
    })
}

fn build_recall_stats_payload_from_rows(rows: &[(String, String)]) -> Value {
    let mut tier_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_sum: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_samples: BTreeMap<String, i64> = BTreeMap::new();
    let mut mode_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut shadow_status_counts: BTreeMap<String, i64> = BTreeMap::new();

    let mut total_budget = 0_i64;
    let mut total_spent = 0_i64;
    let mut total_saved = 0_i64;
    let mut total_hits = 0_i64;
    let mut shadow_ok_overlap_ratio_sum = 0.0_f64;
    let mut shadow_ok_overlap_ratio_samples = 0_i64;
    let mut shadow_ok_jaccard_sum = 0.0_f64;
    let mut shadow_ok_jaccard_samples = 0_i64;
    let mut shadow_ok_rank_delta_sum = 0.0_f64;
    let mut shadow_ok_rank_delta_samples = 0_i64;
    let mut shadow_ok_top1_match_sum = 0.0_f64;
    let mut shadow_ok_top1_match_samples = 0_i64;

    let mut latency_total = 0_i64;
    let mut latency_samples = 0_i64;
    let mut recent: Vec<Value> = Vec::new();

    for (data_str, created_at) in rows {
        let payload: Value = serde_json::from_str(data_str).unwrap_or_else(|_| json!({}));
        let mode = payload
            .get("mode")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string();
        *mode_counts.entry(mode.clone()).or_insert(0) += 1;

        let tier = classify_recall_tier_from_payload(&payload);
        *tier_counts.entry(tier.clone()).or_insert(0) += 1;

        let budget = value_i64_any(&payload, &["budget", "baseline"]);
        let spent = value_i64_any(&payload, &["spent", "served"]);
        let saved = value_i64_any(&payload, &["saved"]);
        let hits = value_i64_any(&payload, &["hits", "results"]);

        total_budget += budget.max(0);
        total_spent += spent.max(0);
        total_saved += saved;
        total_hits += hits.max(0);

        if let Some(latency_ms) = payload.get("latency_ms").and_then(|value| value.as_i64()) {
            if latency_ms >= 0 {
                latency_total += latency_ms;
                latency_samples += 1;
                *tier_latency_sum.entry(tier.clone()).or_insert(0) += latency_ms;
                *tier_latency_samples.entry(tier.clone()).or_insert(0) += 1;
            }
        }
        if let Some(shadow_semantic) = payload
            .get("shadow_semantic")
            .and_then(|value| value.as_object())
        {
            let status = shadow_semantic
                .get("status")
                .and_then(|value| value.as_str())
                .map(normalize_shadow_status)
                .unwrap_or("unknown")
                .to_string();
            *shadow_status_counts.entry(status.clone()).or_insert(0) += 1;

            if status == "ok" {
                if let Some(overlap_ratio) = shadow_semantic
                    .get("overlapRatio")
                    .and_then(|value| value.as_f64())
                {
                    shadow_ok_overlap_ratio_sum += overlap_ratio;
                    shadow_ok_overlap_ratio_samples += 1;
                }
                if let Some(jaccard) = shadow_semantic
                    .get("jaccard")
                    .and_then(|value| value.as_f64())
                {
                    shadow_ok_jaccard_sum += jaccard;
                    shadow_ok_jaccard_samples += 1;
                }
                if let Some(mean_abs_rank_delta) = shadow_semantic
                    .get("meanAbsRankDelta")
                    .and_then(|value| value.as_f64())
                {
                    shadow_ok_rank_delta_sum += mean_abs_rank_delta;
                    shadow_ok_rank_delta_samples += 1;
                }
                if let Some(top1_match) = shadow_semantic
                    .get("top1Match")
                    .and_then(|value| value.as_bool())
                {
                    shadow_ok_top1_match_sum += if top1_match { 1.0 } else { 0.0 };
                    shadow_ok_top1_match_samples += 1;
                }
            }
        }

        recent.push(json!({
            "timestamp": created_at,
            "mode": mode,
            "tier": tier,
            "budget": budget,
            "spent": spent,
            "saved": saved,
            "hits": hits,
            "cached": payload.get("cached").and_then(|value| value.as_bool()).unwrap_or(false),
            "latencyMs": payload.get("latency_ms").and_then(|value| value.as_i64()),
        }));
    }

    let total_recalls = rows.len() as i64;
    let avg_latency_ms = if latency_samples > 0 {
        round1(latency_total as f64 / latency_samples as f64)
    } else {
        0.0
    };
    let savings_pct_vs_budget = if total_budget > 0 {
        round1((total_saved as f64 / total_budget as f64) * 100.0)
    } else {
        0.0
    };
    let shadow_overlap_ratio_avg = if shadow_ok_overlap_ratio_samples > 0 {
        Some(round4(
            shadow_ok_overlap_ratio_sum / shadow_ok_overlap_ratio_samples as f64,
        ))
    } else {
        None
    };
    let shadow_jaccard_avg = if shadow_ok_jaccard_samples > 0 {
        Some(round4(
            shadow_ok_jaccard_sum / shadow_ok_jaccard_samples as f64,
        ))
    } else {
        None
    };
    let shadow_mean_abs_rank_delta_avg = if shadow_ok_rank_delta_samples > 0 {
        Some(round4(
            shadow_ok_rank_delta_sum / shadow_ok_rank_delta_samples as f64,
        ))
    } else {
        None
    };
    let shadow_top1_match_rate = if shadow_ok_top1_match_samples > 0 {
        Some(round4(
            shadow_ok_top1_match_sum / shadow_ok_top1_match_samples as f64,
        ))
    } else {
        None
    };
    let ok_metric_samples = ShadowOkMetricSamples {
        overlap_ratio: shadow_ok_overlap_ratio_samples,
        jaccard: shadow_ok_jaccard_samples,
        mean_abs_rank_delta: shadow_ok_rank_delta_samples,
        top1_match: shadow_ok_top1_match_samples,
    };
    let ok_metric_averages = ShadowOkMetricAverages {
        overlap_ratio: shadow_overlap_ratio_avg,
        jaccard: shadow_jaccard_avg,
        mean_abs_rank_delta: shadow_mean_abs_rank_delta_avg,
        top1_match_rate: shadow_top1_match_rate,
    };
    let shadow_ok_samples = [
        ok_metric_samples.overlap_ratio,
        ok_metric_samples.jaccard,
        ok_metric_samples.mean_abs_rank_delta,
        ok_metric_samples.top1_match,
    ]
    .into_iter()
    .min()
    .unwrap_or(0);
    let shadow_gate = build_shadow_semantic_gate(
        &shadow_status_counts,
        shadow_ok_samples,
        &ok_metric_samples,
        &ok_metric_averages,
    );

    let tier_distribution: Vec<Value> = tier_counts
        .iter()
        .map(|(tier, count)| {
            let percent = if total_recalls > 0 {
                round1((*count as f64 / total_recalls as f64) * 100.0)
            } else {
                0.0
            };
            let avg_tier_latency = match (
                tier_latency_sum.get(tier).copied(),
                tier_latency_samples.get(tier).copied(),
            ) {
                (Some(sum), Some(samples)) if samples > 0 => round1(sum as f64 / samples as f64),
                _ => 0.0,
            };
            json!({
                "tier": tier,
                "count": count,
                "percent": percent,
                "avgLatencyMs": avg_tier_latency
            })
        })
        .collect();

    let tier_distribution_map: Value = json!(tier_counts
        .iter()
        .map(|(tier, count)| (tier.clone(), json!(count)))
        .collect::<serde_json::Map<String, Value>>());

    let avg_latency_map: Value = {
        let mut map = serde_json::Map::new();
        map.insert("overall".to_string(), json!(avg_latency_ms));
        for entry in &tier_distribution {
            if let (Some(tier), Some(avg)) = (
                entry.get("tier").and_then(|value| value.as_str()),
                entry.get("avgLatencyMs"),
            ) {
                map.insert(tier.to_string(), avg.clone());
            }
        }
        Value::Object(map)
    };

    recent.sort_by(|a, b| {
        let a_ts = a
            .get("timestamp")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let b_ts = b
            .get("timestamp")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        b_ts.cmp(a_ts)
    });
    recent.truncate(30);

    json!({
        "summary": {
            "totalRecalls": total_recalls,
            "totalHits": total_hits,
            "totalBudget": total_budget,
            "totalSpent": total_spent,
            "totalSaved": total_saved,
            "savingsPctVsBudget": savings_pct_vs_budget,
            "avgLatencyMs": avg_latency_ms
        },
        "tierDistribution": tier_distribution,
        "tier_distribution": tier_distribution_map,
        "avg_latency_ms": avg_latency_map,
        "estimated_savings": {
            "vs_always_full_pipeline_pct": savings_pct_vs_budget
        },
        "modeCounts": mode_counts,
        "shadow_semantic": {
            "status_counts": shadow_status_counts,
            "ok_samples": shadow_ok_samples,
            "ok_overlap_samples": ok_metric_samples.overlap_ratio,
            "ok_jaccard_samples": ok_metric_samples.jaccard,
            "ok_rank_delta_samples": ok_metric_samples.mean_abs_rank_delta,
            "ok_top1_match_samples": ok_metric_samples.top1_match,
            "ok_overlap_ratio_avg": ok_metric_averages.overlap_ratio,
            "ok_jaccard_avg": ok_metric_averages.jaccard,
            "ok_mean_abs_rank_delta_avg": ok_metric_averages.mean_abs_rank_delta,
            "ok_top1_match_rate": ok_metric_averages.top1_match_rate
        },
        "shadowSemantic": {
            "statusCounts": shadow_status_counts,
            "okSamples": shadow_ok_samples,
            "okOverlapSamples": ok_metric_samples.overlap_ratio,
            "okJaccardSamples": ok_metric_samples.jaccard,
            "okRankDeltaSamples": ok_metric_samples.mean_abs_rank_delta,
            "okTop1MatchSamples": ok_metric_samples.top1_match,
            "okOverlapRatioAvg": ok_metric_averages.overlap_ratio,
            "okJaccardAvg": ok_metric_averages.jaccard,
            "okMeanAbsRankDeltaAvg": ok_metric_averages.mean_abs_rank_delta,
            "okTop1MatchRate": ok_metric_averages.top1_match_rate
        },
        "shadow_semantic_gate": shadow_gate,
        "shadowSemanticGate": {
            "ready": shadow_gate["ready"],
            "decision": shadow_gate["decision"],
            "target": shadow_gate["target"],
            "blockers": shadow_gate["blockers"],
            "metrics": {
                "probedEvents": shadow_gate["metrics"]["probed_events"],
                "okCount": shadow_gate["metrics"]["ok_count"],
                "unavailableCount": shadow_gate["metrics"]["unavailable_count"],
                "errorCount": shadow_gate["metrics"]["error_count"],
                "unknownCount": shadow_gate["metrics"]["unknown_count"],
                "skippedCount": shadow_gate["metrics"]["skipped_count"],
                "okSamples": shadow_gate["metrics"]["ok_samples"],
                "okOverlapSamples": shadow_gate["metrics"]["ok_overlap_samples"],
                "okJaccardSamples": shadow_gate["metrics"]["ok_jaccard_samples"],
                "okRankDeltaSamples": shadow_gate["metrics"]["ok_rank_delta_samples"],
                "okTop1MatchSamples": shadow_gate["metrics"]["ok_top1_match_samples"],
                "okOverlapRatioAvg": shadow_gate["metrics"]["ok_overlap_ratio_avg"],
                "okJaccardAvg": shadow_gate["metrics"]["ok_jaccard_avg"],
                "okMeanAbsRankDeltaAvg": shadow_gate["metrics"]["ok_mean_abs_rank_delta_avg"],
                "okTop1MatchRate": shadow_gate["metrics"]["ok_top1_match_rate"],
                "unavailableRate": shadow_gate["metrics"]["unavailable_rate"],
                "errorRate": shadow_gate["metrics"]["error_rate"]
            },
            "thresholds": {
                "minProbedEvents": shadow_gate["thresholds"]["min_probed_events"],
                "minOkSamples": shadow_gate["thresholds"]["min_ok_samples"],
                "maxUnavailableRate": shadow_gate["thresholds"]["max_unavailable_rate"],
                "maxErrorRate": shadow_gate["thresholds"]["max_error_rate"],
                "minOkOverlapRatio": shadow_gate["thresholds"]["min_ok_overlap_ratio"],
                "minOkJaccard": shadow_gate["thresholds"]["min_ok_jaccard"],
                "maxMeanAbsRankDelta": shadow_gate["thresholds"]["max_mean_abs_rank_delta"],
                "minTop1MatchRate": shadow_gate["thresholds"]["min_top1_match_rate"]
            }
        },
        "recent": recent
    })
}

pub async fn handle_savings(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let now_unix_secs = Utc::now().timestamp();
    let cached_snapshot = match savings_payload_cache().lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    if let Some(snapshot) = savings_payload_cache_if_fresh(cached_snapshot, now_unix_secs) {
        return json_response(StatusCode::OK, snapshot.payload);
    }

    let conn = state.db_read.lock().await;

    let mut stmt = match conn.prepare(
        "SELECT data, created_at FROM events WHERE type = 'boot_savings' ORDER BY created_at ASC",
    ) {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": e.to_string() }),
            );
        }
    };

    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            let data_str: String = row.get(0)?;
            let created: String = row.get(1)?;
            Ok((data_str, created))
        })
        .map(|iter| iter.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    drop(stmt);

    let mut by_operation: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for op in ["recall", "store", "boot", "tool"] {
        by_operation.insert(op.to_string(), (0, 0, 0, 0));
    }

    let boot_agg = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events WHERE type = 'boot_savings'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    by_operation.insert("boot".to_string(), boot_agg);

    let recall_agg = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.spent') AS INTEGER), COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0))), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.budget') AS INTEGER), COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0))), 0), \
                 COUNT(*) \
             FROM events WHERE type = 'recall_query'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    by_operation.insert("recall".to_string(), recall_agg);

    let mut store_agg = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events WHERE type = 'store_savings'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    let store_decision_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type IN ('decision_stored', 'decision_supersede', 'decision_conflict', 'decision_rejected_duplicate')",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    store_agg.3 += store_decision_events;
    by_operation.insert("store".to_string(), store_agg);

    let tool_agg = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events WHERE type = 'tool_call_savings'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    by_operation.insert("tool".to_string(), tool_agg);

    let mut daily_savings_all: BTreeMap<String, i64> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT \
             SUBSTR(created_at, 1, 10) AS day, \
             COALESCE(SUM(CASE \
                 WHEN type = 'boot_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 ELSE 0 END), 0) AS saved_delta \
         FROM events \
         WHERE type IN ('boot_savings', 'recall_query', 'store_savings', 'tool_call_savings') \
           AND created_at IS NOT NULL \
         GROUP BY day \
         ORDER BY day ASC",
    ) {
        let rows = stmt
            .query_map([], |row| {
                let day: Option<String> = row.get(0)?;
                let saved_delta: i64 = row.get(1)?;
                Ok((day.unwrap_or_default(), saved_delta))
            })
            .map(|iter| iter.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        for (day, saved_delta) in rows {
            if !day.is_empty() {
                daily_savings_all.insert(day, saved_delta);
            }
        }
    }

    let mut recall_daily: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT \
             SUBSTR(created_at, 1, 10) AS day, \
             SUM(CASE WHEN COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 1 ELSE 0 END) AS hits, \
             SUM(CASE WHEN COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 0 ELSE 1 END) AS misses \
         FROM events \
         WHERE type = 'recall_query' AND created_at IS NOT NULL \
         GROUP BY day \
         ORDER BY day ASC",
    ) {
        let rows = stmt
            .query_map([], |row| {
                let day: Option<String> = row.get(0)?;
                let hits: i64 = row.get(1)?;
                let misses: i64 = row.get(2)?;
                Ok((day.unwrap_or_default(), hits, misses))
            })
            .map(|iter| iter.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        for (day, hits, misses) in rows {
            if !day.is_empty() {
                recall_daily.insert(day, (hits, misses));
            }
        }
    }

    let mut activity_heatmap_map: BTreeMap<(String, i64), i64> = BTreeMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT \
             CAST(strftime('%w', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER) AS weekday, \
             CAST(strftime('%H', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER) AS hour, \
             COUNT(*) AS cnt \
         FROM events \
         WHERE created_at IS NOT NULL \
         GROUP BY weekday, hour",
    ) {
        let rows = stmt
            .query_map([], |row| {
                let weekday: Option<i64> = row.get(0)?;
                let hour: Option<i64> = row.get(1)?;
                let count: i64 = row.get(2)?;
                Ok((weekday, hour, count))
            })
            .map(|iter| iter.filter_map(|row| row.ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        for (weekday, hour, count) in rows {
            if let (Some(day), Some(hour)) = (weekday, hour) {
                let day_name = weekday_name_from_sqlite(day).to_string();
                *activity_heatmap_map.entry((day_name, hour)).or_insert(0) += count;
            }
        }
    }

    drop(conn);

    let points: Vec<Value> = rows
        .into_iter()
        .map(|(data_str, created)| {
            let d: Value = serde_json::from_str(&data_str).unwrap_or(json!({}));
            let served = d.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
            let baseline = d.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0);
            let saved = d.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
            let percent = d.get("percent").and_then(|v| v.as_i64()).unwrap_or(0);
            let admitted = d.get("admitted").and_then(|v| v.as_i64()).unwrap_or(0);
            let rejected = d.get("rejected").and_then(|v| v.as_i64()).unwrap_or(0);
            let compression_ratio = if served > 0 {
                ((baseline as f64 / served as f64) * 100.0).round() / 100.0
            } else {
                0.0
            };
            json!({
                "timestamp": created,
                "agent": d.get("agent").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "served": served,
                "baseline": baseline,
                "saved": saved,
                "percent": percent,
                "admitted": admitted,
                "rejected": rejected,
                "compressionRatio": compression_ratio
            })
        })
        .collect();

    let total_saved: i64 = points
        .iter()
        .map(|p| p["saved"].as_i64().unwrap_or(0))
        .sum();
    let total_served: i64 = points
        .iter()
        .map(|p| p["served"].as_i64().unwrap_or(0))
        .sum();
    let total_baseline: i64 = points
        .iter()
        .map(|p| p["baseline"].as_i64().unwrap_or(0))
        .sum();
    // Weighted average by baseline (not simple average).
    // Prevents tiny boots with 0% from dragging down 99% large boots.
    let avg_percent = if total_baseline > 0 {
        (total_saved * 100) / total_baseline
    } else {
        0
    };
    let total_boots = points.len() as i64;
    let avg_saved_per_boot = if total_boots > 0 {
        total_saved / total_boots
    } else {
        0
    };
    let avg_served_per_boot = if total_boots > 0 {
        total_served / total_boots
    } else {
        0
    };
    let avg_baseline_per_boot = if total_boots > 0 {
        total_baseline / total_boots
    } else {
        0
    };

    // Daily aggregation
    let mut daily: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for p in &points {
        let ts = p["timestamp"].as_str().unwrap_or("");
        let day = &ts[..ts.len().min(10)];
        if day.is_empty() {
            continue;
        }
        let e = daily.entry(day.to_string()).or_insert((0, 0, 0, 0));
        e.0 += p["saved"].as_i64().unwrap_or(0);
        e.1 += p["served"].as_i64().unwrap_or(0);
        e.2 += p["baseline"].as_i64().unwrap_or(0);
        e.3 += 1;
    }
    let daily_arr: Vec<Value> = daily
        .into_iter()
        .map(|(date, (saved, served, baseline, boots))| {
            json!({"date": date, "saved": saved, "served": served, "baseline": baseline, "boots": boots})
        })
        .collect();

    let mut by_agent: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for p in &points {
        let agent = p["agent"].as_str().unwrap_or("unknown").to_string();
        let e = by_agent.entry(agent).or_insert((0, 0, 0, 0));
        e.0 += p["saved"].as_i64().unwrap_or(0);
        e.1 += p["served"].as_i64().unwrap_or(0);
        e.2 += p["baseline"].as_i64().unwrap_or(0);
        e.3 += 1;
    }
    let by_agent_arr: Vec<Value> = by_agent
        .into_iter()
        .map(|(agent, (saved, served, baseline, boots))| {
            let percent = if baseline > 0 {
                (saved * 100) / baseline
            } else {
                0
            };
            json!({
                "agent": agent,
                "saved": saved,
                "served": served,
                "baseline": baseline,
                "boots": boots,
                "percent": percent
            })
        })
        .collect();

    let recent: Vec<Value> = points.iter().rev().take(20).cloned().collect();

    let by_operation_arr: Vec<Value> = ["recall", "store", "boot", "tool"]
        .iter()
        .map(|op| {
            let (saved, served, baseline, events) =
                by_operation.get(*op).copied().unwrap_or((0, 0, 0, 0));
            let percent = if baseline > 0 {
                (saved * 100) / baseline
            } else {
                0
            };
            json!({
                "operation": op,
                "saved": saved,
                "served": served,
                "baseline": baseline,
                "events": events,
                "percent": percent
            })
        })
        .collect();

    let mut running_saved = 0_i64;
    let cumulative: Vec<Value> = daily_savings_all
        .into_iter()
        .map(|(date, saved_delta)| {
            running_saved += saved_delta;
            json!({
                "date": date,
                "savedDelta": saved_delta,
                "savedTotal": running_saved
            })
        })
        .collect();

    let recall_trend: Vec<Value> = recall_daily
        .into_iter()
        .map(|(date, (hits, misses))| {
            let queries = hits + misses;
            let hit_rate = if queries > 0 {
                ((hits as f64 / queries as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            json!({
                "date": date,
                "hits": hits,
                "misses": misses,
                "queries": queries,
                "hitRatePct": hit_rate
            })
        })
        .collect();

    let activity_heatmap: Vec<Value> = activity_heatmap_map
        .into_iter()
        .map(|((day, hour), count)| {
            json!({
                "day": day,
                "hour": hour,
                "count": count
            })
        })
        .collect();

    let payload = json!({
        "summary": {
            "totalSaved": total_saved,
            "totalServed": total_served,
            "totalBaseline": total_baseline,
            "avgPercent": avg_percent,
            "totalBoots": points.len(),
            "avgSavedPerBoot": avg_saved_per_boot,
            "avgServedPerBoot": avg_served_per_boot,
            "avgBaselinePerBoot": avg_baseline_per_boot,
            "scope": "boot_prompt_plus_event_operations",
            "note": "Boot savings are precise from /boot events. Recall/store/tool figures are event-derived estimates when instrumentation is available."
        },
        "daily": daily_arr,
        "byAgent": by_agent_arr,
        "recent": recent,
        "byOperation": by_operation_arr,
        "cumulative": cumulative,
        "recallTrend": recall_trend,
        "activityHeatmap": activity_heatmap,
    });

    if let Ok(mut cache) = savings_payload_cache().lock() {
        *cache = Some(SavingsPayloadSnapshot {
            computed_at_unix_secs: now_unix_secs,
            payload: payload.clone(),
        });
    }

    json_response(StatusCode::OK, payload)
}

pub async fn handle_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let conn = state.db_read.lock().await;
    let mut stmt = match conn.prepare(
        "SELECT data, created_at FROM events WHERE type = 'recall_query' ORDER BY created_at ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": e.to_string() }),
            );
        }
    };

    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            let data_str: String = row.get(0)?;
            let created_at: Option<String> = row.get(1)?;
            Ok((data_str, created_at.unwrap_or_default()))
        })
        .map(|iter| iter.filter_map(|row| row.ok()).collect())
        .unwrap_or_default();

    json_response(StatusCode::OK, build_recall_stats_payload_from_rows(&rows))
}

// ─── GET /dump ───────────────────────────────────────────────────────────────

pub async fn handle_dump(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let conn = state.db_read.lock().await;

    let memories: Vec<Value> = conn
        .prepare(
            "SELECT id, text, source, type, tags, source_agent, confidence, status, score, \
             retrievals, last_accessed, pinned, disputes_id, supersedes_id, confirmed_by, \
             created_at, updated_at \
             FROM memories WHERE status = 'active' ORDER BY score DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "text": row.get::<_, String>(1).unwrap_or_default(),
                    "source": row.get::<_, Option<String>>(2).unwrap_or(None),
                    "type": row.get::<_, String>(3).unwrap_or_default(),
                    "tags": row.get::<_, Option<String>>(4).unwrap_or(None),
                    "source_agent": row.get::<_, Option<String>>(5).unwrap_or(None),
                    "confidence": row.get::<_, Option<f64>>(6).unwrap_or(Some(0.8)),
                    "status": row.get::<_, Option<String>>(7).unwrap_or(Some("active".to_string())),
                    "score": row.get::<_, Option<f64>>(8).unwrap_or(Some(1.0)),
                    "retrievals": row.get::<_, Option<i64>>(9).unwrap_or(Some(0)),
                    "last_accessed": row.get::<_, Option<String>>(10).unwrap_or(None),
                    "pinned": row.get::<_, Option<i64>>(11).unwrap_or(Some(0)),
                    "disputes_id": row.get::<_, Option<i64>>(12).unwrap_or(None),
                    "supersedes_id": row.get::<_, Option<i64>>(13).unwrap_or(None),
                    "confirmed_by": row.get::<_, Option<String>>(14).unwrap_or(None),
                    "created_at": row.get::<_, Option<String>>(15).unwrap_or(None),
                    "updated_at": row.get::<_, Option<String>>(16).unwrap_or(None),
                }))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let decisions: Vec<Value> = conn
        .prepare(
            "SELECT id, decision, context, type, source_agent, confidence, surprise, status, \
             score, retrievals, last_accessed, pinned, parent_id, disputes_id, supersedes_id, \
             confirmed_by, created_at, updated_at \
             FROM decisions WHERE status = 'active' ORDER BY score DESC",
        )
        .and_then(|mut stmt| {
            stmt.query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "decision": row.get::<_, String>(1).unwrap_or_default(),
                    "context": row.get::<_, Option<String>>(2).unwrap_or(None),
                    "type": row.get::<_, Option<String>>(3).unwrap_or(Some("decision".to_string())),
                    "source_agent": row.get::<_, Option<String>>(4).unwrap_or(None),
                    "confidence": row.get::<_, Option<f64>>(5).unwrap_or(Some(0.8)),
                    "surprise": row.get::<_, Option<f64>>(6).unwrap_or(Some(1.0)),
                    "status": row.get::<_, Option<String>>(7).unwrap_or(Some("active".to_string())),
                    "score": row.get::<_, Option<f64>>(8).unwrap_or(Some(1.0)),
                    "retrievals": row.get::<_, Option<i64>>(9).unwrap_or(Some(0)),
                    "last_accessed": row.get::<_, Option<String>>(10).unwrap_or(None),
                    "pinned": row.get::<_, Option<i64>>(11).unwrap_or(Some(0)),
                    "parent_id": row.get::<_, Option<i64>>(12).unwrap_or(None),
                    "disputes_id": row.get::<_, Option<i64>>(13).unwrap_or(None),
                    "supersedes_id": row.get::<_, Option<i64>>(14).unwrap_or(None),
                    "confirmed_by": row.get::<_, Option<String>>(15).unwrap_or(None),
                    "created_at": row.get::<_, Option<String>>(16).unwrap_or(None),
                    "updated_at": row.get::<_, Option<String>>(17).unwrap_or(None),
                }))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let mut source_nodes: BTreeMap<String, String> = BTreeMap::new();
    for memory in &memories {
        let Some(id) = memory.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(source) = memory
            .get("source")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        source_nodes
            .entry(source.to_string())
            .or_insert_with(|| format!("mem-{id}"));
    }
    for decision in &decisions {
        let Some(id) = decision.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(source) = decision
            .get("context")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        source_nodes
            .entry(source.to_string())
            .or_insert_with(|| format!("dec-{id}"));
    }

    let mut seen_links: HashSet<String> = HashSet::new();
    let mut graph_links: Vec<Value> = Vec::new();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT source_a, source_b, count, last_seen
         FROM co_occurrence
         ORDER BY count DESC, last_seen DESC
         LIMIT 240",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) {
            for row in rows.flatten() {
                let (source_a, source_b, count, last_seen) = row;
                let Some(node_a) = source_nodes.get(&source_a) else {
                    continue;
                };
                let Some(node_b) = source_nodes.get(&source_b) else {
                    continue;
                };
                if node_a == node_b {
                    continue;
                }
                let (left, right) = if node_a <= node_b {
                    (node_a.clone(), node_b.clone())
                } else {
                    (node_b.clone(), node_a.clone())
                };
                let key = format!("{left}|{right}|co_occurrence");
                if !seen_links.insert(key) {
                    continue;
                }
                graph_links.push(json!({
                    "source": left,
                    "target": right,
                    "type": "co_occurrence",
                    "weight": count,
                    "lastSeen": last_seen,
                }));
            }
        }
    }

    for decision in &decisions {
        let Some(id) = decision.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(disputes_id) = decision.get("disputes_id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let left = format!("dec-{id}");
        let right = format!("dec-{disputes_id}");
        let (source, target) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let key = format!("{source}|{target}|conflict");
        if !seen_links.insert(key) {
            continue;
        }
        graph_links.push(json!({
            "source": source,
            "target": target,
            "type": "conflict",
            "weight": 1,
        }));
    }

    json_response(
        StatusCode::OK,
        json!({
            "memories": memories,
            "decisions": decisions,
            "graph": {
                "links": graph_links,
                "nodeCount": memories.len() + decisions.len(),
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_health_{name}_{unique}"))
    }

    #[test]
    fn collect_storage_metrics_reports_storage_backup_count_and_log_bytes() {
        let home_dir = temp_test_dir("storage_metrics");
        let backup_dir = home_dir.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        fs::write(backup_dir.join("cortex-a.db"), b"1234").unwrap();
        fs::write(backup_dir.join("cortex-b.db"), b"56").unwrap();
        fs::write(backup_dir.join("ignore.txt"), b"zzz").unwrap();
        fs::write(home_dir.join("daemon.log"), b"abcd").unwrap();
        fs::write(home_dir.join("daemon.log.1"), b"ef").unwrap();

        let (storage_bytes, backup_count, log_bytes) = collect_storage_metrics(&home_dir);

        assert_eq!(backup_count, 2);
        assert_eq!(log_bytes, 6);
        assert_eq!(storage_bytes, 15);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn collect_embedding_inventory_reports_model_mix_and_backlog() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('m-active-current', 'memory::current', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let memory_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('m-active-other', 'memory::other', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let memory_other_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('d-unknown-model', 'ctx::unknown', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let decision_unknown_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('d-missing-embedding', 'ctx::missing', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let decision_missing_id = conn.last_insert_rowid();

        let blob = crate::embeddings::vector_to_blob(&[0.2, 0.4, 0.6]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
            params![memory_current_id, blob.clone()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            params![memory_other_id, blob.clone()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, NULL)",
            params![decision_unknown_id, blob],
        )
        .unwrap();

        let metrics = collect_embedding_inventory(&conn, "all-minilm-l6-v2");
        assert_eq!(metrics.active_model_embeddings, 1);
        assert_eq!(metrics.other_model_embeddings, 1);
        assert_eq!(metrics.unknown_model_embeddings, 1);
        assert_eq!(metrics.backlog_memories, 1);
        assert_eq!(metrics.backlog_decisions, 2);
        assert!(
            decision_missing_id > 0,
            "decision without embedding should contribute to backlog"
        );
    }

    #[test]
    fn cache_snapshot_if_fresh_enforces_ttl() {
        let now = Utc::now().timestamp();
        let fresh = HealthHeavyMetricsSnapshot {
            computed_at_unix_secs: now - 2,
            embedding_inventory: EmbeddingInventoryMetrics::default(),
            storage_bytes: 10,
            backup_count: 1,
            log_bytes: 2,
        };
        let stale = HealthHeavyMetricsSnapshot {
            computed_at_unix_secs: now - (HEALTH_HEAVY_CACHE_TTL_SECS + 5),
            embedding_inventory: EmbeddingInventoryMetrics::default(),
            storage_bytes: 10,
            backup_count: 1,
            log_bytes: 2,
        };

        assert!(cache_snapshot_if_fresh(Some(fresh), now).is_some());
        assert!(cache_snapshot_if_fresh(Some(stale), now).is_none());
        assert!(cache_snapshot_if_fresh(None, now).is_none());
    }

    #[test]
    fn savings_payload_cache_if_fresh_enforces_ttl() {
        let now = Utc::now().timestamp();
        let fresh = SavingsPayloadSnapshot {
            computed_at_unix_secs: now - 1,
            payload: json!({ "ok": true }),
        };
        let stale = SavingsPayloadSnapshot {
            computed_at_unix_secs: now - (SAVINGS_CACHE_TTL_SECS + 5),
            payload: json!({ "ok": false }),
        };

        assert!(savings_payload_cache_if_fresh(Some(fresh), now).is_some());
        assert!(savings_payload_cache_if_fresh(Some(stale), now).is_none());
        assert!(savings_payload_cache_if_fresh(None, now).is_none());
    }

    #[test]
    fn is_control_center_owner_is_case_insensitive() {
        assert!(is_control_center_owner(Some("control-center")));
        assert!(is_control_center_owner(Some("CoNtRoL-CeNtEr")));
        assert!(!is_control_center_owner(Some("plugin-codex")));
        assert!(!is_control_center_owner(None));
    }

    #[test]
    fn build_recall_stats_payload_summarizes_tiers_and_latency() {
        let rows = vec![
            (
                json!({
                    "mode": "balanced",
                    "budget": 200,
                    "spent": 60,
                    "saved": 140,
                    "hits": 2,
                    "cached": false,
                    "method_breakdown": { "keyword": 2 },
                    "tier": "keyword_only",
                    "latency_ms": 5,
                    "shadow_semantic": {
                        "status": "unavailable",
                        "reason": "query_embedding_unavailable"
                    }
                })
                .to_string(),
                "2026-04-14T10:00:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "full",
                    "budget": 300,
                    "spent": 220,
                    "saved": 80,
                    "hits": 3,
                    "cached": false,
                    "method_breakdown": { "hybrid": 2, "semantic": 1 },
                    "tier": "hybrid_fusion",
                    "latency_ms": 28,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.5,
                        "jaccard": 0.4,
                        "meanAbsRankDelta": 0.75,
                        "top1Match": true
                    }
                })
                .to_string(),
                "2026-04-14T10:01:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 180,
                    "spent": 0,
                    "saved": 180,
                    "hits": 1,
                    "cached": true,
                    "tier": "cache_hit",
                    "latency_ms": 1,
                    "shadow_semantic": {
                        "status": "skipped",
                        "reason": "cache_hit"
                    }
                })
                .to_string(),
                "2026-04-14T10:02:00Z".to_string(),
            ),
        ];

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["summary"]["totalRecalls"], 3);
        assert_eq!(payload["summary"]["totalBudget"], 680);
        assert_eq!(payload["summary"]["totalSpent"], 280);
        assert_eq!(payload["summary"]["totalSaved"], 400);
        assert_eq!(payload["tier_distribution"]["cache_hit"], 1);
        assert_eq!(payload["tier_distribution"]["keyword_only"], 1);
        assert_eq!(payload["tier_distribution"]["hybrid_fusion"], 1);
        assert_eq!(payload["avg_latency_ms"]["overall"], 11.3);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["unavailable"],
            1
        );
        assert_eq!(payload["shadow_semantic"]["status_counts"]["ok"], 1);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["skipped"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_rank_delta_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_ratio_avg"], 0.5);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_avg"], 0.4);
        assert_eq!(
            payload["shadow_semantic"]["ok_mean_abs_rank_delta_avg"],
            0.75
        );
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_rate"], 1.0);
        assert_eq!(payload["shadowSemantic"]["statusCounts"]["ok"], 1);
        assert_eq!(payload["shadowSemantic"]["okOverlapRatioAvg"], 0.5);
        assert_eq!(payload["shadowSemantic"]["okMeanAbsRankDeltaAvg"], 0.75);
        assert_eq!(payload["shadowSemantic"]["okTop1MatchRate"], 1.0);
        assert_eq!(payload["shadowSemantic"]["okOverlapSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okJaccardSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okRankDeltaSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okTop1MatchSamples"], 1);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert!(payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be an array")
            .iter()
            .any(|value| value.as_str() == Some("insufficient_shadow_samples")));
        assert_eq!(payload["shadowSemanticGate"]["decision"], "hold");
        assert_eq!(
            payload["estimated_savings"]["vs_always_full_pipeline_pct"],
            58.8
        );
    }

    #[test]
    fn build_recall_stats_payload_reports_ready_shadow_semantic_gate() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.72,
                        "jaccard": 0.61,
                        "meanAbsRankDelta": 0.42,
                        "top1Match": true
                    }
                })
                .to_string(),
                format!("2026-04-14T10:{idx:02}:00Z"),
            ));
        }
        rows.push((
            json!({
                "mode": "balanced",
                "budget": 220,
                "spent": 110,
                "saved": 110,
                "hits": 2,
                "cached": false,
                "tier": "hybrid_fusion",
                "latency_ms": 9,
                "shadow_semantic": {
                    "status": "unavailable",
                    "reason": "query_embedding_unavailable"
                }
            })
            .to_string(),
            "2026-04-14T11:00:00Z".to_string(),
        ));
        rows.push((
            json!({
                "mode": "balanced",
                "budget": 220,
                "spent": 100,
                "saved": 120,
                "hits": 2,
                "cached": false,
                "tier": "hybrid_fusion",
                "latency_ms": 8,
                "shadow_semantic": {
                    "status": "error",
                    "reason": "transient_probe_failure"
                }
            })
            .to_string(),
            "2026-04-14T11:01:00Z".to_string(),
        ));

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["ok"], 30,
            "ok status count should include all successful probes"
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["decision"],
            "ready_for_vec0_trial"
        );
        assert_eq!(payload["shadow_semantic_gate"]["ready"], true);
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["probed_events"],
            32
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["unavailable_rate"],
            0.0313
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["error_rate"],
            0.0313
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_ratio_avg"],
            0.72
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_jaccard_avg"],
            0.61
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_mean_abs_rank_delta_avg"],
            0.42
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_rate"],
            1.0
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_jaccard_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_rank_delta_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_samples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["decision"],
            "ready_for_vec0_trial"
        );
        assert_eq!(payload["shadowSemanticGate"]["metrics"]["probedEvents"], 32);
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okMeanAbsRankDeltaAvg"],
            0.42
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okTop1MatchRate"],
            1.0
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okOverlapSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okJaccardSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okRankDeltaSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okTop1MatchSamples"],
            30
        );
    }

    #[test]
    fn build_recall_stats_payload_holds_shadow_gate_for_rank_drift() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.78,
                        "jaccard": 0.68,
                        "meanAbsRankDelta": 2.2,
                        "top1Match": false
                    }
                })
                .to_string(),
                format!("2026-04-14T12:{idx:02}:00Z"),
            ));
        }

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        let blockers = payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be present");
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("mean_abs_rank_delta_above_gate")),
            "rank-delta blocker should be present"
        );
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("top1_match_rate_below_gate")),
            "top1-match blocker should be present"
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_mean_abs_rank_delta_avg"],
            2.2
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_rate"],
            0.0
        );
    }

    #[test]
    fn build_recall_stats_payload_holds_shadow_gate_for_under_sampled_ok_metrics() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            let include_rank_signals = idx < 10;
            let mut shadow = json!({
                "status": "ok",
                "overlapRatio": 0.74,
                "jaccard": 0.62
            });
            if include_rank_signals {
                shadow["meanAbsRankDelta"] = json!(0.33);
                shadow["top1Match"] = json!(true);
            }
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": shadow
                })
                .to_string(),
                format!("2026-04-14T13:{idx:02}:00Z"),
            ));
        }

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        let blockers = payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be present");
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("insufficient_rank_delta_samples")),
            "rank-delta sample blocker should be present"
        );
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("insufficient_top1_match_samples")),
            "top1-match sample blocker should be present"
        );
        assert_eq!(payload["shadow_semantic"]["ok_samples"], 10);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_samples"], 30);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_samples"], 30);
        assert_eq!(payload["shadow_semantic"]["ok_rank_delta_samples"], 10);
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_samples"], 10);
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_rank_delta_samples"],
            10
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okOverlapSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okRankDeltaSamples"],
            10
        );
    }

    #[test]
    fn build_recall_stats_payload_normalizes_shadow_status_buckets() {
        let rows = vec![
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "OK", "overlapRatio": 0.7, "jaccard": 0.6, "meanAbsRankDelta": 0.5, "top1Match": true }
                })
                .to_string(),
                "2026-04-14T14:00:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": " UnAvailable " }
                })
                .to_string(),
                "2026-04-14T14:01:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "SKIPPED" }
                })
                .to_string(),
                "2026-04-14T14:02:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "mystery" }
                })
                .to_string(),
                "2026-04-14T14:03:00Z".to_string(),
            ),
        ];

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["ok"], 1);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["unavailable"],
            1
        );
        assert_eq!(payload["shadow_semantic"]["status_counts"]["skipped"], 1);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["unknown"], 1);
    }
}
