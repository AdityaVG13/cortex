//! Lens profiles and the NeedFrame.
//!
//! A Lens is a self-contained read request: a profile, a query/task, typed
//! needs, target handles and a budget. The planner derives a bounded set of
//! needs conservatively: unknown intent stays unknown and is reported as an
//! unresolved need rather than fabricated into requirements.

use crate::protocol::nonempty_opt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensProfile {
    Map,
    Orient,
    Answer,
    Changes,
    Attempts,
    Procedures,
    Conflicts,
    Uncertainty,
    Compare,
    History,
    Audit,
}

impl LensProfile {
    pub const ALL: [LensProfile; 11] = [
        Self::Map,
        Self::Orient,
        Self::Answer,
        Self::Changes,
        Self::Attempts,
        Self::Procedures,
        Self::Conflicts,
        Self::Uncertainty,
        Self::Compare,
        Self::History,
        Self::Audit,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Orient => "orient",
            Self::Answer => "answer",
            Self::Changes => "changes",
            Self::Attempts => "attempts",
            Self::Procedures => "procedures",
            Self::Conflicts => "conflicts",
            Self::Uncertainty => "uncertainty",
            Self::Compare => "compare",
            Self::History => "history",
            Self::Audit => "audit",
        }
    }
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == raw.trim().to_ascii_lowercase())
    }
    /// Needs a profile requires by construction (what it must cover to answer `ok`).
    pub fn implied_needs(self) -> Vec<Need> {
        match self {
            Self::Orient => vec![
                Need::CurrentConstraints,
                Need::OpenObligations,
                Need::FailedAttempts,
                Need::Conflicts,
            ],
            // Answer promises an answer plus applicable constraints. Conflict
            // evidence still surfaces as contested cards when present, but a
            // conflict-free answer is complete (`ok`), not `partial`: the
            // coverage model can only express "conflict cards delivered", so
            // promising conflict coverage here made `ok` unreachable on clean
            // data. Profiles conflicts/uncertainty/orient — and explicit
            // `needs: ["conflicts"]` — keep the promise.
            Self::Answer => vec![Need::Answer, Need::CurrentConstraints],
            Self::Changes => vec![Need::Changes],
            Self::Attempts => vec![Need::FailedAttempts, Need::LastVerifiedOutcome],
            Self::Procedures => vec![Need::Procedures],
            Self::Conflicts => vec![Need::Conflicts],
            Self::Uncertainty => vec![Need::Conflicts, Need::Unverified],
            Self::Compare => vec![Need::Compare],
            Self::History => vec![Need::AsKnown],
            Self::Audit => vec![Need::Audit],
            Self::Map => vec![Need::Map],
        }
    }
    /// Profiles where leads (unsupported candidates) must not appear.
    pub fn high_assurance(self) -> bool {
        matches!(self, Self::Answer | Self::Audit | Self::History)
    }
}

/// Typed needs an agent or application can declare explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    CurrentConstraints,
    OpenObligations,
    LastVerifiedOutcome,
    FailedAttempts,
    Conflicts,
    AsKnown,
    Changes,
    Procedures,
    Unverified,
    Compare,
    Audit,
    Map,
    Answer,
    Recipe { name: String },
}

impl Need {
    pub fn parse(raw: &str) -> Option<Self> {
        let value = raw.trim().to_ascii_lowercase();
        Some(match value.as_str() {
            "current_constraints" | "constraints" => Self::CurrentConstraints,
            "open_obligations" | "obligations" | "open_work" => Self::OpenObligations,
            "last_verified_outcome" | "verified" => Self::LastVerifiedOutcome,
            "failed_attempts" | "failures" | "attempts" => Self::FailedAttempts,
            "conflicts" => Self::Conflicts,
            "as_known" => Self::AsKnown,
            "changes" => Self::Changes,
            "procedures" => Self::Procedures,
            "unverified" | "uncertainty" => Self::Unverified,
            "compare" => Self::Compare,
            "audit" => Self::Audit,
            "map" => Self::Map,
            "answer" => Self::Answer,
            other => {
                let name = other.strip_prefix("recipe:")?;
                if name.is_empty() {
                    return None;
                }
                Self::Recipe {
                    name: name.to_string(),
                }
            }
        })
    }
    pub fn label(&self) -> String {
        match self {
            Self::Recipe { name } => format!("recipe:{name}"),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceDepth {
    Brief,
    Support,
    Exact,
}

impl EvidenceDepth {
    pub fn parse(raw: Option<&str>) -> Self {
        match nonempty_opt(raw).map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("support") | Some("evidence") => Self::Support,
            Some("exact") | Some("source") => Self::Exact,
            _ => Self::Brief,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brief => "brief",
            Self::Support => "support",
            Self::Exact => "exact",
        }
    }
}

/// Handles extracted from text by conservative rules only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handles {
    pub paths: BTreeSet<String>,
    pub symbols: BTreeSet<String>,
    pub tickets: BTreeSet<String>,
    pub quoted: BTreeSet<String>,
    pub legacy_refs: BTreeSet<String>,
    pub time_cues: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeedFrame {
    pub profile: LensProfile,
    pub text: String,
    pub needs: Vec<Need>,
    pub handles: Handles,
    pub evidence: EvidenceDepth,
    /// Things the request asked for that no deterministic rule can satisfy;
    /// reported back as unresolved instead of being guessed.
    pub unresolved: Vec<String>,
}

impl NeedFrame {
    /// Build a frame from the public controls. Explicit needs win; the
    /// profile's implied needs are added; unknown need words are unresolved.
    pub fn build(
        profile: LensProfile,
        text: &str,
        explicit_needs: &[String],
        evidence: EvidenceDepth,
    ) -> Self {
        let mut needs: Vec<Need> = Vec::new();
        let mut unresolved = Vec::new();
        for raw in explicit_needs
            .iter()
            .take(crate::clockwork::MAX_QUERY_TOKENS.saturating_mul(4))
        {
            if needs.len() >= crate::clockwork::MAX_QUERY_TOKENS {
                break;
            }
            match Need::parse(raw) {
                Some(need) => push_unique(&mut needs, need),
                None => unresolved.push(format!("unknown need `{raw}`")),
            }
        }
        for need in profile.implied_needs() {
            push_unique(&mut needs, need);
        }
        let text = crate::clockwork::bound_query_text(text.trim());
        let handles = extract_handles(text);
        if text.is_empty() && needs.is_empty() {
            unresolved.push("empty request".into());
        }
        Self {
            profile,
            text: text.to_string(),
            needs,
            handles,
            evidence,
            unresolved,
        }
    }
}

fn push_unique(needs: &mut Vec<Need>, need: Need) {
    if !needs.contains(&need) {
        needs.push(need);
    }
}

mod handles;
pub use handles::extract_handles;
