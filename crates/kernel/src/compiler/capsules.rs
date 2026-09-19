use crate::db::{ACTIVE_TEMPORAL_SQL, TEMPORAL_BOUNDS_SQL, UNORPHANED_VERSION_SQL};
use crate::handlers::{agent_match_params, estimate_tokens_from_chars};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;

mod fetch;
pub use fetch::*;
mod delta;
pub use delta::build_delta_capsule;

pub(super) fn bounds_and_unorphaned() -> String {
    format!("{TEMPORAL_BOUNDS_SQL} AND {UNORPHANED_VERSION_SQL}")
}

fn current_updated_window() -> String {
    format!(
        "status = 'active' AND updated_at IS NOT NULL AND TRIM(updated_at) != '' AND {}",
        bounds_and_unorphaned()
    )
}

fn latest_updated_at(conn: &Connection, table: &str) -> Option<String> {
    conn.query_row(&format!("SELECT updated_at FROM {table} WHERE {} ORDER BY julianday(updated_at) DESC, id DESC LIMIT 1", current_updated_window()), [], |r| r.get(0)).optional().ok().flatten()
}

fn later_timestamp(left: &str, right: &str) -> bool {
    match (
        super::parse_timestamp(Some(left)),
        super::parse_timestamp(Some(right)),
    ) {
        (Some(a), Some(b)) => b > a,
        (None, Some(_)) => true,
        _ => false,
    }
}

pub fn stored_max_timestamp(conn: &Connection) -> Option<String> {
    // `updated_at` mixes RFC3339 (`now_iso`) and SQLite `datetime('now')`.
    // `MAX(text)` is lexicographic (`T` > space), so a later space-format
    // aging/mutate stamp loses to an earlier same-day RFC3339 value.
    let mem_max = latest_updated_at(conn, "memories");
    let dec_max = latest_updated_at(conn, "decisions");
    match (mem_max, dec_max) {
        (Some(a), Some(b)) if later_timestamp(&a, &b) => Some(b),
        (Some(a), _) => Some(a),
        (None, b) => b,
    }
}

pub fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
    // `lower(trim(source_agent)) =` treated `claude-code (opus)` as a
    // different booter than `claude-code`, so the delta capsule missed the
    // last boot and replayed "new since boot" on every SessionStart.
    let (ident, like) = agent_match_params(agent)?;
    conn.query_row(&format!("SELECT created_at FROM events WHERE type = 'agent_boot' AND {} ORDER BY id DESC LIMIT 1", crate::handlers::ident_match_sql("source_agent", 1)), params![ident, like], |r| r.get::<_, String>(0)).ok()
}

/// `AND owner_id = N` when the table carries owner scoping and a caller is
/// known; empty otherwise. Team-mode boot must not leak another owner's
/// messages, tasks, locks, feed or decisions into a caller's capsule.
pub use crate::db::owner_and_clause as owner_clause;

thread_local! {
    static BOOT_OWNER: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    static BOOT_PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn set_boot_owner(owner: Option<i64>) {
    BOOT_OWNER.with(|cell| cell.set(owner));
}

pub fn boot_owner() -> Option<i64> {
    BOOT_OWNER.with(|cell| cell.get())
}

pub fn set_boot_paths(paths: &[String]) {
    let normalized = crate::handlers::recall::normalize_query_paths(paths);
    BOOT_PATHS.with(|cell| *cell.borrow_mut() = normalized);
}

pub fn with_boot_paths<R>(f: impl FnOnce(&[String]) -> R) -> R {
    BOOT_PATHS.with(|cell| f(&cell.borrow()))
}

/// `Ok(None)` means the boot is unscoped and every id stays eligible.
/// `Ok(Some(empty))` means the boot is path-scoped and no id survived.
/// `Err` is a failed path lookup: not "no paths" (read law would treat that
/// as visible and leak every foreign project into constraints / recent / identity).
pub(crate) fn boot_scope_allowlist(
    conn: &Connection,
    target_type: &str,
    ids: &[i64],
) -> Result<Option<HashSet<i64>>, String> {
    with_boot_paths(|paths| {
        if paths.is_empty() {
            return Ok(None);
        }
        let path_map = crate::handlers::recall::explicit_paths_by_target(conn, target_type, ids)?;
        Ok(Some(
            ids.iter()
                .copied()
                .filter(|id| {
                    crate::handlers::recall::read_path_sets(
                        paths,
                        path_map.get(id).map(Vec::as_slice).unwrap_or(&[]),
                    )
                })
                .collect(),
        ))
    })
}

pub(crate) fn keep_boot_id(allow: &Option<HashSet<i64>>, id: i64) -> bool {
    allow.as_ref().map(|ids| ids.contains(&id)).unwrap_or(true)
}

fn sum_active_chars(conn: &Connection, table: &str, col: &str, scope: &str) -> usize {
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(LENGTH({col})), 0) FROM {table} WHERE {ACTIVE_TEMPORAL_SQL}{scope}"
        ),
        [],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0) as usize
}

pub fn estimate_raw_baseline(conn: &Connection, _home: &Path) -> usize {
    let mem_scope = owner_clause(conn, "memories", boot_owner());
    let dec_scope = owner_clause(conn, "decisions", boot_owner());
    estimate_tokens_from_chars(
        sum_active_chars(conn, "memories", "text", &mem_scope)
            + sum_active_chars(conn, "decisions", "decision", &dec_scope),
    )
}

pub fn record_boot(conn: &Connection, agent: &str) {
    let now = stored_max_timestamp(conn)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let _ = crate::handlers::log_event(
        conn,
        "agent_boot",
        json!({"timestamp": &now, "agent": agent}),
        agent,
    );
}
