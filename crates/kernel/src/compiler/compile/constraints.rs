use super::super::*;
use crate::db::{TEMPORAL_BOUNDS_SQL, UNORPHANED_VERSION_SQL};
use rusqlite::Connection;

fn durable_decisions_where(scope: &str) -> String {
    format!(
        "status = 'active' AND COALESCE(retention_class,'operational') = 'durable' AND {TEMPORAL_BOUNDS_SQL} AND {UNORPHANED_VERSION_SQL}{scope}"
    )
}

/// Durable, active, currently valid decisions (constraint-like kinds first,
/// then oldest first so long-standing rules are never displaced by churn),
/// bounded to `BOOT_CONSTRAINTS_MAX` lines with an explicit omission count.
const BOOT_CONSTRAINTS_MAX: usize = 40;

/// SQL failure is not an empty constraint set. The agent must not treat a
/// broken read as "there are no durable decisions".
fn unavailable_section(heading: &str, noun: &str) -> String {
    format!(
        "{heading}\n- [unavailable] {noun} could not be loaded; do not assume this set is empty"
    )
}

fn constraints_unavailable() -> (String, usize) {
    (
        unavailable_section("## Constraints", "durable decisions"),
        0,
    )
}

/// Same law as constraints: a failed ranked-fact read is not "no facts".
pub(super) fn ranked_facts_unavailable() -> ContextItem {
    ContextItem::new(
        "## TRUTH",
        unavailable_section("## TRUTH", "ranked facts"),
        0.95,
    )
}

pub(super) fn source_ids(candidates: &[RankedCandidate], kind: &str) -> Vec<i64> {
    candidates
        .iter()
        .filter(|candidate| candidate.source_kind == kind)
        .map(|candidate| candidate.source_id)
        .collect()
}

pub(super) fn build_constraints_capsule(conn: &Connection) -> (String, usize) {
    // Cheap cache key: the durable set's cardinality, newest id and newest
    // update; the capsule is rebuilt only when that changes. Project paths
    // are part of the key so a scoped boot cannot reuse an unscoped cache.
    let scope = owner_clause(conn, "decisions", boot_owner());
    let Ok(key) = conn.query_row(&format!("SELECT COUNT(*) || ':' || COALESCE(MAX(id),0) || ':' || COALESCE(MAX(julianday(updated_at)),'') FROM decisions WHERE {}", durable_decisions_where(&scope)), [], |r| r.get::<_, String>(0)) else {
        // A failed COUNT is not "no durable decisions"; skip the cache and
        // load the real capsule, or say the set is unavailable.
        return build_constraints_capsule_uncached(conn).unwrap_or_else(|_| constraints_unavailable());
    };
    let key = format!(
        "{key}|{}|{}",
        with_boot_paths(|paths| paths.join("\u{1f}")),
        boot_owner().map(|id| id.to_string()).unwrap_or_default()
    );
    if let Some((cached, omitted)) = cache_get(conn, "constraints_capsule", &key) {
        return (cached, omitted);
    }
    match build_constraints_capsule_uncached(conn) {
        Ok(pair) => {
            cache_set(conn, "constraints_capsule", &key, &pair.0, pair.1);
            pair
        }
        Err(_) => constraints_unavailable(),
    }
}

fn build_constraints_capsule_uncached(conn: &Connection) -> Result<(String, usize), String> {
    let scope = owner_clause(conn, "decisions", boot_owner());
    let mut stmt = conn.prepare_cached(&format!("SELECT id, decision, COALESCE(type,'decision') FROM decisions WHERE {} ORDER BY CASE WHEN type IN ('constraint','policy','rule','convention','contract','preference') THEN 0 ELSE 1 END, julianday(created_at) ASC, id ASC", durable_decisions_where(&scope))).map_err(|err| err.to_string())?;
    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|err| err.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|err| err.to_string())?;
    let ids: Vec<i64> = rows.iter().map(|row| row.0).collect();
    let allow = boot_scope_allowlist(conn, "decision", &ids)?;
    let rows: Vec<(i64, String, String)> = rows
        .into_iter()
        .filter(|(id, ..)| keep_boot_id(&allow, *id))
        .collect();
    if rows.is_empty() {
        return Ok((String::new(), 0));
    }
    let total = rows.len();
    let omitted = total.saturating_sub(BOOT_CONSTRAINTS_MAX);
    let mut lines: Vec<String> = rows
        .into_iter()
        .take(BOOT_CONSTRAINTS_MAX)
        .map(|(id, text, kind)| format!("- [{kind} d{id}] {text}"))
        .collect();
    if omitted > 0 {
        lines.push(format!(
            "- ({omitted} more durable decisions not shown; query profile=map for the full set)"
        ));
    }
    Ok((format!("## Constraints\n{}", lines.join("\n")), omitted))
}
