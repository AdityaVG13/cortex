use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::OpenFlags;
use serde_json::json;

use super::{savings_payload_cache, savings_payload_cache_if_fresh, SavingsPayloadSnapshot, SAVINGS_HISTORY_DAYS};

pub async fn handle_savings(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let now = chrono::Utc::now().timestamp();
    if let Ok(guard) = savings_payload_cache().lock() {
        if let Some(snapshot) = savings_payload_cache_if_fresh(guard.clone(), now) {
            return json_response(StatusCode::OK, snapshot.payload);
        }
    }
    let payload = match build_savings_payload(&state) {
        Ok(payload) => payload,
        Err(err) => return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":err})),
    };
    if let Ok(mut cache) = savings_payload_cache().lock() {
        *cache = Some(SavingsPayloadSnapshot { computed_at_unix_secs: now, payload: payload.clone() });
    }
    json_response(StatusCode::OK, payload)
}

fn build_savings_payload(state: &RuntimeState) -> Result<serde_json::Value, String> {
    let conn = rusqlite::Connection::open_with_flags(&state.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|err| format!("open savings reader failed: {err}"))?;
    let window = format!("-{SAVINGS_HISTORY_DAYS} days");
    let benchmark = format!("{}%", crate::compaction::BENCHMARK_SOURCE_AGENT_PREFIX);
    let (boot_saved, boot_served, boot_baseline, boots): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(CAST(json_extract(data,'$.saved') AS INTEGER)),0),
                    COALESCE(SUM(CAST(json_extract(data,'$.served') AS INTEGER)),0),
                    COALESCE(SUM(CAST(json_extract(data,'$.baseline') AS INTEGER)),0),
                    COUNT(*)
             FROM events
             WHERE type='boot_savings'
               AND created_at >= datetime('now', ?1)
               AND LOWER(COALESCE(source_agent,'')) NOT LIKE LOWER(?2)",
            rusqlite::params![window, benchmark],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    let (recall_saved, recall_spent, recalls): (i64, i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(CAST(json_extract(data,'$.saved') AS INTEGER)),0),
                    COALESCE(SUM(CAST(json_extract(data,'$.spent') AS INTEGER)),0),
                    COUNT(*)
             FROM events
             WHERE type='recall_query'
               AND created_at >= datetime('now', ?1)
               AND LOWER(COALESCE(source_agent,'')) NOT LIKE LOWER(?2)",
            rusqlite::params![format!("-{SAVINGS_HISTORY_DAYS} days"), benchmark],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or((0, 0, 0));
    let total_saved = boot_saved + recall_saved;
    let total_served = boot_served + recall_spent;
    let total_baseline = boot_baseline + recall_spent + recall_saved;
    let percent = if total_baseline > 0 { (total_saved * 100) / total_baseline } else { 0 };
    Ok(json!({
        "schemaVersion": 1,
        "windowDays": SAVINGS_HISTORY_DAYS,
        "totals": {"saved": total_saved, "served": total_served, "baseline": total_baseline, "percent": percent},
        "boot": {"saved": boot_saved, "served": boot_served, "baseline": boot_baseline, "boots": boots},
        "recall": {"saved": recall_saved, "spent": recall_spent, "queries": recalls}
    }))
}
