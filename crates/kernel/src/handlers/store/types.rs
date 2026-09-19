use crate::protocol::nonempty_opt;
use serde_json::{Value, json};
pub const HARD_MERGE_THRESHOLD: f32 = 0.92;
pub const REVIEW_MERGE_THRESHOLD: f32 = 0.90;
pub const JACCARD_MERGE_THRESHOLD: f64 = 0.70;
pub const MERGE_SCORE_BONUS: f64 = 5.0;
pub const TOO_VAGUE_THRESHOLD: i32 = 20;
pub const BENCHMARK_ENTRY_TYPE: &str = "benchmark";
pub const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex::";
pub const MAX_DECISION_CHARS: usize = 4096;
/// Context is bounded like the decision text: an unbounded field is a
/// resource-bound defect, not a feature.
pub const MAX_CONTEXT_CHARS: usize = 8192;
pub const MAX_EXPLICIT_TTL_SECONDS: i64 = 365 * 24 * 60 * 60;
pub fn is_benchmark_entry_type(entry_type: &str) -> bool {
    entry_type.eq_ignore_ascii_case(BENCHMARK_ENTRY_TYPE)
}
pub fn is_benchmark_source_agent(source_agent: &str) -> bool {
    crate::handlers::starts_with_ascii_ignore_case(
        source_agent.trim(),
        BENCHMARK_SOURCE_AGENT_PREFIX,
    )
}

pub use crate::handlers::{
    agent_identity, agent_match_params, ident_match_sql, optional_ident_match_sql, same_agent,
};
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionProvenance {
    pub source_client: String,
    pub source_model: Option<String>,
    pub reasoning_depth: String,
}
impl DecisionProvenance {
    pub fn from_fields(
        source_agent: &str,
        source_model: Option<&str>,
        reasoning_depth: Option<&str>,
    ) -> Self {
        let normalized_model = nonempty_opt(source_model).map(str::to_string);
        Self {
            source_client: normalize_source_client(source_agent),
            source_model: normalized_model,
            reasoning_depth: normalize_reasoning_depth(reasoning_depth),
        }
    }
    pub fn trust_score(&self, confidence: f64) -> f64 {
        compute_trust_score(confidence, self.source_model.as_deref())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityFactors {
    pub length_score: i32,
    pub specificity_bonus: i32,
    pub question_penalty: i32,
}
impl QualityFactors {
    pub fn as_json(&self) -> Value {
        json!({"length_score":self.length_score,"specificity_bonus":self.specificity_bonus,"question_penalty":self.question_penalty,})
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityAssessment {
    pub score: i32,
    pub factors: QualityFactors,
}
#[derive(Debug, Clone)]
pub struct SemanticCandidate {
    pub id: i64,
    pub decision: String,
    pub similarity: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticDedupAction {
    Insert,
    Merge {
        target_id: i64,
        similarity: f32,
        jaccard: f64,
    },
}
#[derive(Debug)]
pub enum StoreError {
    BadRequest(&'static str),
    Validation {
        message: &'static str,
        quality: i32,
        factors: QualityFactors,
    },
    Internal(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::BadRequest(message) => write!(f, "{message}"),
            StoreError::Validation {
                message, quality, ..
            } => write!(f, "{message} (quality {quality})"),
            StoreError::Internal(message) => write!(f, "{message}"),
        }
    }
}
impl From<String> for StoreError {
    fn from(value: String) -> Self {
        StoreError::Internal(value)
    }
}
/// Drop a trailing ` (model)` suffix and keep `[A-Za-z0-9_-]`. Empty becomes `empty`.
pub fn normalize_client_slug(raw: &str, empty: &str) -> String {
    let normalized: String = raw
        .split('(')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if normalized.is_empty() {
        empty.to_string()
    } else {
        normalized
    }
}
pub fn normalize_source_client(raw: &str) -> String {
    normalize_client_slug(raw, "unknown")
}
pub fn normalize_reasoning_depth(raw: Option<&str>) -> String {
    let normalized = nonempty_opt(raw)
        .map(|value| value.to_ascii_lowercase())
        .map(|value| {
            value
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' {
                        ch
                    } else if ch == ' ' || ch == '_' {
                        '-'
                    } else {
                        '\0'
                    }
                })
                .filter(|ch| *ch != '\0')
                .collect::<String>()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "single-shot".to_string());
    match normalized.as_str() {
        "chain-of-thought" | "single-shot" | "tool-assisted" | "multi-step" | "user-stated" => {
            normalized
        }
        _ => "single-shot".to_string(),
    }
}
/// LEGACY PROVENANCE WEIGHT. Source-model names are provenance, not authority:
/// this multiplier feeds `trust_score`, which `recall::engine::blend_importance`
/// mixes at 35% into ranking and `/import` and the health dump echo back.
/// It is retained only so existing rows keep their recorded value; the
/// promotion-rules bead replaces it with user policy and scoped verified
/// outcomes. Do not add new readers of `trust_score` as a truth signal.
pub fn model_weight(source_model: Option<&str>) -> f64 {
    let Some(model) = source_model.map(|value| value.to_ascii_lowercase()) else {
        return 0.70;
    };
    const WEIGHTS: &[(&[&str], f64)] = &[
        (&["opus"], 1.0),
        (&["sonnet"], 0.85),
        (&["gemini", "pro"], 0.80),
        (&["gemini"], 0.60),
        (&["qwen"], 0.50),
    ];
    WEIGHTS
        .iter()
        .find(|(needles, _)| needles.iter().all(|needle| model.contains(needle)))
        .map(|(_, weight)| *weight)
        .unwrap_or(0.70)
}
pub fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

pub fn compute_trust_score(confidence: f64, source_model: Option<&str>) -> f64 {
    let bounded_confidence = confidence.clamp(0.0, 1.0);
    let raw = bounded_confidence * model_weight(source_model);
    round4(raw).clamp(0.0, 1.0)
}
pub fn validate_explicit_ttl_seconds(ttl_seconds: Option<i64>) -> Result<Option<i64>, StoreError> {
    let Some(ttl_seconds) = ttl_seconds else {
        return Ok(None);
    };
    if ttl_seconds <= 0 {
        return Err(StoreError::BadRequest("ttl_seconds must be > 0"));
    }
    if ttl_seconds > MAX_EXPLICIT_TTL_SECONDS {
        return Err(StoreError::BadRequest(
            "ttl_seconds must be <= 31536000 (365 days)",
        ));
    }
    Ok(Some(ttl_seconds))
}

pub fn sql_owner_and(owner_id: Option<i64>) -> String {
    match owner_id {
        Some(id) => format!(" AND owner_id = {id}"),
        None => String::new(),
    }
}

pub fn with_store_savepoint<T>(
    conn: &mut rusqlite::Connection,
    body: impl FnOnce(&rusqlite::Connection) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let tx = conn
        .savepoint()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    let out = body(&tx)?;
    tx.commit()
        .map_err(|e| StoreError::Internal(e.to_string()))?;
    crate::db::checkpoint_wal_best_effort(conn);
    Ok(out)
}
