// SPDX-License-Identifier: MIT
use rusqlite::Connection;
use std::collections::HashSet;

const RELATED_THRESHOLD: f64 = 0.40;
const AGREEMENT_THRESHOLD: f64 = 0.84;
const CORE_CONTRADICTION_OVERLAP_THRESHOLD: f64 = 0.35;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictClassification {
    Agrees,
    Contradicts,
    Refines,
    Unrelated,
}

impl ConflictClassification {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agrees => "AGREES",
            Self::Contradicts => "CONTRADICTS",
            Self::Refines => "REFINES",
            Self::Unrelated => "UNRELATED",
        }
    }
}

#[derive(Debug, Clone)]
struct DecisionCandidate {
    id: i64,
    decision: String,
    source_agent: String,
    trust_score: f64,
}

#[allow(dead_code)]
pub struct ConflictResult {
    pub classification: ConflictClassification,
    pub is_conflict: bool,
    pub is_update: bool,
    pub matched_id: Option<i64>,
    pub matched_agent: Option<String>,
    pub matched_decision: Option<String>,
    pub matched_trust_score: Option<f64>,
    pub similarity_jaccard: f64,
    pub similarity_cosine: Option<f64>,
}

impl ConflictResult {
    fn unrelated() -> Self {
        Self {
            classification: ConflictClassification::Unrelated,
            is_conflict: false,
            is_update: false,
            matched_id: None,
            matched_agent: None,
            matched_decision: None,
            matched_trust_score: None,
            similarity_jaccard: 0.0,
            similarity_cosine: None,
        }
    }

    fn from_candidate(
        classification: ConflictClassification,
        candidate: &DecisionCandidate,
        source_agent: &str,
        similarity_jaccard: f64,
        similarity_cosine: Option<f64>,
    ) -> Self {
        let is_conflict = matches!(classification, ConflictClassification::Contradicts);
        let is_update = matches!(classification, ConflictClassification::Refines)
            || (matches!(classification, ConflictClassification::Agrees)
                && candidate.source_agent == source_agent);

        Self {
            classification,
            is_conflict,
            is_update,
            matched_id: Some(candidate.id),
            matched_agent: Some(candidate.source_agent.clone()),
            matched_decision: Some(candidate.decision.clone()),
            matched_trust_score: Some(candidate.trust_score),
            similarity_jaccard,
            similarity_cosine,
        }
    }
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
    owner_id: Option<i64>,
) -> Result<ConflictResult, String> {
    let (sql, has_owner_scope) = if owner_id.is_some() {
        (
            "SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) \
             FROM decisions \
             WHERE owner_id = ?1 \
             AND status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             ORDER BY id DESC \
             LIMIT 50",
            true,
        )
    } else {
        (
            "SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) \
             FROM decisions \
             WHERE status = 'active' \
             AND (expires_at IS NULL OR expires_at > datetime('now')) \
             ORDER BY id DESC \
             LIMIT 50",
            false,
        )
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("Failed to prepare conflict query: {e}"))?;

    let rows: Vec<DecisionCandidate> = if has_owner_scope {
        stmt.query_map([owner_id.unwrap_or_default()], |row| {
            Ok(DecisionCandidate {
                id: row.get(0)?,
                decision: row.get(1)?,
                source_agent: row.get(2)?,
                trust_score: row.get(3)?,
            })
        })
        .map_err(|e| format!("Failed to query decisions: {e}"))?
        .filter_map(|r| r.ok())
        .collect()
    } else {
        stmt.query_map([], |row| {
            Ok(DecisionCandidate {
                id: row.get(0)?,
                decision: row.get(1)?,
                source_agent: row.get(2)?,
                trust_score: row.get(3)?,
            })
        })
        .map_err(|e| format!("Failed to query decisions: {e}"))?
        .filter_map(|r| r.ok())
        .collect()
    };

    let mut best_sim = 0.0_f64;
    let mut best_candidate: Option<DecisionCandidate> = None;

    for candidate in &rows {
        let sim = jaccard_similarity(decision, &candidate.decision);
        if sim > best_sim {
            best_sim = sim;
            best_candidate = Some(candidate.clone());
        }
    }

    let Some(best_candidate) = best_candidate else {
        return Ok(ConflictResult::unrelated());
    };

    if best_sim < RELATED_THRESHOLD {
        return Ok(ConflictResult::unrelated());
    }

    let classification = classify_relation(decision, source_agent, &best_candidate, best_sim);
    Ok(ConflictResult::from_candidate(
        classification,
        &best_candidate,
        source_agent,
        best_sim,
        None,
    ))
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
#[allow(dead_code)]
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
             WHERE d.status = 'active' \
             AND (d.expires_at IS NULL OR d.expires_at > datetime('now'))",
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
        let classification = if best_agent.as_deref() == Some(source_agent) {
            ConflictClassification::Refines
        } else {
            ConflictClassification::Contradicts
        };
        Some(ConflictResult {
            classification,
            is_conflict: matches!(classification, ConflictClassification::Contradicts),
            is_update: matches!(classification, ConflictClassification::Refines),
            matched_id: best_id,
            matched_agent: best_agent,
            matched_decision: None,
            matched_trust_score: None,
            similarity_jaccard: 0.0,
            similarity_cosine: Some(best_sim as f64),
        })
    } else if best_sim > MERGE_THRESHOLD && best_agent.as_deref() == Some(source_agent) {
        let classification = ConflictClassification::Agrees;
        Some(ConflictResult {
            classification,
            is_conflict: false,
            is_update: true,
            matched_id: best_id,
            matched_agent: best_agent,
            matched_decision: None,
            matched_trust_score: None,
            similarity_jaccard: 0.0,
            similarity_cosine: Some(best_sim as f64),
        })
    } else {
        None
    }
}

