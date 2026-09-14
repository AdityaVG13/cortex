use crate::compaction::MaintenanceFailure;
use crate::handlers::feedback;
use rusqlite::{params, Connection};

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
    let Some(src) = source.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    match feedback::has_retrieval_immunity(conn, src) {
        Ok(true) => true,
        Ok(false) => false,
        Err(err) => {
            failures.push(MaintenanceFailure {
                op: op.to_string(),
                error: err,
            });
            true
        }
    }
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

fn not_immune_sql(table: &str, ident_prefix: &str, alias_col: &str, window_idx: u8, thresh_idx: u8) -> String {
    format!(
        "(SELECT COUNT(*) FROM recall_feedback \
            WHERE signal > 0 AND julianday('now') - julianday(created_at) <= ?{window_idx} \
              AND result_source = '{ident_prefix}' || {table}.id) < ?{thresh_idx} \
         AND (NULLIF(TRIM({table}.{alias_col}), '') IS NULL \
              OR (SELECT COUNT(*) FROM recall_feedback \
                    WHERE signal > 0 AND julianday('now') - julianday(created_at) <= ?{window_idx} \
                      AND result_source = {table}.{alias_col}) < ?{thresh_idx})"
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
    match conn.execute(sql, params) {
        Ok(n) => n,
        Err(err) => {
            eprintln!("[aging] {op} FAILED: {err}");
            failures.push(MaintenanceFailure {
                op: op.to_string(),
                error: err.to_string(),
            });
            0
        }
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
    match mapped {
        Ok(rows) => rows,
        Err(err) => {
            eprintln!("[aging] {op} FAILED: {err}");
            failures.push(MaintenanceFailure {
                op: op.to_string(),
                error: err.to_string(),
            });
            Vec::new()
        }
    }
}

pub fn run_aging_pass(conn: &Connection) -> AgingReport {
    let mut report = AgingReport::default();
    let failures = &mut report.failures;
    report.compressed += age_memories_to_recent(conn, failures);
    report.compressed += age_memories_to_old(conn, failures);
    report.archived += archive_ancient_memories(conn, failures);
    report.compressed += age_decisions_to_recent(conn, failures);
    report.compressed += age_decisions_to_old(conn, failures);
    report.archived += archive_ancient_decisions(conn, failures);
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
        eprintln!("[aging] Pass had {} FAILED operation(s); see FAILED lines above, reported in AgingReport.failures", report.failures.len());
    }
    report
}
fn age_memories_to_recent(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        "age_memories_to_recent SELECT memories",
        &format!(
            "SELECT id, text, source FROM memories \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'fresh' \
             AND {NOT_DURABLE} \
             AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1"
        ),
        FRESH_DAYS,
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let mut count = 0;
    for (id, text, source) in rows {
        if skip_immune_keys(
            conn,
            failures,
            "age_memories_to_recent retrieval immunity",
            &format!("memory::{id}"),
            source.as_deref(),
        ) {
            continue;
        }
        let compressed = compress_to_key_points(&text);
        count += exec_counted(
            conn,
            failures,
            "age_memories_to_recent UPDATE memories",
            "UPDATE memories SET compressed_text = ?1, age_tier = 'recent', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
    }
    count
}
fn age_memories_to_old(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        "age_memories_to_old SELECT memories",
        &format!(
            "SELECT id, text, source FROM memories \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'recent' \
             AND {NOT_DURABLE} \
             AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1"
        ),
        RECENT_DAYS,
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let mut count = 0;
    for (id, text, source) in rows {
        if skip_immune_keys(
            conn,
            failures,
            "age_memories_to_old retrieval immunity",
            &format!("memory::{id}"),
            source.as_deref(),
        ) {
            continue;
        }
        let compressed = compress_to_one_liner(&text);
        count += exec_counted(
            conn,
            failures,
            "age_memories_to_old UPDATE memories",
            "UPDATE memories SET compressed_text = ?1, age_tier = 'old', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
    }
    count
}
fn archive_ancient_memories(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let sql = format!(
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND {NOT_DURABLE} \
         AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1 \
         AND {}",
        not_immune_sql("memories", "memory::", "source", 2, 3)
    );
    exec_counted(
        conn,
        failures,
        "archive_ancient_memories UPDATE memories",
        &sql,
        params![
            OLD_DAYS,
            feedback::IMMUNITY_WINDOW_DAYS,
            feedback::IMMUNITY_THRESHOLD
        ],
    )
}
fn age_decisions_to_recent(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        "age_decisions_to_recent SELECT decisions",
        &format!(
            "SELECT id, decision, context FROM decisions \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'fresh' \
             AND {NOT_DURABLE} \
             AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1"
        ),
        FRESH_DAYS,
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let mut count = 0;
    for (id, decision, context) in rows {
        if skip_immune_keys(
            conn,
            failures,
            "age_decisions_to_recent retrieval immunity",
            &format!("decision::{id}"),
            context.as_deref(),
        ) {
            continue;
        }
        let full = match context {
            Some(ref ctx) => format!("{decision} — {ctx}"),
            None => decision,
        };
        let compressed = compress_to_key_points(&full);
        count += exec_counted(
            conn,
            failures,
            "age_decisions_to_recent UPDATE decisions",
            "UPDATE decisions SET compressed_text = ?1, age_tier = 'recent', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
    }
    count
}
fn age_decisions_to_old(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        "age_decisions_to_old SELECT decisions",
        &format!(
            "SELECT id, decision, context FROM decisions \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'recent' \
             AND {NOT_DURABLE} \
             AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1"
        ),
        RECENT_DAYS,
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let mut count = 0;
    for (id, decision, context) in rows {
        if skip_immune_keys(
            conn,
            failures,
            "age_decisions_to_old retrieval immunity",
            &format!("decision::{id}"),
            context.as_deref(),
        ) {
            continue;
        }
        let full = match context {
            Some(ref ctx) => format!("{decision} — {ctx}"),
            None => decision,
        };
        let compressed = compress_to_one_liner(&full);
        count += exec_counted(
            conn,
            failures,
            "age_decisions_to_old UPDATE decisions",
            "UPDATE decisions SET compressed_text = ?1, age_tier = 'old', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
    }
    count
}
fn archive_ancient_decisions(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let sql = format!(
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND {NOT_DURABLE} \
         AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))) > ?1 \
         AND {}",
        not_immune_sql("decisions", "decision::", "context", 2, 3)
    );
    exec_counted(
        conn,
        failures,
        "archive_ancient_decisions UPDATE decisions",
        &sql,
        params![
            OLD_DAYS,
            feedback::IMMUNITY_WINDOW_DAYS,
            feedback::IMMUNITY_THRESHOLD
        ],
    )
}
fn compress_to_key_points(text: &str) -> String {
    let sentences: Vec<&str> = text
        .split(['.', '\n'])
        .map(|s| s.trim())
        .filter(|s| s.len() > 5)
        .collect();
    if sentences.len() <= 2 {
        return text.chars().take(300).collect();
    }
    let high_signal = [
        "must",
        "never",
        "always",
        "critical",
        "important",
        "decision",
        "fixed",
        "bug",
        "error",
        "confirmed",
        "approved",
        "rejected",
        "architecture",
        "design",
        "migration",
        "breaking",
        "security",
    ];
    let mut kept: Vec<&str> = Vec::new();
    kept.push(sentences[0]);
    for sentence in &sentences[1..] {
        let lower = sentence.to_lowercase();
        if high_signal.iter().any(|kw| lower.contains(kw)) && kept.len() < 4 {
            kept.push(sentence);
        }
    }
    let result = kept.join(". ");
    if result.len() > 300 {
        result.chars().take(300).collect::<String>() + "..."
    } else {
        result
    }
}
fn compress_to_one_liner(text: &str) -> String {
    let first_sentence = text
        .split(['.', '\n'])
        .map(|s| s.trim())
        .find(|s| s.len() > 5)
        .unwrap_or(text);
    first_sentence.chars().take(120).collect()
}
/// Salience is not deletion authority: low score can demote operational
/// rows to the archive placement, never a durable-retention row. Time-based
/// aging uses the same durable skip.
fn gc_low_score(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let mut count = 0usize;
    let mem_sql = format!(
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND {NOT_DURABLE} \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(created_at), ''))) > ?2 \
         AND {}",
        not_immune_sql("memories", "memory::", "source", 3, 4)
    );
    count += exec_counted(
        conn,
        failures,
        "gc_low_score UPDATE memories",
        &mem_sql,
        params![
            GC_SCORE_THRESHOLD,
            GC_MIN_DAYS,
            feedback::IMMUNITY_WINDOW_DAYS,
            feedback::IMMUNITY_THRESHOLD
        ],
    );
    let dec_sql = format!(
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND {NOT_DURABLE} \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(created_at), ''))) > ?2 \
         AND {}",
        not_immune_sql("decisions", "decision::", "context", 3, 4)
    );
    count += exec_counted(
        conn,
        failures,
        "gc_low_score UPDATE decisions",
        &dec_sql,
        params![
            GC_SCORE_THRESHOLD,
            GC_MIN_DAYS,
            feedback::IMMUNITY_WINDOW_DAYS,
            feedback::IMMUNITY_THRESHOLD
        ],
    );
    if count > 0 {
        eprintln!("[aging] GC archived {count} low-score entries (score < {GC_SCORE_THRESHOLD})");
    }
    count
}
fn cleanup_orphaned_embeddings(_conn: &Connection) -> usize {
    0
}

pub fn get_display_text(text: &str, compressed_text: &Option<String>, age_tier: &str) -> String {
    match age_tier {
        "fresh" => text.to_string(),
        _ => compressed_text
            .as_ref()
            .filter(|c| !c.is_empty())
            .cloned()
            .unwrap_or_else(|| text.to_string()),
    }
}
