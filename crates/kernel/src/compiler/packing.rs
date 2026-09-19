use super::*;
use crate::db::{
    ACTIVE_TEMPORAL_SQL, UNORPHANED_VERSION_SQL, UPDATED_CREATED_STAMP_SQL, VALIDITY_START_SQL,
};
use crate::handlers::estimate_tokens;
use rusqlite::Connection;
use serde_json::{Value, json};
use std::env;
pub fn read_usize_env(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}
pub fn boot_source_token_bounds() -> SourceTokenBounds {
    SourceTokenBounds::new(
        read_usize_env(
            "CORTEX_BOOT_MIN_SOURCE_TOKENS",
            DEFAULT_BOOT_MIN_SOURCE_TOKENS,
        ),
        read_usize_env(
            "CORTEX_BOOT_MAX_SOURCE_TOKENS",
            DEFAULT_BOOT_MAX_SOURCE_TOKENS,
        ),
    )
}
pub fn boot_rank_top_n() -> usize {
    read_usize_env("CORTEX_BOOT_RANK_TOP_N", DEFAULT_BOOT_RANK_TOP_N).min(20)
}
pub fn empty_rank_components() -> RankComponents {
    RankComponents {
        class_score: 0.0,
        recency_score: 0.0,
        relevance_score: 0.0,
        activity_score: 0.0,
        total_score: 0.0,
    }
}
const DECISION_BODY_SQL: &str = "CASE WHEN context IS NOT NULL AND TRIM(context) != '' THEN decision || ' (' || context || ')' ELSE decision END";

fn fetch_rank_table(
    conn: &Connection,
    table: &str,
    source_kind: &'static str,
    body_sql: &str,
    extra_where: &str,
    scope: &str,
) -> Result<Vec<RankedCandidate>, String> {
    let mut stmt = conn.prepare_cached(&format!("SELECT id, {body_sql}, retention_class, score, retrievals, last_accessed, updated_at, created_at, status, confirmed_by, {VALIDITY_START_SQL}, valid_until FROM {table} WHERE {ACTIVE_TEMPORAL_SQL}{extra_where} AND {UNORPHANED_VERSION_SQL}{scope} ORDER BY julianday({UPDATED_CREATED_STAMP_SQL}) DESC, id DESC LIMIT 80")).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RankedCandidate {
                source_kind,
                source_id: row.get::<_, i64>(0)?,
                body: row.get::<_, String>(1)?,
                retention_class: row
                    .get::<_, Option<String>>(2)?
                    .unwrap_or_else(|| "operational".to_string()),
                relevance: row.get::<_, Option<f64>>(3)?.unwrap_or(0.5),
                retrievals: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                last_accessed: row.get::<_, Option<String>>(5)?,
                updated_at: row.get::<_, Option<String>>(6)?,
                created_at: row.get::<_, Option<String>>(7)?,
                status: row
                    .get::<_, Option<String>>(8)?
                    .unwrap_or_else(|| "active".to_string()),
                confirmed_by: row.get(9)?,
                valid_from: row.get(10)?,
                valid_until: row.get(11)?,
                components: empty_rank_components(),
            })
        })
        .map_err(|err| err.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

/// Blank `updated_at` is not NULL. `ORDER BY julianday(updated_at)` ranks
/// those rows last, so a just-created fact with empty `updated_at` can miss
/// the LIMIT 80 window. Fall through `created_at` the same way aging does.
pub fn fetch_rank_candidates(conn: &Connection) -> Result<Vec<RankedCandidate>, String> {
    let mut candidates = fetch_rank_table(
        conn,
        "memories",
        "memory",
        "text",
        " AND type != 'state'",
        &super::owner_clause(conn, "memories", super::boot_owner()),
    )?;
    candidates.extend(fetch_rank_table(
        conn,
        "decisions",
        "decision",
        DECISION_BODY_SQL,
        "",
        &super::owner_clause(conn, "decisions", super::boot_owner()),
    )?);
    let mut scope_err = None;
    super::with_boot_paths(|paths| {
        if paths.is_empty() {
            return;
        }
        let decision_ids: Vec<i64> = candidates
            .iter()
            .filter(|c| c.source_kind == "decision")
            .map(|c| c.source_id)
            .collect();
        let memory_ids: Vec<i64> = candidates
            .iter()
            .filter(|c| c.source_kind == "memory")
            .map(|c| c.source_id)
            .collect();
        let dec_allow = match super::capsules::boot_scope_allowlist(conn, "decision", &decision_ids)
        {
            Ok(allow) => allow,
            Err(err) => {
                scope_err = Some(err);
                return;
            }
        };
        let mem_allow = match super::capsules::boot_scope_allowlist(conn, "memory", &memory_ids) {
            Ok(allow) => allow,
            Err(err) => {
                scope_err = Some(err);
                return;
            }
        };
        candidates.retain(|c| match c.source_kind {
            "decision" => super::capsules::keep_boot_id(&dec_allow, c.source_id),
            "memory" => super::capsules::keep_boot_id(&mem_allow, c.source_id),
            _ => true,
        });
    });
    if let Some(err) = scope_err {
        return Err(err);
    }
    Ok(candidates)
}
mod pack;
pub use pack::*;
