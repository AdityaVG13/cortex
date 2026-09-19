use super::candidates::detect_sql;
use super::jaccard::{
    fill_jaccard_tokens, has_negation, has_polarity_flip, jaccard_similarity_token_sets,
    jaccard_token_set, semantic_tokens, strip_negation_tokens,
};
use super::{
    AGREEMENT_THRESHOLD, CORE_CONTRADICTION_OVERLAP_THRESHOLD, ConflictClassification,
    ConflictResult, DecisionCandidate, RELATED_THRESHOLD, RecentDecisionCandidate,
    RecentDecisionScan,
};
use rusqlite::Connection;
use rustc_hash::{FxBuildHasher, FxHashSet};

#[allow(dead_code)]
pub fn detect_conflict(
    conn: &Connection,
    decision: &str,
    source_agent: &str,
    owner_id: Option<i64>,
) -> Result<ConflictResult, String> {
    let sql = detect_sql(owner_id.is_some());
    let mut stmt = conn
        .prepare_cached(&sql)
        .map_err(|e| format!("Failed to prepare conflict query: {e}"))?;
    let map_row = |row: &rusqlite::Row<'_>| {
        Ok(DecisionCandidate {
            id: row.get(0)?,
            decision: row.get(1)?,
            source_agent: row.get(2)?,
            trust_score: row.get(3)?,
        })
    };
    let rows: Vec<DecisionCandidate> = super::query_with_optional_i64(&mut stmt, owner_id, map_row)
        .map_err(|e| format!("Failed to query decisions: {e}"))?;
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

pub(super) fn classify_relation(
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
    jaccard_similarity_token_sets(&core_a, &core_b) >= CORE_CONTRADICTION_OVERLAP_THRESHOLD
}
