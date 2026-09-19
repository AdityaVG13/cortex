use crate::compaction::MaintenanceFailure;
use crate::db::{LAST_ACCESSED_CREATED_STAMP_SQL, UPDATED_CREATED_STAMP_SQL};
use crate::handlers::feedback;
use crate::protocol::nonempty_opt;
use rusqlite::{Connection, params};

/// Blank TEXT is not NULL. `COALESCE(updated_at, created_at)` sticks on `''`,
/// `julianday('')` is NULL, and the age predicate never matches -- so those
/// rows skip every tier, including expired-but-still-`active` ones. Fall
/// through the first non-blank stamp instead of treating blank as unbounded.
const FRESH_DAYS: i64 = 3;
const RECENT_DAYS: i64 = 14;
const OLD_DAYS: i64 = 60;
const GC_SCORE_THRESHOLD: f64 = 0.15;
const GC_MIN_DAYS: i64 = 3;
/// Durable identity is not a compression or archive candidate. Salience GC
/// already refuses it; time-based aging must too or a standing policy leaves
/// current recall after one pass per tier (fresh → recent → old → archived).
const NOT_DURABLE: &str = "COALESCE(retention_class, 'operational') != 'durable'";

fn skip_immune(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    source: Option<&str>,
) -> bool {
    let Some(src) = nonempty_opt(source) else {
        return false;
    };
    feedback::has_retrieval_immunity(conn, src).unwrap_or_else(|err| {
        failures.push(MaintenanceFailure {
            op: op.to_string(),
            error: err,
        });
        true
    })
}

/// Recall stores `memory::{id}` / `decision::{id}`, and also the display
/// source (`memories.source` / `decisions.context`) when that column is set.
/// Checking only the column misses the ident the feedback row actually uses.
fn skip_immune_keys(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    ident: &str,
    alias: Option<&str>,
) -> bool {
    if skip_immune(conn, failures, op, Some(ident)) {
        return true;
    }
    skip_immune(conn, failures, op, alias)
}

fn not_immune_sql(
    table: &str,
    ident_prefix: &str,
    alias_col: &str,
    window_idx: u8,
    thresh_idx: u8,
) -> String {
    format!(
        "(SELECT COUNT(*) FROM recall_feedback WHERE signal > 0 AND julianday('now') - julianday(created_at) <= ?{window_idx} AND result_source = '{ident_prefix}' || {table}.id) < ?{thresh_idx} AND (NULLIF(TRIM({table}.{alias_col}), '') IS NULL OR (SELECT COUNT(*) FROM recall_feedback WHERE signal > 0 AND julianday('now') - julianday(created_at) <= ?{window_idx} AND result_source = {table}.{alias_col}) < ?{thresh_idx})"
    )
}

/// Outcome of one aging pass: only mutations that actually committed count as
/// work. Every destructive op that failed is reported in `failures` (op names
/// the operation and target table); empty `failures` means the pass did
/// exactly what it claims.
#[derive(Debug, Default, Clone)]
pub struct AgingReport {
    pub compressed: usize,
    pub archived: usize,
    pub failures: Vec<MaintenanceFailure>,
}

/// Runs one destructive maintenance statement, returning rows affected.
/// On failure the error is logged (op + driver message) and recorded in
/// `failures`; returns 0 — a denied mutation is never counted as work.
fn exec_counted(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    sql: &str,
    params: impl rusqlite::Params,
) -> usize {
    crate::compaction::exec_counted_named("aging", conn, failures, op, sql, params)
}

struct AgeKind {
    table: &'static str,
    text_col: &'static str,
    alias_col: &'static str,
    ident: &'static str,
    join_alias: bool,
}

const MEMORIES: AgeKind = AgeKind {
    table: "memories",
    text_col: "text",
    alias_col: "source",
    ident: "memory",
    join_alias: false,
};
const DECISIONS: AgeKind = AgeKind {
    table: "decisions",
    text_col: "decision",
    alias_col: "context",
    ident: "decision",
    join_alias: true,
};

fn aging_body(text: String, alias: Option<String>, join: bool) -> String {
    match (join, alias) {
        (true, Some(ctx)) => format!("{text} — {ctx}"),
        _ => text,
    }
}

/// Loads the candidate rows for a tier-aging step. A failed SELECT is
/// recorded as a failure with zero rows (pre-fix it silently looked like an
/// empty candidate set).
fn select_tier_candidates<T>(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    op: &str,
    sql: &str,
    threshold: i64,
    map_row: fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    let mapped = conn.prepare(sql).and_then(|mut stmt| {
        let rows = stmt.query_map(params![threshold], map_row)?;
        Ok(rows.collect::<Result<Vec<T>, _>>()?)
    });
    mapped.unwrap_or_else(|err| {
        eprintln!("[aging] {op} FAILED: {err}");
        failures.push(MaintenanceFailure {
            op: op.to_string(),
            error: err.to_string(),
        });
        Vec::new()
    })
}

