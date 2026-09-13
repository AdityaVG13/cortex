//! Plain boot audit APIs; boot execution is provided by kernel operations.
use serde_json::json;

const BOOT_AUDIT_RETENTION_DAYS_DEFAULT: i64 = 90;
fn boot_audit_retention_days() -> i64 {
    std::env::var("CORTEX_BOOT_AUDIT_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&v| v >= 0)
        .unwrap_or(BOOT_AUDIT_RETENTION_DAYS_DEFAULT)
}

pub fn record_boot_audit_best_effort(
    conn: &rusqlite::Connection, agent: &str, profile: &str, max_tokens: usize, result: &crate::compiler::BootResult, latency_ms: i64,
) {
    let token_savings = result.savings.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
    let capsules_count = result.capsules.len() as i64;
    let capsules_json = serde_json::to_string(&result.capsules).unwrap_or_else(|_| "[]".to_string());
    let retention_days = boot_audit_retention_days();
    if retention_days > 0 {
        if let Err(e) = conn.execute(
            "DELETE FROM boot_audits WHERE julianday(created_at) < julianday('now', ?1)",
            rusqlite::params![format!("-{retention_days} days")],
        ) {
            eprintln!("[boot_audits] prune failed: {e}");
        }
    }
    if let Err(e) = conn.execute(
        "INSERT INTO boot_audits (agent, profile, budget_tokens, token_estimate,
                                 token_savings, capsules_count, capsules_json, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            agent,
            profile,
            max_tokens as i64,
            result.token_estimate as i64,
            token_savings,
            capsules_count,
            capsules_json,
            latency_ms
        ],
    ) {
        eprintln!("[boot_audits] insert failed: {e}");
    }
}

pub fn query_boot_audits(conn: &rusqlite::Connection, agent: Option<&str>, limit: Option<usize>) -> rusqlite::Result<serde_json::Value> {
    let limit = limit.unwrap_or(50).min(500);
    let rows: Vec<serde_json::Value> = match agent {
        Some(agent) => conn
            .prepare(
                "SELECT id, agent, profile, budget_tokens, token_estimate,
                    token_savings, capsules_count, latency_ms, created_at
             FROM boot_audits WHERE agent = ?1 ORDER BY id DESC LIMIT ?2",
            )
            .and_then(|mut stmt| stmt.query_map(rusqlite::params![agent, limit as i64], row_to_json)?.collect())?,
        None => conn
            .prepare(
                "SELECT id, agent, profile, budget_tokens, token_estimate,
                    token_savings, capsules_count, latency_ms, created_at
             FROM boot_audits ORDER BY id DESC LIMIT ?1",
            )
            .and_then(|mut stmt| stmt.query_map(rusqlite::params![limit as i64], row_to_json)?.collect())?,
    };
    Ok(json!({"audits": rows, "count": rows.len(), "retention_days": boot_audit_retention_days()}))
}

fn row_to_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({"id":row.get::<_,i64>(0)?,"agent":row.get::<_,String>(1)?,"profile":row.get::<_,String>(2)?,
        "budget_tokens":row.get::<_,i64>(3)?,"token_estimate":row.get::<_,i64>(4)?,"token_savings":row.get::<_,i64>(5)?,
        "capsules_count":row.get::<_,i64>(6)?,"latency_ms":row.get::<_,Option<i64>>(7)?,"created_at":row.get::<_,String>(8)?}))
}
