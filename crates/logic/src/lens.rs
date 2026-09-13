//! Lens profiles and the NeedFrame.
//!
//! A Lens is a self-contained read request: a profile, a query/task, typed
//! needs, target handles and a budget. The planner derives a bounded set of
//! needs conservatively: unknown intent stays unknown and is reported as an
//! unresolved need rather than fabricated into requirements.

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
            Self::Answer => vec![Need::Answer, Need::CurrentConstraints, Need::Conflicts],
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
        match raw.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
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
                Some(need) => {
                    if !needs.contains(&need) {
                        needs.push(need);
                    }
                }
                None => unresolved.push(format!("unknown need `{raw}`")),
            }
        }
        for need in profile.implied_needs() {
            if !needs.contains(&need) {
                needs.push(need);
            }
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

/// Deterministic handle extraction: paths (contain `/` and a dot or known
/// dir), `path::symbol`, ticket keys (`ABC-123`), quoted phrases, legacy refs
/// (`memory::12`, `decision::7`), and ISO date / "as of" cues.
fn insert_capped(set: &mut BTreeSet<String>, value: String) {
    if set.len() < crate::clockwork::MAX_QUERY_TOKENS {
        set.insert(value);
    }
}

pub fn extract_handles(text: &str) -> Handles {
    let text = crate::clockwork::bound_query_text(text);
    let mut handles = Handles::default();
    let mut chars = text.char_indices().peekable();
    let mut in_quote: Option<usize> = None;
    while let Some((i, c)) = chars.next() {
        if c == '"' {
            match in_quote.take() {
                Some(start) => {
                    let phrase = text[start + 1..i].trim();
                    if !phrase.is_empty() {
                        insert_capped(&mut handles.quoted, phrase.to_string());
                    }
                }
                None => in_quote = Some(i),
            }
        }
    }
    for token in text.split(|c: char| {
        c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}')
    }) {
        let token =
            token.trim_matches(|c: char| matches!(c, '.' | '"' | '\'' | '`' | ':' | '?' | '!'));
        if token.is_empty() {
            continue;
        }
        if let Some((kind, id)) = token.split_once("::") {
            if matches!(kind, "memory" | "decision")
                && id.bytes().all(|b| b.is_ascii_digit())
                && !id.is_empty()
            {
                insert_capped(&mut handles.legacy_refs, token.to_string());
                continue;
            }
            if kind.contains('/') && !id.is_empty() {
                insert_capped(&mut handles.symbols, token.to_string());
                insert_capped(&mut handles.paths, kind.to_string());
                continue;
            }
        }
        if token.contains('/') && token.len() > 2 && !token.starts_with("http") {
            insert_capped(&mut handles.paths, token.to_string());
            continue;
        }
        if is_ticket(token) {
            insert_capped(&mut handles.tickets, token.to_ascii_uppercase());
            continue;
        }
        if is_iso_date(token) {
            insert_capped(&mut handles.time_cues, token.to_string());
        }
    }
    let lower = text.to_ascii_lowercase();
    for cue in ["as of", "before", "historical", "back then", "at the time"] {
        if lower.contains(cue) {
            insert_capped(&mut handles.time_cues, cue.to_string());
        }
    }
    handles
}

fn is_ticket(token: &str) -> bool {
    let Some((prefix, number)) = token.split_once('-') else {
        return false;
    };
    prefix.len() >= 2
        && prefix.len() <= 10
        && prefix.bytes().all(|b| b.is_ascii_alphabetic())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

fn is_iso_date(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}
