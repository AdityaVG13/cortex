use rusqlite::Connection;
use rustc_hash::{FxBuildHasher, FxHashSet};
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
fn fold_jaccard_token(token: &str) -> String {
    if token.bytes().all(|byte| byte.is_ascii()) {
        if token.bytes().any(|byte| byte.is_ascii_uppercase()) {
            token.to_ascii_lowercase()
        } else {
            token.to_owned()
        }
    } else {
        token.to_lowercase()
    }
}

fn ascii_tokens_already_folded(text: &str) -> bool {
    text.bytes().all(|byte| !byte.is_ascii_uppercase())
}

fn jaccard_borrowed(a: &str, b: &str) -> f64 {
    let mut left = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for word in a.split_whitespace().filter(|word| word.len() > 1) {
        left.insert(word);
    }
    let mut right = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for word in b.split_whitespace().filter(|word| word.len() > 1) {
        right.insert(word);
    }
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let (smaller, larger) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    let intersection = smaller
        .iter()
        .filter(|token| larger.contains(*token))
        .count() as f64;
    let union = (left.len() + right.len()) as f64 - intersection;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    if ascii_tokens_already_folded(a) && ascii_tokens_already_folded(b) {
        jaccard_borrowed(a, b)
    } else {
        jaccard_similarity_token_sets(&jaccard_token_set(a), &jaccard_token_set(b))
    }
}

#[derive(Debug, Clone)]
pub struct RecentDecisionCandidate {
    pub id: i64,
    pub decision: String,
    pub source_agent: String,
    pub trust_score: f64,
    pub in_conflict_window: bool,
}

pub struct RecentDecisionScan {
    pub relation: ConflictResult,
    pub max_jaccard: f64,
}

fn fill_jaccard_tokens(text: &str, tokens: &mut FxHashSet<String>) {
    tokens.clear();
    for word in text.split_whitespace().filter(|word| word.len() > 1) {
        tokens.insert(fold_jaccard_token(word));
    }
}

pub fn jaccard_token_set(text: &str) -> FxHashSet<String> {
    let mut tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    fill_jaccard_tokens(text, &mut tokens);
    tokens
}

pub(crate) fn jaccard_similarity_token_sets(
    left: &FxHashSet<String>,
    right: &FxHashSet<String>,
) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let (smaller, larger) = if left.len() <= right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let intersection = smaller
        .iter()
        .filter(|token| larger.contains(*token))
        .count() as f64;
    let union = (left.len() + right.len()) as f64 - intersection;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn recent_candidate_to_decision_candidate(
    candidate: &RecentDecisionCandidate,
) -> DecisionCandidate {
    DecisionCandidate {
        id: candidate.id,
        decision: candidate.decision.clone(),
        source_agent: candidate.source_agent.clone(),
        trust_score: candidate.trust_score,
    }
}

pub fn fetch_recent_decision_candidates(
    conn: &Connection,
    owner_id: Option<i64>,
) -> Result<Vec<RecentDecisionCandidate>, String> {
    let (sql, has_owner_scope) = if owner_id.is_some() {
        (
            "SELECT id, decision, source_agent, trust_score, MAX(in_conflict_window) AS in_conflict_window \
             FROM ( \
                 SELECT id, decision, source_agent, trust_score, 1 AS in_conflict_window \
                 FROM ( \
                     SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) AS trust_score \
                     FROM decisions \
                     WHERE owner_id = ?1 \
                     AND status = 'active' \
                     AND (expires_at IS NULL OR expires_at > datetime('now')) \
                     ORDER BY id DESC \
                     LIMIT 50 \
                 ) \
                 UNION ALL \
                 SELECT id, decision, source_agent, trust_score, 0 AS in_conflict_window \
                 FROM ( \
                     SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) AS trust_score \
                     FROM decisions \
                     WHERE owner_id = ?1 \
                     AND status = 'active' \
                     AND (expires_at IS NULL OR expires_at > datetime('now')) \
                     ORDER BY created_at DESC \
                     LIMIT 50 \
                 ) \
             ) \
             GROUP BY id, decision, source_agent, trust_score \
             ORDER BY in_conflict_window DESC, id DESC",
            true,
        )
    } else {
        (
            "SELECT id, decision, source_agent, trust_score, MAX(in_conflict_window) AS in_conflict_window \
             FROM ( \
                 SELECT id, decision, source_agent, trust_score, 1 AS in_conflict_window \
                 FROM ( \
                     SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) AS trust_score \
                     FROM decisions \
                     WHERE status = 'active' \
                     AND (expires_at IS NULL OR expires_at > datetime('now')) \
                     ORDER BY id DESC \
                     LIMIT 50 \
                 ) \
                 UNION ALL \
                 SELECT id, decision, source_agent, trust_score, 0 AS in_conflict_window \
                 FROM ( \
                     SELECT id, decision, source_agent, COALESCE(trust_score, confidence, 0.8) AS trust_score \
                     FROM decisions \
                     WHERE status = 'active' \
                     AND (expires_at IS NULL OR expires_at > datetime('now')) \
                     ORDER BY created_at DESC \
                     LIMIT 50 \
                 ) \
             ) \
             GROUP BY id, decision, source_agent, trust_score \
             ORDER BY in_conflict_window DESC, id DESC",
            false,
        )
    };
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(|error| format!("Failed to prepare recent decision query: {error}"))?;
    let map_candidate = |row: &rusqlite::Row<'_>| {
        let in_conflict_window: i64 = row.get(4)?;
        Ok(RecentDecisionCandidate {
            id: row.get(0)?,
            decision: row.get(1)?,
            source_agent: row.get(2)?,
            trust_score: row.get(3)?,
            in_conflict_window: in_conflict_window != 0,
        })
    };
    if has_owner_scope {
        let candidates = stmt
            .query_map([owner_id.unwrap_or_default()], map_candidate)
            .map_err(|error| format!("Failed to query recent decisions: {error}"))?
            .filter_map(|row| row.ok())
            .collect::<Vec<_>>();
        Ok(candidates)
    } else {
        let candidates = stmt
            .query_map([], map_candidate)
            .map_err(|error| format!("Failed to query recent decisions: {error}"))?
            .filter_map(|row| row.ok())
            .collect::<Vec<_>>();
        Ok(candidates)
    }
}

