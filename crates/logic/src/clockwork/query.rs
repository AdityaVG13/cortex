use super::anchors::{Anchor, AnchorKind};
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
        format!(
            "t={}|a={}|e={:?}|m={}|o={:?}|g={:?}|p={:?}|s={:?}",
            terms.join(" "),
            anchors.join(","),
            entity_ids,
            self.temporal_mode.as_str(),
            self.owner_id,
            self.goal_id,
            paths,
            self.session_id
        )
    }
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
    let extracted = super::extract_anchors(raw, &[], super::MAX_ANCHORS_PER_QUERY);
    let mut extra = Vec::new();
    for path in &paths {
        extra.push(QueryAnchor {
            kind: AnchorKind::Path,
            value: super::normalize_anchor_value(AnchorKind::Path, path),
            specificity: 3,
        });
    }
    for symbol in &symbols {
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
        temporal_mode,
        as_of: inferred_as_of,
        owner_id,
        session_id,
        goal_id,
        paths,
        symbols,
        head_id,
    }
}

pub fn query_signature(frame: &QueryFrame) -> String {
    format!(
        "{:016x}",
        fnv1a64(frame.canonical_signature_payload().as_bytes())
    )
}

fn infer_temporal(raw: &str, explicit: Option<&str>) -> (TemporalMode, Option<String>) {
    if let Some(value) = explicit.map(str::trim).filter(|v| !v.is_empty()) {
        return (TemporalMode::ExplicitAsOf, Some(value.to_string()));
    }
    let lower = raw.to_ascii_lowercase();
    if let Some(iso) = extract_iso_date(&lower) {
        return (TemporalMode::ExplicitAsOf, Some(iso));
    }
    if lower.contains("as of") || lower.contains("as-of") {
        return (TemporalMode::ExplicitAsOf, None);
    }
    if [
        "before",
        "after",
        "history",
        "historical",
        "previous version",
        "rolled back",
    ]
    .iter()
    .any(|cue| lower.contains(cue))
    {
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
            return Some(text[i..i + 10].to_string());
        }
    }
    None
}

pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
