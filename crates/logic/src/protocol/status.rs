use serde::{Deserialize, Serialize};

/// Stable response vocabulary. One status per response; HTTP codes are
/// derived from it so a transport can never invent a meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    /// Declared required needs are represented at the named frontier.
    Ok,
    /// Useful evidence supplied with explicit unmet needs.
    Partial,
    /// The declared matching plan exhausted its authorized search domain.
    NoMatch,
    /// Multiple applicable Threads, identities or interpretations remain.
    Ambiguous,
    /// The required evidence bundle does not fit; `required_plan_bytes`
    /// names a concrete safe plan size, not a proven optimum.
    NeedsMoreBudget,
    /// Required indices do not meet the requested frontier.
    ProjectionPending,
    /// Cursor, restore epoch or retained delta history is incompatible.
    ResnapshotRequired,
    /// The backend or a required source cannot be read.
    Unavailable,
    /// The caller lacks authority; protected content is not disclosed.
    Denied,
    /// A write may have committed; query/retry the same idempotency key.
    OutcomeUnknown,
    /// The request itself is malformed, unknown, or violates a declared
    /// limit. Explicit failure for unknown operations / critical flags.
    InvalidRequest,
}

impl ResponseStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Partial => "partial",
            Self::NoMatch => "no_match",
            Self::Ambiguous => "ambiguous",
            Self::NeedsMoreBudget => "needs_more_budget",
            Self::ProjectionPending => "projection_pending",
            Self::ResnapshotRequired => "resnapshot_required",
            Self::Unavailable => "unavailable",
            Self::Denied => "denied",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::InvalidRequest => "invalid_request",
        }
    }
    pub fn parse(input: &str) -> Option<Self> {
        Some(match input.trim() {
            "ok" => Self::Ok,
            "partial" => Self::Partial,
            "no_match" => Self::NoMatch,
            "ambiguous" => Self::Ambiguous,
            "needs_more_budget" => Self::NeedsMoreBudget,
            "projection_pending" => Self::ProjectionPending,
            "resnapshot_required" => Self::ResnapshotRequired,
            "unavailable" => Self::Unavailable,
            "denied" => Self::Denied,
            "outcome_unknown" => Self::OutcomeUnknown,
            "invalid_request" => Self::InvalidRequest,
            _ => return None,
        })
    }
    /// HTTP code derived from the status.
    pub fn http_code(self) -> u16 {
        match self {
            Self::Ok | Self::Partial | Self::NoMatch | Self::Ambiguous => 200,
            Self::NeedsMoreBudget => 413,
            Self::ProjectionPending => 202,
            Self::ResnapshotRequired => 409,
            Self::Unavailable => 503,
            Self::Denied => 403,
            Self::OutcomeUnknown => 202,
            Self::InvalidRequest => 400,
        }
    }
    /// Conservative mapping for legacy handlers that only know an HTTP code.
    /// Used while error sites migrate to explicit statuses.
    pub fn from_http_code(code: u16) -> Self {
        match code {
            200..=299 => Self::Ok,
            401 | 403 => Self::Denied,
            404 => Self::NoMatch,
            409 => Self::ResnapshotRequired,
            413 => Self::NeedsMoreBudget,
            429 | 502 | 503 | 504 => Self::Unavailable,
            400..=499 => Self::InvalidRequest,
            _ => Self::Unavailable,
        }
    }
    /// A response with this status is a successful answer shape (may still
    /// carry unmet needs); anything else is a failure the caller must handle.
    pub fn is_answer(self) -> bool {
        matches!(
            self,
            Self::Ok | Self::Partial | Self::NoMatch | Self::Ambiguous
        )
    }
}