pub fn scan_recent_decision_candidates(
    candidates: &[RecentDecisionCandidate],
    decision: &str,
    source_agent: &str,
    decision_tokens: &FxHashSet<String>,
) -> RecentDecisionScan {
    let mut max_jaccard = 0.0_f64;
    let mut best_conflict_sim = 0.0_f64;
    let mut best_conflict_idx: Option<usize> = None;

    let mut candidate_tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for (index, candidate) in candidates.iter().enumerate() {
        fill_jaccard_tokens(&candidate.decision, &mut candidate_tokens);
        let similarity = jaccard_similarity_token_sets(decision_tokens, &candidate_tokens);
        max_jaccard = max_jaccard.max(similarity);
        if candidate.in_conflict_window && similarity > best_conflict_sim {
            best_conflict_sim = similarity;
            best_conflict_idx = Some(index);
        }
    }

    let Some(best_candidate) =
        best_conflict_idx.map(|index| recent_candidate_to_decision_candidate(&candidates[index]))
    else {
        return RecentDecisionScan {
            relation: ConflictResult::unrelated(),
            max_jaccard,
        };
    };
    if best_conflict_sim < RELATED_THRESHOLD {
        return RecentDecisionScan {
            relation: ConflictResult::unrelated(),
            max_jaccard,
        };
    }
    let classification =
        classify_relation(decision, source_agent, &best_candidate, best_conflict_sim);
    RecentDecisionScan {
        relation: ConflictResult::from_candidate(
            classification,
            &best_candidate,
            source_agent,
            best_conflict_sim,
            None,
        ),
        max_jaccard,
    }
}

#[allow(dead_code)]
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
        .prepare_cached(sql)
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
    let incoming_tokens = jaccard_token_set(decision);
    let mut candidate_tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    let mut best_sim = 0.0_f64;
    let mut best_idx: Option<usize> = None;
    for (index, candidate) in rows.iter().enumerate() {
        fill_jaccard_tokens(&candidate.decision, &mut candidate_tokens);
        let sim = jaccard_similarity_token_sets(&incoming_tokens, &candidate_tokens);
        if sim > best_sim {
            best_sim = sim;
            best_idx = Some(index);
        }
    }
    let Some(best_candidate) = best_idx.map(|index| &rows[index]) else {
        return Ok(ConflictResult::unrelated());
    };
    if best_sim < RELATED_THRESHOLD {
        return Ok(ConflictResult::unrelated());
    }
    let classification = classify_relation(decision, source_agent, best_candidate, best_sim);
    Ok(ConflictResult::from_candidate(
        classification,
        best_candidate,
        source_agent,
        best_sim,
        None,
    ))
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
fn semantic_tokens(text: &str) -> FxHashSet<String> {
    let mut tokens = FxHashSet::with_capacity_and_hasher(16, FxBuildHasher);
    for token in text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 1)
    {
        tokens.insert(if token.bytes().any(|byte| byte.is_ascii_uppercase()) {
            token.to_ascii_lowercase()
        } else {
            token.to_owned()
        });
    }
    tokens
}
fn has_negation(tokens: &FxHashSet<String>) -> bool {
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
fn strip_negation_tokens(tokens: &FxHashSet<String>) -> FxHashSet<String> {
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
fn has_polarity_flip(tokens_a: &FxHashSet<String>, tokens_b: &FxHashSet<String>) -> bool {
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
fn jaccard_similarity_sets(left: &FxHashSet<String>, right: &FxHashSet<String>) -> f64 {
    jaccard_similarity_token_sets(left, right)
}
