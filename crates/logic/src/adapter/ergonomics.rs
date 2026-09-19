use super::CapabilityManifest;
use serde::{Deserialize, Serialize};

/// Ergonomic benchmark tally for an unfamiliar agent's first session:
/// counted events, rendered in three shapes so prose, JSON and terse
/// deliveries can be compared on the same run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErgonomicsTally {
    pub first_useful_orientation_ms: Option<u64>,
    pub schema_tokens: usize,
    pub mistaken_calls: usize,
    pub unresolved_aliases: usize,
    pub unsupported_completions: usize,
    pub expansions: usize,
    pub success: bool,
}

impl ErgonomicsTally {
    pub fn render(&self, shape: &str) -> String {
        match shape {
            "json" => serde_json::to_string(self).unwrap_or_default(),
            "terse" => format!(
                "orient={}ms schema={} mistakes={} aliases={} unsupported={} expansions={} success={}",
                self.first_useful_orientation_ms
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-".into()),
                self.schema_tokens,
                self.mistaken_calls,
                self.unresolved_aliases,
                self.unsupported_completions,
                self.expansions,
                self.success
            ),
            _ => format!(
                "First useful orientation after {}; {} schema tokens read; {} mistaken calls; {} unresolved aliases; {} unsupported completions; {} expansions; task {}.",
                self.first_useful_orientation_ms
                    .map(|v| format!("{v} ms"))
                    .unwrap_or_else(|| "never".into()),
                self.schema_tokens,
                self.mistaken_calls,
                self.unresolved_aliases,
                self.unsupported_completions,
                self.expansions,
                if self.success {
                    "succeeded"
                } else {
                    "did not succeed"
                }
            ),
        }
    }
}

/// A proposed context replacement: spans the host may swap for a bundle or
/// expansion handle. Always host-verified, never mandatory, and only ever
/// proposed to hosts that declare `replace_context_spans`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextReplacement {
    pub span_start: usize,
    pub span_end: usize,
    pub replacement: String,
    pub reason: String,
    pub bytes_saved: i64,
}

pub fn propose_replacements(
    manifest: &CapabilityManifest,
    candidates: Vec<ContextReplacement>,
) -> Vec<ContextReplacement> {
    if manifest.replace_context_spans {
        candidates
    } else {
        Vec::new()
    }
}
