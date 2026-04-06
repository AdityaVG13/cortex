// SPDX-License-Identifier: MIT
//! Progressive Memory Aging — background worker that compresses old memories.
//!
//! Tiers:
//!   fresh   (0-3 days)   — full text, no compression
//!   recent  (3-14 days)  — compressed to key points (1-2 sentences)
//!   old     (14-60 days) — reduced to a single sentence
//!   ancient (60+ days)   — archived (status = 'archived'), only explicit search
//!
//! Pinned entries (pinned = 1) are immune to aging.
//! Compression uses extractive summarization (first sentence + key phrases)
//! to avoid depending on an LLM for the background job.

use crate::handlers::feedback;
use rusqlite::{params, Connection};

/// Age tier boundaries in days.
const FRESH_DAYS: i64 = 3;
const RECENT_DAYS: i64 = 14;
const OLD_DAYS: i64 = 60;

/// Score below which unretrieved entries are garbage-collected.
const GC_SCORE_THRESHOLD: f64 = 0.15;
/// Minimum days since last access before GC kicks in.
const GC_MIN_DAYS: i64 = 3;

/// Run one aging pass over memories and decisions.
/// Returns (compressed_count, archived_count).
pub fn run_aging_pass(conn: &Connection) -> (usize, usize) {
    let mut compressed = 0usize;
    let mut archived = 0usize;

    // ── Age memories ────────────────────────────────────────────────────────
    compressed += age_memories_to_recent(conn);
    compressed += age_memories_to_old(conn);
    archived += archive_ancient_memories(conn);

    // ── Age decisions ───────────────────────────────────────────────────────
    compressed += age_decisions_to_recent(conn);
    compressed += age_decisions_to_old(conn);
    archived += archive_ancient_decisions(conn);

    // ── Score-based garbage collection ──────────────────────────────────────
    // Archive entries whose score has decayed below threshold and haven't
    // been retrieved in GC_MIN_DAYS. These are noise (test entries, stale
    // decisions) that survived time-based aging but lost all relevance.
    archived += gc_low_score(conn);

    // ── Orphaned embedding cleanup ─────────────────────────────────────────
    let orphans = cleanup_orphaned_embeddings(conn);
    if orphans > 0 {
        eprintln!("[aging] Cleaned {orphans} orphaned embeddings");
    }

    if compressed > 0 || archived > 0 {
        eprintln!("[aging] Pass complete: {compressed} compressed, {archived} archived");
    }

    (compressed, archived)
}

// ─── Memory aging ───────────────────────────────────────────────────────────

