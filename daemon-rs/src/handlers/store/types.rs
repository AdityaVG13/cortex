// SPDX-License-Identifier: MIT
use serde_json::{json, Value};
pub(crate) const HARD_MERGE_THRESHOLD: f32 = 0.92;
pub(crate) const REVIEW_MERGE_THRESHOLD: f32 = 0.90;
pub(crate) const JACCARD_MERGE_THRESHOLD: f64 = 0.70;
pub(crate) const MERGE_SCORE_BONUS: f64 = 5.0;
pub(crate) const TOO_VAGUE_THRESHOLD: i32 = 20;
pub(crate) const BENCHMARK_ENTRY_TYPE: &str = "benchmark";
pub(crate) const BENCHMARK_SOURCE_AGENT_PREFIX: &str = "amb-cortex::";
pub(crate) const MAX_DECISION_CHARS: usize = 4096;
pub(crate) const MAX_EXPLICIT_TTL_SECONDS: i64 = 365 * 24 * 60 * 60;
pub(crate) fn is_benchmark_entry_type(entry_type: &str) -> bool {
    entry_type.eq_ignore_ascii_case(BENCHMARK_ENTRY_TYPE)
}
pub(crate) fn is_benchmark_source_agent(source_agent: &str) -> bool {
    source_agent.trim().to_ascii_lowercase().starts_with(BENCHMARK_SOURCE_AGENT_PREFIX)
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DecisionProvenance {
    pub(crate) source_client: String,
    pub(crate) source_model: Option<String>,
    pub(crate) reasoning_depth: String,
}
impl DecisionProvenance {
    pub(crate) fn from_fields(source_agent: &str, source_model: Option<&str>, reasoning_depth: Option<&str>) -> Self {
        let normalized_model = source_model.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
        Self {
            source_client: normalize_source_client(source_agent),
            source_model: normalized_model,
            reasoning_depth: normalize_reasoning_depth(reasoning_depth),
        }
    }
    pub(crate) fn trust_score(&self, confidence: f64) -> f64 {
        compute_trust_score(confidence, self.source_model.as_deref())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityFactors {
    pub(crate) length_score: i32,
    pub(crate) specificity_bonus: i32,
    pub(crate) question_penalty: i32,
}
impl QualityFactors {
    pub(crate) fn as_json(&self) -> Value {
        json!({
            "length_score": self.length_score,
            "specificity_bonus": self.specificity_bonus,
            "question_penalty": self.question_penalty,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QualityAssessment {
    pub(crate) score: i32,
    pub(crate) factors: QualityFactors,
}
#[derive(Debug, Clone)]
pub(crate) struct SemanticCandidate {
    pub(crate) id: i64,
    pub(crate) decision: String,
    pub(crate) similarity: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SemanticDedupAction {
    Insert,
    Merge { target_id: i64, similarity: f32, jaccard: f64 },
}
#[derive(Debug)]
pub(crate) enum StoreError {
    BadRequest(&'static str),
    Validation { message: &'static str, quality: i32, factors: QualityFactors },
    Internal(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::BadRequest(message) => write!(f, "{message}"),
            StoreError::Validation { message, quality, .. } => write!(f, "{message} (quality {quality})"),
            StoreError::Internal(message) => write!(f, "{message}"),
        }
    }
}
impl From<String> for StoreError {
    fn from(value: String) -> Self {
        StoreError::Internal(value)
    }
}
pub(crate) fn normalize_source_client(raw: &str) -> String {
    let before_model = raw.split('(').next().unwrap_or(raw).trim().to_ascii_lowercase();
    let normalized: String = before_model.chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_').collect();
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
    }
}
pub(crate) fn normalize_reasoning_depth(raw: Option<&str>) -> String {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
        "chain-of-thought" | "single-shot" | "tool-assisted" | "multi-step" | "user-stated" => normalized,
        _ => "single-shot".to_string(),
    }
}
pub(crate) fn model_weight(source_model: Option<&str>) -> f64 {
    let Some(model) = source_model.map(|value| value.to_ascii_lowercase()) else {
        return 0.70;
    };
    if model.contains("opus") {
        1.0
    } else if model.contains("sonnet") {
        0.85
    } else if model.contains("gemini") && model.contains("pro") {
        0.80
    } else if model.contains("gemini") {
        0.60
    } else if model.contains("qwen") {
        0.50
    } else {
        0.70
    }
}
pub(crate) fn compute_trust_score(confidence: f64, source_model: Option<&str>) -> f64 {
    let bounded_confidence = confidence.clamp(0.0, 1.0);
    let raw = bounded_confidence * model_weight(source_model);
    ((raw * 10_000.0).round() / 10_000.0).clamp(0.0, 1.0)
}
pub(crate) fn validate_explicit_ttl_seconds(ttl_seconds: Option<i64>) -> Result<Option<i64>, StoreError> {
    let Some(ttl_seconds) = ttl_seconds else {
        return Ok(None);
    };
    if ttl_seconds <= 0 {
        return Err(StoreError::BadRequest("ttl_seconds must be > 0"));
    }
    if ttl_seconds > MAX_EXPLICIT_TTL_SECONDS {
        return Err(StoreError::BadRequest("ttl_seconds must be <= 31536000 (365 days)"));
    }
    Ok(Some(ttl_seconds))
}