pub fn run_aging_pass(conn: &Connection) -> AgingReport {
    let mut report = AgingReport::default();
    let failures = &mut report.failures;
    for (kind, from, to, days, compress) in [
        (
            &MEMORIES,
            "fresh",
            "recent",
            FRESH_DAYS,
            compress_to_key_points as fn(&str) -> String,
        ),
        (
            &MEMORIES,
            "recent",
            "old",
            RECENT_DAYS,
            compress_to_one_liner,
        ),
        (
            &DECISIONS,
            "fresh",
            "recent",
            FRESH_DAYS,
            compress_to_key_points,
        ),
        (
            &DECISIONS,
            "recent",
            "old",
            RECENT_DAYS,
            compress_to_one_liner,
        ),
    ] {
        report.compressed += age_to_tier(conn, failures, kind, from, to, days, compress);
    }
    for kind in [&MEMORIES, &DECISIONS] {
        report.archived += archive_ancient(conn, failures, kind);
    }
    report.archived += gc_low_score(conn, failures);
    let orphans = cleanup_orphaned_embeddings(conn);
    if orphans > 0 {
        eprintln!("[aging] Cleaned {orphans} orphaned embeddings");
    }
    if report.compressed > 0 || report.archived > 0 {
        eprintln!(
            "[aging] Pass complete: {} compressed, {} archived",
            report.compressed, report.archived
        );
    }
    if !report.failures.is_empty() {
        eprintln!(
            "[aging] Pass had {} FAILED operation(s); see FAILED lines above, reported in AgingReport.failures",
            report.failures.len()
        );
    }
    report
}
fn age_to_tier(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    kind: &AgeKind,
    from_tier: &str,
    to_tier: &str,
    days: i64,
    compress: fn(&str) -> String,
) -> usize {
    let label = format!("age_{}_to_{to_tier}", kind.table);
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        &format!("{label} SELECT {}", kind.table),
        &format!(
            "SELECT id, {}, {} FROM {} WHERE status = 'active' AND pinned = 0 AND age_tier = '{from_tier}' AND {NOT_DURABLE} AND julianday('now') - julianday({UPDATED_CREATED_STAMP_SQL}) > ?1",
            kind.text_col, kind.alias_col, kind.table
        ),
        days,
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let mut count = 0;
    for (id, text, alias) in rows {
        if skip_immune_keys(
            conn,
            failures,
            &format!("{label} retrieval immunity"),
            &format!("{}::{id}", kind.ident),
            alias.as_deref(),
        ) {
            continue;
        }
        let compressed = compress(&aging_body(text, alias, kind.join_alias));
        count += exec_counted(
            conn,
            failures,
            &format!("{label} UPDATE {}", kind.table),
            &format!(
                "UPDATE {} SET compressed_text = ?1, age_tier = '{to_tier}', updated_at = datetime('now') WHERE id = ?2",
                kind.table
            ),
            params![compressed, id],
        );
    }
    count
}

fn archive_sql(kind: &AgeKind, extra: &str, window_idx: u8, thresh_idx: u8) -> String {
    format!(
        "UPDATE {} SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') WHERE status = 'active' AND pinned = 0 AND {NOT_DURABLE} {extra} AND {}",
        kind.table,
        not_immune_sql(
            kind.table,
            &format!("{}::", kind.ident),
            kind.alias_col,
            window_idx,
            thresh_idx
        )
    )
}

fn archive_ancient(
    conn: &Connection,
    failures: &mut Vec<MaintenanceFailure>,
    kind: &AgeKind,
) -> usize {
    exec_counted(
        conn,
        failures,
        &format!("archive_ancient_{} UPDATE {}", kind.table, kind.table),
        &archive_sql(
            kind,
            &format!(
                "AND age_tier = 'old' AND julianday('now') - julianday({UPDATED_CREATED_STAMP_SQL}) > ?1"
            ),
            2,
            3,
        ),
        params![
            OLD_DAYS,
            feedback::IMMUNITY_WINDOW_DAYS,
            feedback::IMMUNITY_THRESHOLD
        ],
    )
}
fn gc_low_score(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let mut count = 0usize;
    for kind in [&MEMORIES, &DECISIONS] {
        count += exec_counted(
            conn,
            failures,
            &format!("gc_low_score UPDATE {}", kind.table),
            &archive_sql(
                kind,
                &format!(
                    "AND score < ?1 AND julianday('now') - julianday({LAST_ACCESSED_CREATED_STAMP_SQL}) > ?2"
                ),
                3,
                4,
            ),
            params![
                GC_SCORE_THRESHOLD,
                GC_MIN_DAYS,
                feedback::IMMUNITY_WINDOW_DAYS,
                feedback::IMMUNITY_THRESHOLD
            ],
        );
    }
    if count > 0 {
        eprintln!("[aging] GC archived {count} low-score entries (score < {GC_SCORE_THRESHOLD})");
    }
    count
}
fn cleanup_orphaned_embeddings(_conn: &Connection) -> usize {
    0
}

mod compress;
pub use compress::get_display_text;
use compress::{compress_to_key_points, compress_to_one_liner};
