const RELATED_THRESHOLD: f64 = 0.40;
const AGREEMENT_THRESHOLD: f64 = 0.84;
const CORE_CONTRADICTION_OVERLAP_THRESHOLD: f64 = 0.35;

/// Deposit copies `decisions.type` onto these kinds. They are separate
/// observations: Jaccard must not merge a later note into them, supersede
/// them, or spend the recent-50 conflict window on them. The SQL `NOT IN`
/// lists in this file must match.
pub fn is_typed_evidence_kind(entry_type: &str) -> bool {
    matches!(
        entry_type.trim().to_ascii_lowercase().as_str(),
        "case"
            | "counterexample"
            | "attempt"
            | "failure"
            | "outcome"
            | "procedure"
            | "playbook"
            | "runbook"
            | "exception"
            | "constraint"
            | "policy"
            | "rule"
            | "convention"
            | "contract"
            | "obligation"
            | "checkpoint"
            | "preference"
            | "lesson"
            | "verified_result"
    )
}

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

fn query_with_optional_i64<T>(
    stmt: &mut rusqlite::Statement<'_>,
    owner_id: Option<i64>,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> rusqlite::Result<Vec<T>> {
    stmt.query_map(rusqlite::params_from_iter(owner_id), map)?
        .collect()
}

mod candidates;
mod detect;
mod jaccard;

pub use candidates::fetch_recent_decision_candidates;
pub use detect::{detect_conflict, scan_recent_decision_candidates};
pub use jaccard::{
    fold_jaccard_token, jaccard_similarity, jaccard_similarity_token_sets, jaccard_token_set,
};
