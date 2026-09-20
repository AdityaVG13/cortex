//! Term bridges (Loop 4): the paraphrase learner.
//!
//! On a successful outcome, every query token pairs with every token of
//! each used document (bounded cross product, both sides in the
//! `tokenize_cues` vocabulary). Pairs that recur across successes
//! (positive mass ≥ 2, unvetoed) expand future queries holding the
//! query-side term. Harmful reuse vetoes the pair. Principal-scoped;
//! bounded by maintenance trim, never on the hot path.

use rusqlite::{Connection, OptionalExtension, params};

pub const DDL: &str = "CREATE TABLE IF NOT EXISTS term_bridges (principal TEXT NOT NULL, query_term TEXT NOT NULL, doc_term TEXT NOT NULL, positive INTEGER NOT NULL DEFAULT 0, negative INTEGER NOT NULL DEFAULT 0, last_seen TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (principal, query_term, doc_term));";

pub const TRIM_KEEP_ROWS: i64 = 50_000;
/// Pairs are the bounded head of the sorted cross product, so volumes stay
/// predictable no matter how verbose a query or document is.
pub const MAX_QUERY_TERMS: usize = 8;
pub const MAX_DOC_TERMS: usize = 8;
pub const MAX_SOURCES: usize = 8;

pub fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

/// Credit (or veto, when harmful) the query × document term pairs.
/// Returns the pairs touched.
pub fn record_pairs(
    conn: &Connection,
    principal: &str,
    query_terms: &[String],
    doc_terms: &[String],
    harmful: bool,
) -> Result<usize, String> {
    ensure(conn)?;
    let mut query_terms: Vec<&str> = query_terms.iter().map(String::as_str).collect();
    query_terms.sort_unstable();
    query_terms.dedup();
    let mut doc_terms: Vec<&str> = doc_terms.iter().map(String::as_str).collect();
    doc_terms.sort_unstable();
    doc_terms.dedup();
    let mut touched = 0;
    for query_term in query_terms.into_iter().take(MAX_QUERY_TERMS) {
        if query_term.len() < 3 || query_term.len() > 64 {
            continue;
        }
        for doc_term in doc_terms.iter().take(MAX_DOC_TERMS) {
            if doc_term.len() < 3 || doc_term.len() > 64 || doc_term == &query_term {
                continue;
            }
            let (column, first_positive, first_negative) = if harmful {
                ("negative", 0, 1)
            } else {
                ("positive", 1, 0)
            };
            conn.execute(
                &format!(
                    "INSERT INTO term_bridges (principal, query_term, doc_term, positive, negative) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (principal, query_term, doc_term) DO UPDATE SET {column} = {column} + 1, last_seen = datetime('now')"
                ),
                params![principal, query_term, doc_term, first_positive, first_negative],
            )
            .map_err(|e| e.to_string())?;
            touched += 1;
        }
    }
    Ok(touched)
}

/// Document-side tokens for a `decision::N` / `memory::N` source.
/// Unresolvable sources yield nothing (skipped, never an error).
pub fn doc_terms(conn: &Connection, source: &str) -> Vec<String> {
    let (kind, id) = crate::handlers::feedback::parse_source(source);
    let Some(id) = id else {
        return Vec::new();
    };
    let (table, column) = match kind.as_str() {
        "decision" => ("decisions", "decision"),
        "memory" => ("memories", "text"),
        _ => return Vec::new(),
    };
    let text: Option<String> = conn
        .query_row(
            &format!("SELECT {column} FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None);
    let Some(text) = text else {
        return Vec::new();
    };
    let mut terms = crate::runtime::assembly::tokenize_cues(&text);
    terms.truncate(MAX_DOC_TERMS * 4);
    terms
}

/// Maintenance-only trim: keep the most recently seen pairs.
pub fn trim(conn: &Connection, keep: i64) -> Result<usize, String> {
    ensure(conn)?;
    let deleted = conn
        .execute(
            "DELETE FROM term_bridges WHERE rowid NOT IN (SELECT rowid FROM term_bridges ORDER BY last_seen DESC, principal ASC, query_term ASC, doc_term ASC LIMIT ?1)",
            params![keep.max(1)],
        )
        .map_err(|e| e.to_string())?;
    Ok(deleted)
}
