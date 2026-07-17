use super::*;
use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use rusqlite::{params, OpenFlags};
use serde_json::{json, Value};
use std::collections::BTreeMap;
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
    let stale_snapshot = match savings_payload_cache().lock() {
        Ok(guard) => guard.clone(),
        Err(_) => None,
    };
    let stale_or_error = |message: String| -> Response {
        if let Some(snapshot) = stale_snapshot.clone() {
            return json_response(StatusCode::OK, snapshot.payload);
        }
        json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":message}))
    };
    let conn =
        match rusqlite::Connection::open_with_flags(&state.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) {
            Ok(conn) => conn,
            Err(err) => {
                return stale_or_error(format!("open savings reader failed: {err}"));
            }
        };
    let busy_timeout_ms = crate::db::SQLITE_BUSY_TIMEOUT_MS;
    if let Err(err) = conn.execute_batch(&format!(
        r#"
        PRAGMA query_only = ON;
        PRAGMA busy_timeout = {busy_timeout_ms};
        PRAGMA foreign_keys = ON;
        PRAGMA mmap_size = 268435456;
        PRAGMA cache_size = -8000;
        PRAGMA temp_store = MEMORY;
        "#,
    )) {
        return stale_or_error(format!("configure savings reader failed: {err}"));
    }
    let savings_window_modifier = format!("-{SAVINGS_HISTORY_DAYS} days");
    let benchmark_source_pattern = format!("{}%", crate::compaction::BENCHMARK_SOURCE_AGENT_PREFIX);
    let (total_saved, total_served, total_baseline, total_boots): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), \
                 COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), \
                 COUNT(*) \
             FROM events \
             WHERE type = 'boot_savings' \
               AND created_at >= datetime('now', ?1) \
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)",
            params![savings_window_modifier.clone(), benchmark_source_pattern.clone()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap_or((0, 0, 0, 0));
    let mut boot_daily_stmt = match conn.prepare(
        "SELECT \
             SUBSTR(created_at, 1, 10) AS day, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0) AS saved, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0) AS served, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0) AS baseline, \
             COUNT(*) AS boots \
         FROM events \
         WHERE type = 'boot_savings' \
           AND created_at >= datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) \
           AND created_at IS NOT NULL \
         GROUP BY day \
         ORDER BY day ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare boot daily query failed: {e}")),
    };
    let boot_daily_rows =
        match boot_daily_stmt.query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
            let day: Option<String> = row.get(0)?;
            Ok((day.unwrap_or_default(), row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
        }) {
            Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
            Err(e) => return stale_or_error(format!("query boot daily failed: {e}")),
        };
    drop(boot_daily_stmt);
    let daily_arr: Vec<Value> = boot_daily_rows
        .iter()
        .filter_map(|(day, saved, served, baseline, boots)| {
            if day.is_empty() {
                None
            } else {
                Some(json!({"date":day,"saved":saved,"served":served,"baseline":baseline,"boots":boots}))
            }
        })
        .collect();
    let mut boot_by_agent_stmt = match conn.prepare(
        "SELECT \
             COALESCE(NULLIF(TRIM(COALESCE(json_extract(data, '$.agent'), 'unknown')), ''), 'unknown') AS agent, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0) AS saved, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0) AS served, \
             COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0) AS baseline, \
             COUNT(*) AS boots \
         FROM events \
         WHERE type = 'boot_savings' \
           AND created_at >= datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) \
         GROUP BY agent",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare boot by-agent query failed: {e}")),
    };
    let boot_by_agent_rows = match boot_by_agent_stmt
        .query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
        }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query boot by-agent failed: {e}")),
    };
    drop(boot_by_agent_stmt);
    let mut by_agent: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for (agent, saved, served, baseline, boots) in boot_by_agent_rows {
        by_agent.insert(agent, (saved, served, baseline, boots));
    }
    let by_agent_arr: Vec<Value> = by_agent
        .into_iter()
        .map(|(agent, (saved, served, baseline, boots))| {
            let percent = if baseline > 0 { (saved * 100) / baseline } else { 0 };
            json!({"agent":agent,"saved":saved,"served":served,
"baseline":baseline,"boots":boots,"percent":percent})
        })
        .collect();
    let mut recent_boot_stmt = match conn.prepare(
        "SELECT data, created_at \
         FROM events \
         WHERE type = 'boot_savings' \
           AND created_at >= datetime('now', ?1) \
           AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
           AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) \
         ORDER BY created_at DESC \
         LIMIT 20",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare recent boot query failed: {e}")),
    };
    let recent_rows = match recent_boot_stmt.query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
        let data_str: String = row.get(0)?;
        let created: String = row.get(1)?;
        Ok((data_str, created))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query recent boot rows failed: {e}")),
    };
    drop(recent_boot_stmt);
    let recent: Vec<Value> = recent_rows
        .into_iter()
        .map(|(data_str, created)| {
            let d: Value = serde_json::from_str(&data_str).unwrap_or(json!({}));
            let served = d.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
            let baseline = d.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0);
            let saved = d.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
            let percent = d.get("percent").and_then(|v| v.as_i64()).unwrap_or(0);
            let admitted = d.get("admitted").and_then(|v| v.as_i64()).unwrap_or(0);
            let rejected = d.get("rejected").and_then(|v| v.as_i64()).unwrap_or(0);
            let compression_ratio = if served > 0 { ((baseline as f64 / served as f64) * 100.0).round() / 100.0 } else { 0.0 };
            json!({"timestamp":created,"agent":d
.get("agent").and_then(|v|v.as_str()).unwrap_or("unknown"),"served":served,"baseline":baseline,"saved":saved,"percent":percent,
"admitted":admitted,"rejected":rejected,"compressionRatio":compression_ratio})
        })
        .collect();
    let mut by_operation: BTreeMap<String, (i64, i64, i64, i64)> = BTreeMap::new();
    for op in ["recall", "store", "boot", "tool"] {
        by_operation.insert(op.to_string(), (0, 0, 0, 0));
    }
    let mut rollup_op_stmt = match conn.prepare(
        "SELECT operation, \
             COALESCE(SUM(saved), 0) AS saved, \
             COALESCE(SUM(served), 0) AS served, \
             COALESCE(SUM(baseline), 0) AS baseline, \
             COALESCE(SUM(events), 0) AS events \
         FROM event_savings_rollups \
         WHERE day >= date('now', ?1) \
         GROUP BY operation",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare rollup operation query failed: {e}")),
    };
    let rollup_op_rows = match rollup_op_stmt.query_map(params![savings_window_modifier.clone()], |row| {
        let operation: String = row.get(0)?;
        let saved: i64 = row.get(1)?;
        let served: i64 = row.get(2)?;
        let baseline: i64 = row.get(3)?;
        let events: i64 = row.get(4)?;
        Ok((operation, saved, served, baseline, events))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => {
            return stale_or_error(format!("query rollup operation aggregates failed: {e}"));
        }
    };
    drop(rollup_op_stmt);
    for (operation, saved, served, baseline, events) in rollup_op_rows {
        let entry = by_operation.entry(operation).or_insert((0, 0, 0, 0));
        entry.0 += saved;
        entry.1 += served;
        entry.2 += baseline;
        entry.3 += events;
    }
    let mut op_stmt=match conn.prepare(
"SELECT \
             CASE \
                 WHEN type = 'recall_query' THEN 'recall' \
                 WHEN type = 'store_savings' THEN 'store' \
                 WHEN type = 'tool_call_savings' THEN 'tool' \
              END AS operation, \
             COALESCE(SUM(CASE \
                 WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 ELSE 0 END), 0) AS saved, \
             COALESCE(SUM(CASE \
                 WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.spent') AS INTEGER), COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)) \
                 WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) \
                 WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) \
                 ELSE 0 END), 0) AS served, \
             COALESCE(SUM(CASE \
                 WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.budget') AS INTEGER), COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)) \
                  WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) \
                  WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) \
                  ELSE 0 END), 0) AS baseline, \
             COUNT(*) AS events \
           FROM events \
           WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') \
             AND created_at >= datetime('now', ?1) \
             AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) \
             AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) \
             AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) \
           GROUP BY operation"
,){Ok(stmt)=>stmt,Err(e)=>return stale_or_error(format!("prepare operation aggregate query failed: {e}")),};
    let op_rows = match op_stmt.query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
        let operation: String = row.get(0)?;
        let saved: i64 = row.get(1)?;
        let served: i64 = row.get(2)?;
        let baseline: i64 = row.get(3)?;
        let events: i64 = row.get(4)?;
        Ok((operation, saved, served, baseline, events))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query operation aggregates failed: {e}")),
    };
    drop(op_stmt);
    for (operation, saved, served, baseline, events) in op_rows {
        let entry = by_operation.entry(operation).or_insert((0, 0, 0, 0));
        entry.0 += saved;
        entry.1 += served;
        entry.2 += baseline;
        entry.3 += events;
    }
    let mut daily_savings_all: BTreeMap<String, i64> = BTreeMap::new();
    let mut recall_daily: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut rollup_daily_stmt = match conn.prepare(
        "SELECT day, \
             COALESCE(SUM(saved), 0) AS saved_delta, \
             COALESCE(SUM(hits), 0) AS hits, \
             COALESCE(SUM(misses), 0) AS misses \
         FROM event_savings_rollups \
         WHERE day >= date('now', ?1) \
         GROUP BY day \
         ORDER BY day ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare rollup daily savings query failed: {e}")),
    };
    let rollup_daily_rows = match rollup_daily_stmt.query_map(params![savings_window_modifier.clone()], |row| {
        let day: String = row.get(0)?;
        let saved_delta: i64 = row.get(1)?;
        let hits: i64 = row.get(2)?;
        let misses: i64 = row.get(3)?;
        Ok((day, saved_delta, hits, misses))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query rollup daily savings failed: {e}")),
    };
    drop(rollup_daily_stmt);
    for (day, saved_delta, hits, misses) in rollup_daily_rows {
        if day.is_empty() {
            continue;
        }
        *daily_savings_all.entry(day.clone()).or_insert(0) += saved_delta;
        if hits + misses > 0 {
            let entry = recall_daily.entry(day).or_insert((0, 0));
            entry.0 += hits;
            entry.1 += misses;
        }
    }
    let mut daily_stmt = match conn.prepare(
        "SELECT \
             SUBSTR(created_at, 1, 10) AS day, \
             COALESCE(SUM(CASE \
                 WHEN type = 'boot_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 ELSE 0 END), 0) AS saved_delta, \
             SUM(CASE \
                 WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 1 \
                 ELSE 0 END) AS hits, \
             SUM(CASE \
                 WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 0 \
                 WHEN type = 'recall_query' THEN 1 \
                 ELSE 0 END) AS misses \
            FROM events \
            WHERE type IN ('boot_savings', 'recall_query', 'store_savings', 'tool_call_savings')
              AND created_at >= datetime('now', ?1)
              AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2)
              AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2)
              AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)
              AND created_at IS NOT NULL \
            GROUP BY day \
            ORDER BY day ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare daily savings query failed: {e}")),
    };
    let daily_rows = match daily_stmt.query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
        let day: Option<String> = row.get(0)?;
        let saved_delta: i64 = row.get(1)?;
        let hits: i64 = row.get(2)?;
        let misses: i64 = row.get(3)?;
        Ok((day.unwrap_or_default(), saved_delta, hits, misses))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query daily savings failed: {e}")),
    };
    drop(daily_stmt);
    for (day, saved_delta, hits, misses) in daily_rows {
        if !day.is_empty() {
            *daily_savings_all.entry(day.clone()).or_insert(0) += saved_delta;
            if hits + misses > 0 {
                let entry = recall_daily.entry(day).or_insert((0, 0));
                entry.0 += hits;
                entry.1 += misses;
            }
        }
    }
    let mut activity_heatmap_map: BTreeMap<(String, i64), i64> = BTreeMap::new();
    let mut rollup_heatmap_stmt = match conn.prepare(
        "SELECT \
             CAST(strftime('%w', day) AS INTEGER) AS weekday, \
             hour, \
             COALESCE(SUM(events), 0) AS cnt \
         FROM event_savings_rollups \
         WHERE day >= date('now', ?1) \
         GROUP BY weekday, hour",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare rollup heatmap query failed: {e}")),
    };
    let rollup_heatmap_rows = match rollup_heatmap_stmt.query_map(params![savings_window_modifier.clone()], |row| {
        let weekday: Option<i64> = row.get(0)?;
        let hour: Option<i64> = row.get(1)?;
        let count: i64 = row.get(2)?;
        Ok((weekday, hour, count))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query rollup heatmap failed: {e}")),
    };
    drop(rollup_heatmap_stmt);
    for (weekday, hour, count) in rollup_heatmap_rows {
        if let (Some(day), Some(hour)) = (weekday, hour) {
            let day_name = weekday_name_from_sqlite(day).to_string();
            *activity_heatmap_map.entry((day_name, hour)).or_insert(0) += count;
        }
    }
    let mut heatmap_stmt = match conn.prepare(
        "SELECT \
             CAST(strftime('%w', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER) AS weekday, \
             CAST(strftime('%H', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER) AS hour, \
             COUNT(*) AS cnt \
            FROM events \
            WHERE type IN ('boot_savings', 'recall_query', 'store_savings', 'tool_call_savings')
               AND created_at >= datetime('now', ?1)
               AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2)
               AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2)
               AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)
               AND created_at IS NOT NULL \
             GROUP BY weekday, hour",
    ) {
        Ok(stmt) => stmt,
        Err(e) => return stale_or_error(format!("prepare activity heatmap query failed: {e}")),
    };
    let heatmap_rows = match heatmap_stmt.query_map(params![savings_window_modifier.clone(), benchmark_source_pattern.clone()], |row| {
        let weekday: Option<i64> = row.get(0)?;
        let hour: Option<i64> = row.get(1)?;
        let count: i64 = row.get(2)?;
        Ok((weekday, hour, count))
    }) {
        Ok(iter) => iter.filter_map(|row| row.ok()).collect::<Vec<_>>(),
        Err(e) => return stale_or_error(format!("query activity heatmap failed: {e}")),
    };
    drop(heatmap_stmt);
    for (weekday, hour, count) in heatmap_rows {
        if let (Some(day), Some(hour)) = (weekday, hour) {
            let day_name = weekday_name_from_sqlite(day).to_string();
            *activity_heatmap_map.entry((day_name, hour)).or_insert(0) += count;
        }
    }
    let avg_percent = if total_baseline > 0 { (total_saved * 100) / total_baseline } else { 0 };
    by_operation.insert("boot".to_string(), (total_saved, total_served, total_baseline, total_boots));
    let avg_saved_per_boot = if total_boots > 0 { total_saved / total_boots } else { 0 };
    let avg_served_per_boot = if total_boots > 0 { total_served / total_boots } else { 0 };
    let avg_baseline_per_boot = if total_boots > 0 { total_baseline / total_boots } else { 0 };
    let by_operation_arr: Vec<Value> = ["recall", "store", "boot", "tool"]
        .iter()
        .map(|op| {
            let (saved, served, baseline, events) = by_operation.get(*op).copied().unwrap_or((0, 0, 0, 0));
            let percent = if baseline > 0 { (saved * 100) / baseline } else { 0 };
            json!({"operation":op,"saved":saved,"served":served,"baseline":
baseline,"events":events,"percent":percent})
        })
        .collect();
    let mut running_saved = 0_i64;
    let cumulative: Vec<Value> = daily_savings_all
        .into_iter()
        .map(|(date, saved_delta)| {
            running_saved += saved_delta;
            json!({"date":date,"savedDelta":saved_delta,"savedTotal":
running_saved})
        })
        .collect();
    let recall_trend: Vec<Value> = recall_daily
        .into_iter()
        .map(|(date, (hits, misses))| {
            let queries = hits + misses;
            let hit_rate = if queries > 0 { ((hits as f64 / queries as f64) * 1000.0).round() / 10.0 } else { 0.0 };
            json!({"date":date,"hits":hits,
"misses":misses,"queries":queries,"hitRatePct":hit_rate})
        })
        .collect();
    let activity_heatmap: Vec<Value> = activity_heatmap_map
        .into_iter()
        .map(|((day, hour), count)| json!({"day":day,"hour":hour,"count":count}))
        .collect();
    let payload = json!({"summary":{
"totalSaved":total_saved,"totalServed":total_served,"totalBaseline":total_baseline,"avgPercent":avg_percent,"totalBoots":
total_boots,"avgSavedPerBoot":avg_saved_per_boot,"avgServedPerBoot":avg_served_per_boot,"avgBaselinePerBoot":avg_baseline_per_boot
,"scope":"boot_prompt_plus_event_operations","note":
"Boot savings are precise from /boot events. Recall/store/tool figures are event-derived estimates when instrumentation is available. Analytics scope is the last 30 days."
},"daily":daily_arr,"byAgent":by_agent_arr,"recent":recent,"byOperation":by_operation_arr,"cumulative":cumulative,"recallTrend":
recall_trend,"activityHeatmap":activity_heatmap,});
    if let Ok(mut cache) = savings_payload_cache().lock() {
        *cache = Some(SavingsPayloadSnapshot { computed_at_unix_secs: now_unix_secs, payload: payload.clone() });
    }
    json_response(StatusCode::OK, payload)
}
