// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RetentionClass {
    Durable,
    #[default]
    Operational,
    Audit,
    Ephemeral,
}

impl RetentionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Operational => "operational",
            Self::Audit => "audit",
            Self::Ephemeral => "ephemeral",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "durable" => Some(Self::Durable),
            "operational" => Some(Self::Operational),
            "audit" => Some(Self::Audit),
            "ephemeral" => Some(Self::Ephemeral),
            _ => None,
        }
    }

    pub fn default_ttl_seconds(self) -> Option<i64> {
        match self {
            Self::Durable => None,
            Self::Operational => Some(90 * 24 * 60 * 60),
            Self::Audit => Some(365 * 24 * 60 * 60),
            Self::Ephemeral => Some(14 * 24 * 60 * 60),
        }
    }

    pub fn from_entry_type(entry_type: &str) -> Option<Self> {
        match entry_type.trim().to_ascii_lowercase().as_str() {
            "decision" | "policy" | "rule" | "convention" | "contract" | "procedure"
            | "playbook" | "runbook" => Some(Self::Durable),
            "trace" | "security" | "rollback" | "permission" | "audit" => Some(Self::Audit),
            "chatter" | "scratch" | "transient" | "temporary" | "ephemeral" => {
                Some(Self::Ephemeral)
            }
            "observation" | "note" | "finding" | "fact" | "memory" | "focus_summary" => {
                Some(Self::Operational)
            }
            _ => None,
        }
    }

    pub fn classify(
        explicit: Option<Self>,
        entry_type: &str,
        text: &str,
        context: Option<&str>,
    ) -> Self {
        if let Some(explicit) = explicit {
            return explicit;
        }
        if let Some(mapped) = Self::from_entry_type(entry_type) {
            return mapped;
        }

        let combined = match context {
            Some(context) if !context.trim().is_empty() => {
                format!("{} {}", text.trim(), context.trim()).to_ascii_lowercase()
            }
            _ => text.trim().to_ascii_lowercase(),
        };
        if [
            "architectural",
            "architecture",
            "convention",
            "always",
            "never",
            "api contract",
            "must ",
            "do not",
        ]
        .iter()
        .any(|needle| combined.contains(needle))
        {
            return Self::Durable;
        }
        if ["rollback", "permission", "security event", "audit"]
            .iter()
            .any(|needle| combined.contains(needle))
        {
            return Self::Audit;
        }
        if ["throwaway", "temporary", "transient", "scratch"]
            .iter()
            .any(|needle| combined.contains(needle))
        {
            return Self::Ephemeral;
        }

        Self::Operational
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
    pub retention_class: Option<RetentionClass>,
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
    pub retention_class: Option<RetentionClass>,
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
    pub retention_class: Option<RetentionClass>,
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
