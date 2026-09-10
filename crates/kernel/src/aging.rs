use crate::compaction::MaintenanceFailure;
use crate::handlers::feedback;
use rusqlite::{params, Connection};
const FRESH_DAYS: i64 = 3;
const RECENT_DAYS: i64 = 14;
const OLD_DAYS: i64 = 60;
const GC_SCORE_THRESHOLD: f64 = 0.15;
const GC_MIN_DAYS: i64 = 3;

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
        Ok(rows.flatten().collect::<Vec<T>>())
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
        "SELECT id, text, source FROM memories \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'fresh' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
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
        if let Some(ref src) = source {
            if feedback::has_retrieval_immunity(conn, src) {
                continue;
            }
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
        "SELECT id, text, source FROM memories \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'recent' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
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
        if let Some(ref src) = source {
            if feedback::has_retrieval_immunity(conn, src) {
                continue;
            }
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
    exec_counted(
        conn,
        failures,
        "archive_ancient_memories UPDATE memories",
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![OLD_DAYS],
    )
}
fn age_decisions_to_recent(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let rows: Vec<(i64, String, Option<String>)> = select_tier_candidates(
        conn,
        failures,
        "age_decisions_to_recent SELECT decisions",
        "SELECT id, decision, context FROM decisions \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'fresh' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
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
        "SELECT id, decision, context FROM decisions \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'recent' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
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
    exec_counted(
        conn,
        failures,
        "archive_ancient_decisions UPDATE decisions",
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![OLD_DAYS],
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
/// rows to the archive placement, never a durable-retention row.
fn gc_low_score(conn: &Connection, failures: &mut Vec<MaintenanceFailure>) -> usize {
    let mut count = 0usize;
    count += exec_counted(
        conn,
        failures,
        "gc_low_score UPDATE memories",
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND COALESCE(retention_class, 'operational') != 'durable' \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(last_accessed, created_at)) > ?2",
        params![GC_SCORE_THRESHOLD, GC_MIN_DAYS],
    );
    count += exec_counted(
        conn,
        failures,
        "gc_low_score UPDATE decisions",
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND COALESCE(retention_class, 'operational') != 'durable' \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(last_accessed, created_at)) > ?2",
        params![GC_SCORE_THRESHOLD, GC_MIN_DAYS],
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
