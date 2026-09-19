use super::ids::{Frontier, LogicalId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The eight semantic operations plus operator capabilities.
pub const KNOWN_OPERATIONS: &[&str] = &[
    "capabilities",
    "orient",
    "query",
    "expand",
    "commit",
    "checkpoint",
    "resolve",
    "feedback",
];

/// Trusted adapter input. Never populated from a model-authored field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipalContext {
    pub authenticated_id: String,
    pub policy_epoch: String,
    /// Transport that established the identity (bearer, api_key, local_uid…).
    pub established_by: String,
}

/// Typed narrowing selector. `owner_id` is a *selector* validated against the
/// principal, not a credential.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<LogicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub output_bytes: u64,
    pub candidate_rows: u32,
    pub graph_depth: u16,
    pub work_units: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            output_bytes: 4096,
            candidate_rows: 96,
            graph_depth: 2,
            work_units: 1000,
        }
    }
}

/// Host-attested statement of what the *current* invocation already holds.
/// Absent or unknown presence means: deliver self-contained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPresence {
    pub context_epoch: String,
    pub invocation: String,
    /// Exact (record revision, representation version) pairs present.
    #[serde(default)]
    pub present: Vec<PresentRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentRepresentation {
    pub revision: LogicalId,
    pub representation: String,
}

/// Cross-cutting request envelope. Public model inputs are normally only the
/// body (task/need, thread, budget, evidence depth); adapters fill the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub protocol_version: String,
    pub request_id: String,
    pub brain_id: String,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_context: Option<PrincipalContext>,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_frontier: Option<Frontier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_commit_receipt: Option<LogicalId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_presence: Option<ContextPresence>,
    #[serde(default)]
    pub limits: Limits,
    /// Flags the sender declares the receiver MUST understand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_flags: Vec<String>,
    #[serde(default)]
    pub body: Value,
    /// Unknown *namespaced* fields (`x-…` / `ext.…`) round-trip untouched.
    #[serde(flatten)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvelopeError {
    UnknownOperation(String),
    UnknownRequiredFlag(String),
    UnknownField(String),
    MissingField(&'static str),
}

impl std::fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOperation(op) => write!(
                f,
                "unknown operation `{op}`; known: {}",
                KNOWN_OPERATIONS.join(", ")
            ),
            Self::UnknownRequiredFlag(flag) => {
                write!(f, "required flag `{flag}` is not understood")
            }
            Self::UnknownField(name) => write!(
                f,
                "unknown non-namespaced field `{name}`; prefix extensions with `x-`"
            ),
            Self::MissingField(name) => write!(f, "missing required field `{name}`"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

/// Flags every runtime understands. Anything else in `required_flags` fails.
pub const KNOWN_FLAGS: &[&str] = &["read_your_writes", "exact_source", "no_leads"];

impl Envelope {
    pub fn is_extension_name(name: &str) -> bool {
        name.starts_with("x-") || name.starts_with("ext.")
    }

    /// Permissive parsing never means permissive policy: unknown operations,
    /// unknown required flags and unknown *non-namespaced* fields fail with
    /// the field to fix.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        for (field, value) in [
            ("protocol_version", self.protocol_version.as_str()),
            ("request_id", self.request_id.as_str()),
            ("brain_id", self.brain_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(EnvelopeError::MissingField(field));
            }
        }
        if !KNOWN_OPERATIONS.contains(&self.operation.as_str()) {
            return Err(EnvelopeError::UnknownOperation(self.operation.clone()));
        }
        for flag in &self.required_flags {
            if !KNOWN_FLAGS.contains(&flag.as_str()) {
                return Err(EnvelopeError::UnknownRequiredFlag(flag.clone()));
            }
        }
        if let Some(name) = self.extensions.keys().find(|k| !Self::is_extension_name(k)) {
            return Err(EnvelopeError::UnknownField(name.clone()));
        }
        Ok(())
    }
}
