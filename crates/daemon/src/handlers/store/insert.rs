use super::*;
use crate::api_types::RetentionClass;
use crate::conflict::{jaccard_similarity, ConflictClassification, ConflictResult};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub(crate) fn insert_decision_with_state(
    tx: &rusqlite::Transaction<'_>, decision: &str, context: Option<&str>, entry_type: &str, source_agent: &str, provenance: &DecisionProvenance,
    confidence: f64, trust_score: f64, quality: i32, retention_class: RetentionClass, expires_at: Option<&str>, ts: &str, owner_id: Option<i64>, status: &str,
    disputes_id: Option<i64>, supersedes_id: Option<i64>, surprise: Option<f64>,
) -> Result<i64, StoreError> {
    let surprise = surprise.map(round4);
    if let Some
(oid)=owner_id{tx.execute(
"INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, disputes_id, supersedes_id, owner_id, quality, retention_class, expires_at, observed_at, valid_from, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14, ?14, ?14, ?15, ?16, ?17, ?18)"
,params![decision,context,entry_type,source_agent,confidence,surprise,status,disputes_id,supersedes_id,oid,quality,retention_class
.as_str(),expires_at,ts,provenance.source_client.as_str(),provenance.source_model.as_deref(),provenance.reasoning_depth.as_str(),
trust_score,],)}else{tx.execute(
"INSERT INTO decisions \
             (decision, context, type, source_agent, confidence, surprise, status, disputes_id, supersedes_id, quality, retention_class, expires_at, observed_at, valid_from, created_at, updated_at, source_client, source_model, reasoning_depth, trust_score) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?13, ?13, ?14, ?15, ?16, ?17)"
,params![decision,context,entry_type,source_agent,confidence,surprise,status,disputes_id,supersedes_id,quality,retention_class.
as_str(),expires_at,ts,provenance.source_client.as_str(),provenance.source_model.as_deref(),provenance.reasoning_depth.as_str(),
trust_score,],)}.map_err(|e|StoreError::Internal(e.to_string()))?;
    Ok(tx.last_insert_rowid())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_conflict_record(
    tx: &rusqlite::Transaction<'_>, source_decision_id: Option<i64>, target_decision_id: i64, classification: ConflictClassification, similarity_jaccard: f64,
    similarity_cosine: Option<f64>, status: &str, resolution_strategy: Option<&str>, resolved_by: Option<&str>, ts: &str,
) -> Result<i64, StoreError> {
    let resolved_at = if status == "open" { None } else { Some(ts) };
    tx.execute(
"INSERT INTO decision_conflicts \
         (source_decision_id, target_decision_id, classification, similarity_jaccard, similarity_cosine, status, resolution_strategy, resolved_by, resolved_at, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
,params![source_decision_id,target_decision_id,classification.as_str(),round4(similarity_jaccard),similarity_cosine.map(round4),
status,resolution_strategy,resolved_by,resolved_at,ts,],).map_err(|e|StoreError::Internal(e.to_string()))?;
    Ok(tx.last_insert_rowid())
}
pub(crate) fn decorate_entry_with_relation(entry: &mut Value, relation: &ConflictResult, conflict_record: Option<Value>) {
    if let Some(object) = entry.as_object_mut() {
        object.insert("classification".to_string(), json!(relation.classification.as_str()));
        object.insert("relation".to_string(), relation_to_json(relation));
        if let Some(conflict_record) = conflict_record {
            object.insert("conflict".to_string(), conflict_record);
        }
    }
}
pub(crate) fn relation_to_json(relation: &ConflictResult) -> Value {
    json!({"matched_id":relation.matched_id,
"matched_agent":relation.matched_agent,"matched_trust_score":relation.matched_trust_score.map(round4),"similarity":{"jaccard":
round4(relation.similarity_jaccard),"cosine":relation.similarity_cosine.map(round4),},})
}
pub(crate) fn conflict_record_json(
    record_id: i64, source_decision_id: Option<i64>, target_decision_id: i64, classification: ConflictClassification, status: &str, strategy: Option<&str>,
) -> Value {
    json!({"id":record_id,"source_decision_id":source_decision_id,"target_decision_id":target_decision_id,
"classification":classification.as_str(),"status":status,"resolution_strategy":strategy,})
}
pub(crate) fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}
pub(crate) fn assess_quality(text: &str) -> QualityAssessment {
    let trimmed = text.trim();
    let len = if trimmed.is_ascii() { trimmed.len() } else { trimmed.chars().count() };
    let length_score = if len < 10 {
        0
    } else if len < 50 {
        30
    } else if len < 200 {
        70
    } else {
        100
    };
    let specificity_bonus = if has_specificity_markers(trimmed) { 20 } else { 0 };
    let question_penalty = if trimmed.ends_with('?') { -30 } else { 0 };
    let score = (length_score + specificity_bonus + question_penalty).clamp(0, 100);
    QualityAssessment { score, factors: QualityFactors { length_score, specificity_bonus, question_penalty } }
}
pub(crate) fn has_specificity_markers(text: &str) -> bool {
    let file_extensions = [".rs", ".go", ".py", ".ts", ".tsx", ".js", ".jsx", ".json", ".toml", ".yaml", ".yml", ".sql", ".md"];
    let code_prefixes = ["fn ", "func ", "def ", "class ", "struct ", "impl ", "select ", "insert ", "update ", "delete "];
    let has_path = text.contains('/') || text.contains('\\');
    let has_extension = file_extensions.iter().any(|ext| contains_ascii_ignore_case(text, ext));
    let has_function =
        text.contains("::") || text.contains("()") || text.contains("->") || code_prefixes.iter().any(|needle| contains_ascii_ignore_case(text, needle));
    let has_identifier = text.split_whitespace().any(|token| token.contains('_') && token.chars().any(|ch| ch.is_ascii_alphabetic()));
    has_path || has_extension || has_function || has_identifier
}
fn contains_ascii_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.as_bytes().windows(needle.len()).any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
#[allow(dead_code)]
pub(crate) fn choose_semantic_dedup_action(candidates: &[SemanticCandidate], incoming_text: &str) -> SemanticDedupAction {
    for candidate in candidates {
        let jaccard = jaccard_similarity(incoming_text, &candidate.decision);
        if should_merge_candidate(candidate.similarity, jaccard) {
            return SemanticDedupAction::Merge { target_id: candidate.id, similarity: candidate.similarity, jaccard };
        }
    }
    SemanticDedupAction::Insert
}
pub(crate) fn should_merge_candidate(similarity: f32, jaccard: f64) -> bool {
    if similarity > HARD_MERGE_THRESHOLD {
        return true;
    }
    (REVIEW_MERGE_THRESHOLD..=HARD_MERGE_THRESHOLD).contains(&similarity) && jaccard > JACCARD_MERGE_THRESHOLD
}

#[allow(dead_code)]
pub(crate) fn fetch_top_semantic_candidates(_conn: &Connection, _query_vector: &[f32], _owner_id: Option<i64>) -> Result<Vec<SemanticCandidate>, StoreError> {
    Ok(Vec::new())
}