fn classify_relation(
    incoming_decision: &str,
    incoming_agent: &str,
    candidate: &DecisionCandidate,
    similarity_jaccard: f64,
) -> ConflictClassification {
    if similarity_jaccard < RELATED_THRESHOLD {
        return ConflictClassification::Unrelated;
    }

    if contradiction_signal(incoming_decision, &candidate.decision, similarity_jaccard) {
        return ConflictClassification::Contradicts;
    }

    if similarity_jaccard >= AGREEMENT_THRESHOLD {
        return ConflictClassification::Agrees;
    }

    if candidate.source_agent == incoming_agent || similarity_jaccard >= RELATED_THRESHOLD {
        return ConflictClassification::Refines;
    }

    ConflictClassification::Unrelated
}

fn contradiction_signal(a: &str, b: &str, similarity_jaccard: f64) -> bool {
    if similarity_jaccard < RELATED_THRESHOLD {
        return false;
    }

    let tokens_a = semantic_tokens(a);
    let tokens_b = semantic_tokens(b);

    let neg_a = has_negation(&tokens_a);
    let neg_b = has_negation(&tokens_b);
    if neg_a == neg_b {
        return has_polarity_flip(&tokens_a, &tokens_b) && similarity_jaccard >= 0.55;
    }

    let core_a = strip_negation_tokens(&tokens_a);
    let core_b = strip_negation_tokens(&tokens_b);
    let overlap = jaccard_similarity_sets(&core_a, &core_b);
    overlap >= CORE_CONTRADICTION_OVERLAP_THRESHOLD
}

fn semantic_tokens(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 1)
        .map(|token| token.to_string())
        .collect()
}

fn has_negation(tokens: &HashSet<String>) -> bool {
    const NEGATION_TOKENS: &[&str] = &[
        "not",
        "never",
        "no",
        "without",
        "avoid",
        "dont",
        "can't",
        "cannot",
        "disable",
        "disabled",
        "forbid",
        "forbidden",
        "against",
    ];
    NEGATION_TOKENS.iter().any(|token| tokens.contains(*token))
}

fn strip_negation_tokens(tokens: &HashSet<String>) -> HashSet<String> {
    const NEGATION_TOKENS: &[&str] = &[
        "not",
        "never",
        "no",
        "without",
        "avoid",
        "dont",
        "can't",
        "cannot",
        "disable",
        "disabled",
        "forbid",
        "forbidden",
        "against",
    ];
    tokens
        .iter()
        .filter(|token| !NEGATION_TOKENS.contains(&token.as_str()))
        .cloned()
        .collect()
}

fn has_polarity_flip(tokens_a: &HashSet<String>, tokens_b: &HashSet<String>) -> bool {
    const FLIP_PAIRS: &[(&str, &str)] = &[
        ("always", "never"),
        ("must", "never"),
        ("allow", "forbid"),
        ("enable", "disable"),
        ("use", "avoid"),
    ];

    FLIP_PAIRS.iter().any(|(lhs, rhs)| {
        (tokens_a.contains(*lhs) && tokens_b.contains(*rhs))
            || (tokens_a.contains(*rhs) && tokens_b.contains(*lhs))
    })
}

fn jaccard_similarity_sets(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = (left.len() + right.len()) as f64 - intersection;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
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

        // Same content, same agent => agreement + compatibility update flag
        let result =
            detect_conflict(&conn, "Cortex uses SQLite for storage", "claude", None).unwrap();
        assert!(result.is_update);
        assert!(!result.is_conflict);
        assert_eq!(result.classification, ConflictClassification::Agrees);

        // Same content, different agent => agreement
        let result =
            detect_conflict(&conn, "Cortex uses SQLite for storage", "droid", None).unwrap();
        assert!(!result.is_conflict);
        assert!(!result.is_update);
        assert_eq!(result.classification, ConflictClassification::Agrees);

        // Contradicting intent => contradiction
        let result = detect_conflict(&conn, "Never use SQLite for storage", "droid", None).unwrap();
        assert_eq!(result.classification, ConflictClassification::Contradicts);
        assert!(result.is_conflict);

        // Different content => no conflict
        let result =
            detect_conflict(&conn, "Something totally different and new", "claude", None).unwrap();
        assert!(!result.is_conflict);
        assert!(!result.is_update);
        assert_eq!(result.classification, ConflictClassification::Unrelated);
    }

    #[test]
    fn test_detect_conflict_ignores_expired_decisions() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, expires_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now', '-1 second'))",
            rusqlite::params![
                "Cortex uses SQLite for storage",
                "test",
                "decision",
                "claude",
                "active"
            ],
        )
        .unwrap();

        let result =
            detect_conflict(&conn, "Cortex uses SQLite for storage", "claude", None).unwrap();
        assert!(!result.is_conflict);
        assert!(!result.is_update);
    }

    #[test]
    fn test_detect_conflict_scopes_by_owner_when_requested() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn.execute(
            "ALTER TABLE decisions ADD COLUMN owner_id INTEGER DEFAULT 0",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "Always use sqlite for local memory",
                "owner-one",
                "decision",
                "claude",
                "active",
                1_i64
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, owner_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                "Never use sqlite for local memory",
                "owner-two",
                "decision",
                "droid",
                "active",
                2_i64
            ],
        )
        .unwrap();

        let result = detect_conflict(
            &conn,
            "Always use sqlite for local memory",
            "claude",
            Some(1),
        )
        .unwrap();
        assert_eq!(result.matched_id, Some(1));
    }
}
