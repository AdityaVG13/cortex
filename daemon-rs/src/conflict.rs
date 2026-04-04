use rusqlite::Connection;
use std::collections::HashSet;

pub struct ConflictResult {
    pub is_conflict: bool,
    pub is_update: bool,
    pub matched_id: Option<i64>,
    pub matched_agent: Option<String>,
}

/// Jaccard similarity between two strings (word-level).
/// Matches the Node.js jaccardSimilarity: splits on whitespace,
/// filters tokens shorter than 2 chars, lowercases.
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<String> = a
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect();
    let set_b: HashSet<String> = b
        .split_whitespace()
        .filter(|w| w.len() > 1)
        .map(|w| w.to_lowercase())
        .collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = (set_a.len() + set_b.len()) as f64 - intersection;
    if union == 0.0 {
        return 0.0;
    }
    intersection / union
}

/// Detect conflicts by checking the last 50 active decisions.
/// Same agent + sim > 0.6  => update (supersede old)
/// Different agent + sim > 0.6 => conflict (disputed)
pub fn detect_conflict(
    conn: &Connection,
    decision: &str,
    source_agent: &str,
) -> Result<ConflictResult, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, decision, source_agent \
             FROM decisions \
             WHERE status = 'active' \
             ORDER BY id DESC \
             LIMIT 50",
        )
        .map_err(|e| format!("Failed to prepare conflict query: {e}"))?;

    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| format!("Failed to query decisions: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    let mut best_sim = 0.0_f64;
    let mut best_id: Option<i64> = None;
    let mut best_agent: Option<String> = None;

    for (id, text, agent) in &rows {
        let sim = jaccard_similarity(decision, text);
        if sim > best_sim {
            best_sim = sim;
            best_id = Some(*id);
            best_agent = Some(agent.clone());
        }
    }

    if best_sim > 0.6 {
        // Threshold 0.6 matches Node.js
        if best_agent.as_deref() == Some(source_agent) {
            // Same agent, high similarity => update (supersede)
            Ok(ConflictResult {
                is_conflict: false,
                is_update: true,
                matched_id: best_id,
                matched_agent: best_agent,
            })
        } else {
            // Different agent, high similarity => conflict
            Ok(ConflictResult {
                is_conflict: true,
                is_update: false,
                matched_id: best_id,
                matched_agent: best_agent,
            })
        }
    } else {
        Ok(ConflictResult {
            is_conflict: false,
            is_update: false,
            matched_id: None,
            matched_agent: None,
        })
    }
}

/// Embedding-based conflict detection with semantic dedup.
///
/// Three tiers:
///   - cosine > 0.85: hard conflict/update (existing behavior)
///   - cosine 0.70-0.85, same agent: semantic merge (NEW -- dedup zone)
///   - cosine < 0.70: no conflict, proceed to Jaccard fallback
///
/// The merge tier prevents near-duplicate memories that waste context budget.
/// Instead of storing "use uv for python" alongside "always use uv, never pip",
/// it merges them into a single strengthened entry.
pub fn detect_conflict_cosine(
    decision: &str,
    source_agent: &str,
    engine: &crate::embeddings::EmbeddingEngine,
    conn: &Connection,
) -> Option<ConflictResult> {
    let new_vec = engine.embed(decision)?;

    let mut stmt = conn
        .prepare(
            "SELECT d.id, d.source_agent, e.vector \
             FROM decisions d \
             JOIN embeddings e ON e.target_type = 'decision' AND e.target_id = d.id \
             WHERE d.status = 'active'",
        )
        .ok()?;

    let rows: Vec<(i64, String, Vec<u8>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();

    let mut best_sim = 0.0f32;
    let mut best_id: Option<i64> = None;
    let mut best_agent: Option<String> = None;

    for (id, agent, blob) in &rows {
        let existing_vec = crate::embeddings::blob_to_vector(blob);
        let sim = crate::embeddings::cosine_similarity(&new_vec, &existing_vec);
        if sim > best_sim {
            best_sim = sim;
            best_id = Some(*id);
            best_agent = Some(agent.clone());
        }
    }

    const HARD_THRESHOLD: f32 = 0.85;
    const MERGE_THRESHOLD: f32 = 0.70;

    if best_sim > HARD_THRESHOLD {
        // Hard conflict/update (existing behavior)
        let is_update = best_agent.as_deref() == Some(source_agent);
        Some(ConflictResult {
            is_conflict: !is_update,
            is_update,
            matched_id: best_id,
            matched_agent: best_agent,
        })
    } else if best_sim > MERGE_THRESHOLD && best_agent.as_deref() == Some(source_agent) {
        // Semantic dedup: same agent, similar content → treat as update (supersede old)
        // The new entry replaces the old, keeping the brain lean
        Some(ConflictResult {
            is_conflict: false,
            is_update: true,
            matched_id: best_id,
            matched_agent: best_agent,
        })
    } else {
        None
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_identical() {
        assert!(jaccard_similarity("hello world foo", "hello world foo") > 0.99);
    }

    #[test]
    fn test_jaccard_similar() {
        assert!(jaccard_similarity("hello world foo", "hello world bar") > 0.3);
    }

    #[test]
    fn test_jaccard_different() {
        assert!(jaccard_similarity("completely different text", "nothing alike here at all") < 0.1);
    }

    #[test]
    fn test_jaccard_empty() {
        assert_eq!(jaccard_similarity("", ""), 1.0);
    }

    #[test]
    fn test_detect_conflict() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();

        // Insert a decision
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "Cortex uses SQLite for storage",
                "test",
                "decision",
                "claude",
                "active"
            ],
        )
        .unwrap();

        // Same content, same agent => update
        let result = detect_conflict(&conn, "Cortex uses SQLite for storage", "claude").unwrap();
        assert!(result.is_update);
        assert!(!result.is_conflict);

        // Same content, different agent => conflict
        let result = detect_conflict(&conn, "Cortex uses SQLite for storage", "droid").unwrap();
        assert!(result.is_conflict);
        assert!(!result.is_update);

        // Different content => no conflict
        let result =
            detect_conflict(&conn, "Something totally different and new", "claude").unwrap();
        assert!(!result.is_conflict);
        assert!(!result.is_update);
    }
}