fn age_memories_to_recent(conn: &Connection) -> usize {
    let mut count = 0;
    let rows: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, text, source FROM memories \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'fresh' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        )
        .and_then(|mut stmt| {
            let mapped = stmt.query_map(params![FRESH_DAYS], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            Ok(mapped.flatten().collect())
        })
        .unwrap_or_default();

    for (id, text, source) in rows {
        // Skip aging if this memory has strong retrieval feedback (frequently useful)
        if let Some(ref src) = source {
            if feedback::has_retrieval_immunity(conn, src) {
                continue;
            }
        }
        let compressed = compress_to_key_points(&text);
        let _ = conn.execute(
            "UPDATE memories SET compressed_text = ?1, age_tier = 'recent', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
        count += 1;
    }
    count
}

fn age_memories_to_old(conn: &Connection) -> usize {
    let mut count = 0;
    let rows: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, text, source FROM memories \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'recent' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        )
        .and_then(|mut stmt| {
            let mapped = stmt.query_map(params![RECENT_DAYS], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            Ok(mapped.flatten().collect())
        })
        .unwrap_or_default();

    for (id, text, source) in rows {
        if let Some(ref src) = source {
            if feedback::has_retrieval_immunity(conn, src) {
                continue;
            }
        }
        let compressed = compress_to_one_liner(&text);
        let _ = conn.execute(
            "UPDATE memories SET compressed_text = ?1, age_tier = 'old', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
        count += 1;
    }
    count
}

fn archive_ancient_memories(conn: &Connection) -> usize {
    conn.execute(
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![OLD_DAYS],
    )
    .unwrap_or(0)
}

// ─── Decision aging ─────────────────────────────────────────────────────────

fn age_decisions_to_recent(conn: &Connection) -> usize {
    let mut count = 0;
    let rows: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, decision, context FROM decisions \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'fresh' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        )
        .and_then(|mut stmt| {
            let mapped = stmt.query_map(params![FRESH_DAYS], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            Ok(mapped.flatten().collect())
        })
        .unwrap_or_default();

    for (id, decision, context) in rows {
        let full = match context {
            Some(ref ctx) => format!("{decision} — {ctx}"),
            None => decision,
        };
        let compressed = compress_to_key_points(&full);
        let _ = conn.execute(
            "UPDATE decisions SET compressed_text = ?1, age_tier = 'recent', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
        count += 1;
    }
    count
}

fn age_decisions_to_old(conn: &Connection) -> usize {
    let mut count = 0;
    let rows: Vec<(i64, String, Option<String>)> = conn
        .prepare(
            "SELECT id, decision, context FROM decisions \
             WHERE status = 'active' AND pinned = 0 \
             AND age_tier = 'recent' \
             AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        )
        .and_then(|mut stmt| {
            let mapped = stmt.query_map(params![RECENT_DAYS], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            Ok(mapped.flatten().collect())
        })
        .unwrap_or_default();

    for (id, decision, context) in rows {
        let full = match context {
            Some(ref ctx) => format!("{decision} — {ctx}"),
            None => decision,
        };
        let compressed = compress_to_one_liner(&full);
        let _ = conn.execute(
            "UPDATE decisions SET compressed_text = ?1, age_tier = 'old', updated_at = datetime('now') WHERE id = ?2",
            params![compressed, id],
        );
        count += 1;
    }
    count
}

fn archive_ancient_decisions(conn: &Connection) -> usize {
    conn.execute(
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND age_tier = 'old' \
         AND julianday('now') - julianday(COALESCE(updated_at, created_at)) > ?1",
        params![OLD_DAYS],
    )
    .unwrap_or(0)
}

// ─── Extractive compression ────────────────────────────────────────────────
// No LLM dependency — pure text extraction. Keeps first sentence and any
// sentences containing high-signal keywords.

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

// ─── Score-based garbage collection ────────────────────────────────────────

/// Archive entries with deeply decayed scores that haven't been retrieved
/// recently. These are noise entries (test data, stale one-offs) that
/// time-based aging hasn't caught yet.
fn gc_low_score(conn: &Connection) -> usize {
    let mut count = 0usize;

    count += conn.execute(
        "UPDATE memories SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(last_accessed, created_at)) > ?2",
        params![GC_SCORE_THRESHOLD, GC_MIN_DAYS],
    ).unwrap_or(0);

    count += conn.execute(
        "UPDATE decisions SET status = 'archived', age_tier = 'ancient', updated_at = datetime('now') \
         WHERE status = 'active' AND pinned = 0 \
         AND score < ?1 \
         AND julianday('now') - julianday(COALESCE(last_accessed, created_at)) > ?2",
        params![GC_SCORE_THRESHOLD, GC_MIN_DAYS],
    ).unwrap_or(0);

    if count > 0 {
        eprintln!("[aging] GC archived {count} low-score entries (score < {GC_SCORE_THRESHOLD})");
    }
    count
}

/// Remove embeddings for entries that are no longer active.
/// These accumulate when entries are superseded or archived.
fn cleanup_orphaned_embeddings(conn: &Connection) -> usize {
    let mut count = 0usize;

    count += conn.execute(
        "DELETE FROM embeddings WHERE target_type = 'memory' \
         AND NOT EXISTS (SELECT 1 FROM memories m WHERE m.id = embeddings.target_id AND m.status = 'active')",
        [],
    ).unwrap_or(0);

    count += conn.execute(
        "DELETE FROM embeddings WHERE target_type = 'decision' \
         AND NOT EXISTS (SELECT 1 FROM decisions d WHERE d.id = embeddings.target_id AND d.status = 'active')",
        [],
    ).unwrap_or(0);

    count
}

// ─── Retrieval-time helper ──────────────────────────────────────────────────

/// Get the best available text for a memory: compressed if aged, full if fresh.
/// Called by recall to serve age-appropriate content.
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

