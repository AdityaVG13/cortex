// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::{NaiveDateTime, Timelike, Utc};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::{ensure_auth, json_response, truncate_chars};
use crate::state::RuntimeState;

// ─── GET /health ─────────────────────────────────────────────────────────────

pub async fn handle_health(State(state): State<RuntimeState>) -> Response {
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

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();

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

    json_response(
        StatusCode::OK,
        json!({
            "status": if db_corrupted { "degraded" } else { "ok" },
            "degraded": degraded || db_corrupted,
            "db_corrupted": db_corrupted,
            "embedding_status": embedding_status,
            "team_mode": state.team_mode,
            "db_freelist_pages": db_freelist_pages,
            "db_size_bytes": db_size_bytes,
            "stats": {
                "memories": memories,
                "decisions": decisions,
                "embeddings": embeddings_count,
                "events": events,
                "home": home
            }
        }),
    )
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
            )
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
            )
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

    json_response(
        StatusCode::OK,
        json!({
            "memories": memories,
            "decisions": decisions,
        }),
    )
}
