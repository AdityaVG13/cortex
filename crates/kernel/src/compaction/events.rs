use super::*;
use rusqlite::{Connection, params};
pub fn rollup_old_boot_savings(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    rollup_old_boot_savings_with_retention(conn, failures, BOOT_SAVINGS_RETENTION_DAYS)
}
pub fn rollup_old_boot_savings_with_retention(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    // A failed aggregate is not zero work: DELETE still ran and would drop
    // uncounted boot_savings or every existing rollup. Skip the whole pass.
    let retention_window = format!("-{retention_days} days");
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    let (old_saved, old_served, old_baseline, old_boots): (i64, i64, i64, i64) = match conn.query_row("SELECT COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), COUNT(*) FROM events WHERE type = 'boot_savings' AND julianday(NULLIF(TRIM(created_at), '')) < julianday('now', ?1) AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)", params![retention_window.clone(), benchmark_source_pattern.clone()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))) { Ok(row) => row, Err(err) => return fail_zero(failures, "rollup_old_boot_savings SELECT events (boot_savings)", err) };
    let (rollup_saved, rollup_served, rollup_baseline, rollup_boots, rollup_rows): (i64, i64, i64, i64, i64) = match conn.query_row("SELECT COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0)), 0), COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)), 0), COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)), 0), COALESCE(SUM(COALESCE(CAST(json_extract(data, '$.boots') AS INTEGER), 0)), 0), COUNT(*) FROM events WHERE type = 'boot_savings_rollup'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))) { Ok(row) => row, Err(err) => return fail_zero(failures, "rollup_old_boot_savings SELECT events (boot_savings_rollup)", err) };
    if old_boots <= 0 && rollup_rows <= 1 {
        return 0;
    }
    let merged_saved = old_saved + rollup_saved;
    let merged_served = old_served + rollup_served;
    let merged_baseline = old_baseline + rollup_baseline;
    let merged_boots = old_boots + rollup_boots;
    let Ok(sp) = crate::db::SqliteSavepoint::enter(conn, "boot_rollup") else {
        return fail_zero(
            failures,
            "rollup_old_boot_savings SAVEPOINT",
            "failed to enter savepoint",
        );
    };
    let deleted_old = match conn.execute("DELETE FROM events WHERE type = 'boot_savings' AND julianday(NULLIF(TRIM(created_at), '')) < julianday('now', ?1) AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)", params![retention_window, benchmark_source_pattern]) { Ok(n) => n, Err(err) => return fail_zero(failures, "rollup_old_boot_savings DELETE events (boot_savings)", err) };
    let deleted_rollups =
        match conn.execute("DELETE FROM events WHERE type = 'boot_savings_rollup'", []) {
            Ok(n) => n,
            Err(err) => {
                return fail_zero(
                    failures,
                    "rollup_old_boot_savings DELETE events (boot_savings_rollup)",
                    err,
                );
            }
        };
    if merged_boots > 0 {
        let payload = serde_json::json!({"saved":merged_saved,"served":merged_served,"baseline":merged_baseline,"boots":merged_boots,"retention_days":retention_days,"rolled_up_at":chrono::Utc::now().to_rfc3339(),}).to_string();
        if let Err(err) = conn.execute("INSERT INTO events (type, data, source_agent, created_at) VALUES ('boot_savings_rollup', ?1, 'compaction', datetime('now'))", params![payload]) { return fail_zero(failures, "rollup_old_boot_savings INSERT boot_savings_rollup", err); }
        if let Err(err) = sp.release() {
            return fail_zero(failures, "rollup_old_boot_savings RELEASE", err);
        }
        let consolidated_rollups = deleted_rollups.saturating_sub(1);
        deleted_old + consolidated_rollups
    } else if let Err(err) = sp.release() {
        fail_zero(failures, "rollup_old_boot_savings RELEASE", err)
    } else {
        deleted_old + deleted_rollups
    }
}
pub fn rollup_old_savings_events(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    retention_days: i64,
) -> usize {
    let retention_window = format!("-{retention_days} days");
    let benchmark_source_pattern = format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");
    type SavingsRollupRow = (String, i64, String, i64, i64, i64, i64, i64, i64);
    // Candidate SELECT stays fail-safe-swallowed: no candidates -> no deletes.
    let rollup_rows:Vec<SavingsRollupRow>=conn.prepare("SELECT SUBSTR(created_at, 1, 10) AS day, COALESCE(CAST(strftime('%H', REPLACE(SUBSTR(created_at, 1, 19), 'T', ' ')) AS INTEGER), 0) AS hour, CASE WHEN type = 'recall_query' THEN 'recall' WHEN type = 'store_savings' THEN 'store' WHEN type = 'tool_call_savings' THEN 'tool' END AS operation, COALESCE(SUM(CASE WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) ELSE 0 END), 0) AS saved, COALESCE(SUM(CASE WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.spent') AS INTEGER), COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0)) WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0) ELSE 0 END), 0) AS served, COALESCE(SUM(CASE WHEN type = 'recall_query' THEN COALESCE(CAST(json_extract(data, '$.budget') AS INTEGER), COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0)) WHEN type = 'store_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) WHEN type = 'tool_call_savings' THEN COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0) ELSE 0 END), 0) AS baseline, COUNT(*) AS events, SUM(CASE WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 1 ELSE 0 END) AS hits, SUM(CASE WHEN type = 'recall_query' AND COALESCE(CAST(json_extract(data, '$.hits') AS INTEGER), 0) > 0 THEN 0 WHEN type = 'recall_query' THEN 1 ELSE 0 END) AS misses FROM events WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') AND created_at IS NOT NULL AND julianday(NULLIF(TRIM(created_at), '')) < julianday('now', ?1) AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2) GROUP BY day, hour, operation").and_then(|mut stmt|{let rows=stmt.query_map(params![retention_window.clone(),benchmark_source_pattern.clone()],|row|{Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,i64>(7)?,row.get::<_,i64>(8)?,))})?;rows.collect::<Result<Vec<_>,_>>()}).unwrap_or_default();
    if rollup_rows.is_empty() {
        return 0;
    }
    // INSERT + add-on-conflict then DELETE must be atomic: a failed delete
    // would leave the source events, and the next pass would add them twice.
    let Ok(sp) = crate::db::SqliteSavepoint::enter(conn, "savings_rollup") else {
        return fail_zero(
            failures,
            "rollup_old_savings_events SAVEPOINT",
            "failed to enter savepoint",
        );
    };
    for (day, hour, operation, saved, served, baseline, events, hits, misses) in rollup_rows {
        if let Err(err) = conn.execute("INSERT INTO event_savings_rollups (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now')) ON CONFLICT(day, hour, operation) DO UPDATE SET saved = event_savings_rollups.saved + excluded.saved, served = event_savings_rollups.served + excluded.served, baseline = event_savings_rollups.baseline + excluded.baseline, events = event_savings_rollups.events + excluded.events, hits = event_savings_rollups.hits + excluded.hits, misses = event_savings_rollups.misses + excluded.misses, updated_at = datetime('now')", params![day, hour, operation, saved, served, baseline, events, hits, misses]) { return fail_zero(failures, "rollup_old_savings_events INSERT event_savings_rollups", err); }
    }
    let deleted = match conn.execute("DELETE FROM events WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings') AND created_at IS NOT NULL AND julianday(NULLIF(TRIM(created_at), '')) < julianday('now', ?1) AND LOWER(COALESCE(source_agent, '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) NOT LIKE LOWER(?2) AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) NOT LIKE LOWER(?2)", params![retention_window, benchmark_source_pattern]) { Ok(n) => n, Err(err) => return fail_zero(failures, "rollup_old_savings_events DELETE events (savings)", err) };
    if let Err(err) = sp.release() {
        return fail_zero(failures, "rollup_old_savings_events RELEASE", err);
    }
    deleted
}
mod prune;
pub use prune::*;
