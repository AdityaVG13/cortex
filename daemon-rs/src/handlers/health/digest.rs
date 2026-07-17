// SPDX-License-Identifier: MIT
use crate::handlers::{ensure_auth_rated, json_response, truncate_chars};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::params;
use serde_json::{json, Value};
pub async fn handle_digest(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    match build_digest(&conn) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Digest failed: {err}") })),
    }
}
pub fn build_digest(conn: &rusqlite::Connection) -> Result<Value, String> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let today_like = format!("{today}%");
    let benchmark_source_pattern = format!("{}%", crate::compaction::BENCHMARK_SOURCE_AGENT_PREFIX);
    let total_memories: i64 = conn.query_row("SELECT COUNT(*) FROM memories WHERE status = 'active'", [], |r| r.get(0)).unwrap_or(0);
    let total_decisions: i64 = conn.query_row("SELECT COUNT(*) FROM decisions WHERE status = 'active'", [], |r| r.get(0)).unwrap_or(0);
    let total_conflicts: i64 = conn.query_row("SELECT COUNT(*) FROM decisions WHERE status = 'disputed'", [], |r| r.get(0)).unwrap_or(0);
    let new_memories: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories WHERE created_at LIKE ?1", params![today_like.clone()], |r| r.get(0))
        .unwrap_or(0);
    let new_decisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions WHERE created_at LIKE ?1", params![today_like.clone()], |r| r.get(0))
        .unwrap_or(0);
    let stores_today: i64 = conn
        .query_row("SELECT COUNT(*) FROM events WHERE type = 'decision_stored' AND created_at LIKE ?1", params![today_like.clone()], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    let conflicts_today: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE type = 'decision_conflict' AND created_at LIKE ?1",
            params![today_like.clone()],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let decayed_memories: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories WHERE status = 'active' AND score < 0.5 AND pinned = 0", [], |r| r.get(0))
        .unwrap_or(0);
    let decayed_decisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM decisions WHERE status = 'active' AND score < 0.5 AND pinned = 0", [], |r| r.get(0))
        .unwrap_or(0);
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
    let (raw_total_saved, raw_total_served, raw_boot_count, today_saved, today_served, today_boots): (i64, i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COUNT(*), \
                 COALESCE(SUM(CASE WHEN created_at LIKE ?1 THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) ELSE 0 END), 0), \
                 COALESCE(SUM(CASE WHEN created_at LIKE ?1 THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) ELSE 0 END), 0), \
                 COALESCE(SUM(CASE WHEN created_at LIKE ?1 THEN 1 ELSE 0 END), 0) \
             FROM events \
             WHERE type = 'boot_savings' \
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
            params![today_like.clone(), benchmark_source_pattern.clone()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|e| e.to_string())?;
    let (rollup_saved, rollup_served, rollup_boots): (i64, i64, i64) = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.boots') AS INTEGER), 0)), 0) \
             FROM events \
             WHERE type = 'boot_savings_rollup'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let total_saved = raw_total_saved + rollup_saved;
    let total_served = raw_total_served + rollup_served;
    let boot_count = raw_boot_count + rollup_boots;
    let agent_str = if agent_boots.is_empty() {
        "none".to_string()
    } else {
        agent_boots
            .iter()
            .map(|row| {
                format!(
                    "{} ({})",
                    row.get("source_agent").and_then(|v| v.as_str()).unwrap_or("unknown"),
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
