// SPDX-License-Identifier: MIT
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Sql,
}

impl ExportFormat {
    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "sql" => Some(Self::Sql),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StoreRequest {
    pub decision: Option<String>,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub source_agent: Option<String>,
    pub source_model: Option<String>,
    pub confidence: Option<f64>,
    pub reasoning_depth: Option<String>,
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportPayload {
    pub memories: Option<Vec<ImportMemory>>,
    pub decisions: Option<Vec<ImportDecision>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportMemory {
    pub text: String,
    pub source: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub tags: Option<String>,
    pub source_agent: Option<String>,
    pub source_client: Option<String>,
    pub source_model: Option<String>,
    pub confidence: Option<f64>,
    pub reasoning_depth: Option<String>,
    pub trust_score: Option<f64>,
    pub score: Option<f64>,
    pub observed_at: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportDecision {
    pub decision: String,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub source_agent: Option<String>,
    pub source_client: Option<String>,
    pub source_model: Option<String>,
    pub confidence: Option<f64>,
    pub reasoning_depth: Option<String>,
    pub trust_score: Option<f64>,
    pub score: Option<f64>,
    pub observed_at: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub owner_id: Option<i64>,
    pub visibility: Option<String>,
    pub source_agent_fallback: String,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            owner_id: None,
            visibility: None,
            source_agent_fallback: "import".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImportCounts {
    pub memories: usize,
    pub decisions: usize,
}
