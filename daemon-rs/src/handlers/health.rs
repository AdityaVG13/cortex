// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::{ensure_auth, json_response, truncate_chars};
use crate::state::RuntimeState;

// ─── GET /health ─────────────────────────────────────────────────────────────

pub async fn handle_health(State(state): State<RuntimeState>) -> Response {
    // Read DB stats in a short lock, then drop it before the network call.
    let (memories, decisions, embeddings_count, events) = {
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
        (m, d, e, ev)
    }; // DB lock released here.

    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();

    let degraded = state
        .degraded_mode
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
            "status": "ok",
            "degraded": degraded,
            "embedding_status": embedding_status,
            "team_mode": state.team_mode,
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
            json!({
                "timestamp": created,
                "agent": d.get("agent").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "served": d.get("served").and_then(|v| v.as_i64()).unwrap_or(0),
                "baseline": d.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0),
                "saved": d.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
                "percent": d.get("percent").and_then(|v| v.as_i64()).unwrap_or(0),
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
    let avg_percent = if !points.is_empty() {
        points
            .iter()
            .map(|p| p["percent"].as_i64().unwrap_or(0))
            .sum::<i64>()
            / points.len() as i64
    } else {
        0
    };

    // Daily aggregation
    let mut daily: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();
    for p in &points {
        let ts = p["timestamp"].as_str().unwrap_or("");
        let day = &ts[..ts.len().min(10)];
        if day.is_empty() {
            continue;
        }
        let e = daily.entry(day.to_string()).or_insert((0, 0, 0));
        e.0 += p["saved"].as_i64().unwrap_or(0);
        e.1 += p["served"].as_i64().unwrap_or(0);
        e.2 += 1;
    }
    let daily_arr: Vec<Value> = daily
        .into_iter()
        .map(|(date, (saved, served, boots))| {
            json!({"date": date, "saved": saved, "served": served, "boots": boots})
        })
        .collect();

    let recent: Vec<&Value> = points.iter().rev().take(20).collect();

    json_response(
        StatusCode::OK,
        json!({
            "summary": {
                "totalSaved": total_saved,
                "totalServed": total_served,
                "totalBaseline": total_baseline,
                "avgPercent": avg_percent,
                "totalBoots": points.len()
            },
            "daily": daily_arr,
            "recent": recent,
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

