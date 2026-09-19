use super::anchors::{Anchor, AnchorKind};
use crate::protocol::nonempty_opt;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TemporalMode {
    Current,
    Historical,
    ExplicitAsOf,
    Any,
}

impl TemporalMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Historical => "historical",
            Self::ExplicitAsOf => "explicit_as_of",
            Self::Any => "any",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryAnchor {
    pub kind: AnchorKind,
    pub value: String,
    pub specificity: u8,
}

impl QueryAnchor {
    pub fn from_anchor(anchor: &Anchor) -> Self {
        Self {
            kind: anchor.kind,
            value: anchor.value.clone(),
            specificity: anchor.specificity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryFrame {
    pub raw: String,
    pub terms: Vec<String>,
    pub quoted_phrases: Vec<String>,
    pub anchors: Vec<QueryAnchor>,
    pub entity_ids: Vec<i64>,
    /// Entities that came in through query expansion (stems, lexicon,
    /// sibling anchors). Expansion is an access aid: these can support a
    /// vote but are never a hard anchor.
    pub expanded_entity_ids: Vec<i64>,
    pub temporal_mode: TemporalMode,
    pub as_of: Option<String>,
    pub owner_id: Option<i64>,
    pub session_id: Option<String>,
    pub goal_id: Option<i64>,
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
    pub head_id: Option<i64>,
}

impl QueryFrame {
    pub fn canonical_signature_payload(&self) -> String {
        let mut terms = self.terms.clone();
        terms.sort();
        terms.dedup();
        let mut anchors: Vec<String> = self
            .anchors
            .iter()
            .filter(|a| a.specificity >= 2)
            .map(|a| format!("{}:{}", a.kind.as_str(), a.value))
            .collect();
        anchors.sort();
        anchors.dedup();
        let mut entity_ids = self.entity_ids.clone();
        entity_ids.sort_unstable();
        entity_ids.dedup();
        let mut paths = self.paths.clone();
        paths.sort();
        let mut symbols = self.symbols.clone();
        symbols.sort();
        format!(
            "t={}|a={}|e={:?}|m={}|o={:?}|g={:?}|p={:?}|s={:?}|sym={:?}|as={:?}|h={:?}",
            terms.join(" "),
            anchors.join(","),
            entity_ids,
            self.temporal_mode.as_str(),
            self.owner_id,
            self.goal_id,
            paths,
            self.session_id,
            symbols,
            self.as_of,
            self.head_id
        )
    }
}

/// Byte cap for attacker-controlled query / need text. MCP stdin already
/// allows 2 MiB; without a tighter bound, FTS, anchor extract, and per-token
/// SQLite lookups run over the whole payload.
pub const MAX_QUERY_BYTES: usize = 8 * 1024;
/// Token / list cap for query terms, paths, symbols, and seed expansion.
pub const MAX_QUERY_TOKENS: usize = 32;

pub fn bound_query_text(raw: &str) -> &str {
    if raw.len() <= MAX_QUERY_BYTES {
        return raw;
    }
    let mut end = MAX_QUERY_BYTES;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    &raw[..end]
}

pub fn parse_query_frame(
    raw: &str,
    owner_id: Option<i64>,
    session_id: Option<String>,
    goal_id: Option<i64>,
    paths: Vec<String>,
    symbols: Vec<String>,
    as_of: Option<String>,
    head_id: Option<i64>,
) -> QueryFrame {
    let raw = bound_query_text(raw);
    let extracted = super::extract_anchors(raw, &[], super::MAX_ANCHORS_PER_QUERY);
    let mut extra = Vec::new();
    for path in paths.iter().take(MAX_QUERY_TOKENS) {
        extra.push(QueryAnchor {
            kind: AnchorKind::Path,
            value: super::normalize_anchor_value(AnchorKind::Path, path),
            specificity: 3,
        });
    }
    for symbol in symbols.iter().take(MAX_QUERY_TOKENS) {
        extra.push(QueryAnchor {
            kind: AnchorKind::Symbol,
            value: super::normalize_anchor_value(AnchorKind::Symbol, symbol),
            specificity: 3,
        });
    }
    if let Some(session) = session_id.as_deref() {
        extra.push(QueryAnchor {
            kind: AnchorKind::Session,
            value: session.to_ascii_lowercase(),
            specificity: 1,
        });
    }
    if let Some(goal) = goal_id {
        extra.push(QueryAnchor {
            kind: AnchorKind::Goal,
            value: goal.to_string(),
            specificity: 3,
        });
    }
    let mut anchors: Vec<QueryAnchor> = extracted.iter().map(QueryAnchor::from_anchor).collect();
    anchors.extend(extra);
    anchors.sort_by(|a, b| {
        b.specificity
            .cmp(&a.specificity)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.value.cmp(&b.value))
    });
    anchors.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);

    let quoted_phrases: Vec<String> = extracted
        .iter()
        .filter(|a| a.kind == AnchorKind::QuotedPhrase)
        .map(|a| a.value.clone())
        .collect();
    let terms: Vec<String> = extracted
        .iter()
        .filter(|a| a.kind == AnchorKind::Term || a.kind == AnchorKind::Acronym)
        .map(|a| a.value.clone())
        .collect();

    let (temporal_mode, inferred_as_of) = infer_temporal(raw, as_of.as_deref());
    QueryFrame {
        raw: raw.to_string(),
        terms,
        quoted_phrases,
        anchors,
        entity_ids: Vec::new(),
        expanded_entity_ids: Vec::new(),
        temporal_mode,
        as_of: inferred_as_of,
        owner_id,
        session_id,
        goal_id,
        paths: paths.into_iter().take(MAX_QUERY_TOKENS).collect(),
        symbols: symbols.into_iter().take(MAX_QUERY_TOKENS).collect(),
        head_id,
    }
}

pub fn query_signature(frame: &QueryFrame) -> String {
    crate::traces::content_hash(&frame.canonical_signature_payload())
}

fn infer_temporal(raw: &str, explicit: Option<&str>) -> (TemporalMode, Option<String>) {
    if let Some(value) = nonempty_opt(explicit) {
        return (TemporalMode::ExplicitAsOf, Some(value.to_string()));
    }
    let lower = raw.to_ascii_lowercase();
    // A bare YYYY-MM-DD is ordinary task language ("the 2024-01-15 outage",
    // a dated folder). Substring dates are not a temporal cue, matching the
    // "before"/"after" rule below: only an explicit as-of phrase (or the
    // caller-supplied timestamp) switches Current recall into as-of.
    // `contains("as of")` is a prefix of "as official"; require a phrase
    // boundary and take the date after that phrase, not the first date in
    // the whole string ("the 2023-12-01 outage as of 2024-01-15").
    if let Some(pos) = as_of_phrase_at(&lower) {
        // Phrase without a YYYY-MM-DD still must stay Current: ExplicitAsOf
        // with a missing timestamp opens archived/superseded and the history
        // arm the same way a real as-of query does.
        if let Some(date) = extract_iso_date(&lower[pos..]) {
            return (TemporalMode::ExplicitAsOf, Some(date));
        }
    }
    // Substring "before"/"after" is not a temporal cue: they fire inside
    // ordinary task language ("before merging", "after login") and would
    // switch Current recall into Historical (archived rows + history arm).
    let historical_phrase = lower.contains("previous version") || lower.contains("rolled back");
    let historical_token = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|t| matches!(t, "history" | "historical"));
    if historical_phrase || historical_token {
        return (TemporalMode::Historical, None);
    }
    if ["now", "current", "latest"]
        .iter()
        .any(|cue| lower.contains(cue))
    {
        return (TemporalMode::Current, None);
    }
    (TemporalMode::Current, None)
}

fn as_of_phrase_at(lower: &str) -> Option<usize> {
    for pat in ["as of", "as-of"] {
        let mut start = 0;
        while let Some(rel) = lower[start..].find(pat) {
            let i = start + rel;
            let before_ok = i == 0 || !lower.as_bytes()[i - 1].is_ascii_alphanumeric();
            let after = i + pat.len();
            let after_ok = after >= lower.len() || !lower.as_bytes()[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(i);
            }
            start = i + 1;
        }
    }
    None
}

fn extract_iso_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len().saturating_sub(9) {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && bytes[i + 4] == b'-'
            && bytes[i + 5].is_ascii_digit()
            && bytes[i + 6].is_ascii_digit()
            && bytes[i + 7] == b'-'
            && bytes[i + 8].is_ascii_digit()
            && bytes[i + 9].is_ascii_digit()
        {
            let month = (bytes[i + 5] - b'0') * 10 + (bytes[i + 6] - b'0');
            let day = (bytes[i + 8] - b'0') * 10 + (bytes[i + 9] - b'0');
            if (1..=12).contains(&month) && (1..=31).contains(&day) {
                return Some(text[i..i + 10].to_string());
            }
        }
    }
    None
}
