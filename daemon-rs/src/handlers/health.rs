// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{NaiveDateTime, Timelike, Utc};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

use super::{ensure_auth, json_response, truncate_chars};
use crate::state::RuntimeState;

const STORAGE_LOG_FILES: &[&str] = &[
    "daemon.log",
    "daemon.err.log",
    "daemon.out.log",
    "mcp-crash.log",
    "rust-daemon.err.log",
];

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

// ─── GET /health ─────────────────────────────────────────────────────────────

pub async fn build_health_payload(state: &RuntimeState) -> Value {
    // Read DB stats in a short lock, then drop it before the network call.
    let (memories, decisions, embeddings_count, events, db_freelist_pages) = {
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
        (m, d, e, ev, freelist)
    }; // DB lock released here.

    let db_size_bytes = std::fs::metadata(&state.db_path)
        .map(|meta| meta.len())
        .unwrap_or(0);

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

    let (storage_bytes, backup_count, log_bytes) = collect_storage_metrics(&state.home);
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
        "status": if degraded || db_corrupted { "degraded" } else { "ok" },
        "ready": ready,
        "degraded": degraded || db_corrupted,
        "db_corrupted": db_corrupted,
        "embedding_status": embedding_status,
        "team_mode": state.team_mode,
        "db_freelist_pages": db_freelist_pages,
        "db_size_bytes": db_size_bytes,
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
    if let Err(resp) = ensure_auth(&headers, &state) {
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

fn parse_event_timestamp_utc(raw: &str) -> Option<chrono::DateTime<Utc>> {
    if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(ts.with_timezone(&Utc));
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            naive, Utc,
        ));
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(chrono::DateTime::<Utc>::from_naive_utc_and_offset(
            naive, Utc,
        ));
    }

    None
}

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

fn build_recall_stats_payload_from_rows(rows: &[(String, String)]) -> Value {
    let mut tier_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_sum: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_samples: BTreeMap<String, i64> = BTreeMap::new();
    let mut mode_counts: BTreeMap<String, i64> = BTreeMap::new();

    let mut total_budget = 0_i64;
    let mut total_spent = 0_i64;
    let mut total_saved = 0_i64;
    let mut total_hits = 0_i64;

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
        "recent": recent
    })
}

pub async fn handle_savings(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
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

    let mut by_operation: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for op in ["recall", "store", "boot", "tool"] {
        by_operation.insert(op.to_string(), (0, 0, 0, 0));
    }
    let mut daily_savings_all: BTreeMap<String, i64> = BTreeMap::new();
    let mut recall_daily: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut activity_heatmap_map: BTreeMap<(String, i64), i64> = BTreeMap::new();

    let mut event_stmt = match conn
        .prepare("SELECT type, data, source_agent, created_at FROM events ORDER BY created_at ASC")
    {
        Ok(s) => s,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": e.to_string() }),
            );
        }
    };

    let event_rows: Vec<(String, Option<String>, Option<String>, String)> = event_stmt
        .query_map([], |row| {
            let event_type: String = row.get(0)?;
            let data: Option<String> = row.get(1)?;
            let source_agent: Option<String> = row.get(2)?;
            let created_at: Option<String> = row.get(3)?;
            Ok((
                event_type,
                data,
                source_agent,
                created_at.unwrap_or_default(),
            ))
        })
        .map(|iter| iter.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();

    for (event_type, data_str, source_agent, created_at) in event_rows {
        let parsed = data_str
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .unwrap_or_else(|| json!({}));

        let (operation, saved, served, baseline) = match event_type.as_str() {
            "boot_savings" => (
                Some("boot"),
                parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0),
            ),
            "recall_query" => (
                Some("recall"),
                parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed
                    .get("spent")
                    .or_else(|| parsed.get("served"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
                parsed
                    .get("budget")
                    .or_else(|| parsed.get("baseline"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0),
            ),
            "store_savings" => (
                Some("store"),
                parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0),
            ),
            "decision_stored"
            | "decision_supersede"
            | "decision_conflict"
            | "decision_rejected_duplicate" => (Some("store"), 0, 0, 0),
            "tool_call_savings" => (
                Some("tool"),
                parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0),
                parsed.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0),
            ),
            _ => (None, 0, 0, 0),
        };

        if let Some(op) = operation {
            let entry = by_operation.entry(op.to_string()).or_insert((0, 0, 0, 0));
            entry.0 += saved;
            entry.1 += served;
            entry.2 += baseline;
            entry.3 += 1;
        }

        if saved > 0 {
            let day = &created_at[..created_at.len().min(10)];
            if !day.is_empty() {
                *daily_savings_all.entry(day.to_string()).or_insert(0) += saved;
            }
        }

        if event_type == "recall_query" {
            let day = &created_at[..created_at.len().min(10)];
            if !day.is_empty() {
                let row = recall_daily.entry(day.to_string()).or_insert((0, 0));
                if parsed.get("hits").and_then(|v| v.as_i64()).unwrap_or(0) > 0 {
                    row.0 += 1;
                } else {
                    row.1 += 1;
                }
            }
        }

        if !created_at.is_empty() {
            if let Some(ts) = parse_event_timestamp_utc(&created_at) {
                let day = ts.format("%a").to_string();
                let hour = ts.hour() as i64;
                *activity_heatmap_map.entry((day, hour)).or_insert(0) += 1;
            }
        } else if let Some(agent) = source_agent {
            if !agent.is_empty() {
                *activity_heatmap_map
                    .entry(("Unknown".to_string(), 0))
                    .or_insert(0) += 1;
            }
        }
    }

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

    json_response(
        StatusCode::OK,
        json!({
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
        }),
    )
}

pub async fn handle_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
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
    if let Err(resp) = ensure_auth(&headers, &state) {
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
                    "latency_ms": 5
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
                    "latency_ms": 28
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
                    "latency_ms": 1
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
            payload["estimated_savings"]["vs_always_full_pipeline_pct"],
            58.8
        );
    }
}
