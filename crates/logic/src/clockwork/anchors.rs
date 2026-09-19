use super::query::QueryAnchor;
use crate::graph::{Mention, extract_mentions};
use serde::{Serialize, Serializer};
use std::collections::BTreeSet;

mod classify;
use classify::{
    classify_token, collapse_path_dots, extract_content_bigrams, extract_quoted_phrases,
    looks_like_secret, tokenize,
};

pub const MAX_ANCHORS_PER_TRACE: usize = 64;
pub const MAX_ANCHORS_PER_QUERY: usize = 32;
pub(super) const MAX_ANCHOR_CHARS: usize = 128;

const STOP_WORDS: &[&str] = &[
    "a", "about", "all", "an", "and", "any", "are", "as", "at", "be", "been", "being", "both",
    "but", "by", "can", "could", "did", "do", "does", "each", "every", "few", "for", "from", "had",
    "has", "have", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me",
    "might", "more", "most", "my", "no", "not", "of", "on", "or", "our", "shall", "should", "so",
    "some", "that", "the", "their", "then", "they", "this", "to", "was", "we", "were", "what",
    "when", "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnchorKind {
    Citation,
    Path,
    Symbol,
    Entity,
    Ticket,
    ErrorCode,
    Command,
    Flag,
    UrlHost,
    QuotedPhrase,
    Term,
    Acronym,
    Goal,
    Session,
    Source,
}

const KIND_NAMES: &[(AnchorKind, &str)] = &[
    (AnchorKind::Citation, "citation"),
    (AnchorKind::Path, "path"),
    (AnchorKind::Symbol, "symbol"),
    (AnchorKind::Entity, "entity"),
    (AnchorKind::Ticket, "ticket"),
    (AnchorKind::ErrorCode, "error_code"),
    (AnchorKind::Command, "command"),
    (AnchorKind::Flag, "flag"),
    (AnchorKind::UrlHost, "url_host"),
    (AnchorKind::QuotedPhrase, "quoted_phrase"),
    (AnchorKind::Term, "term"),
    (AnchorKind::Acronym, "acronym"),
    (AnchorKind::Goal, "goal"),
    (AnchorKind::Session, "session"),
    (AnchorKind::Source, "source"),
];

impl AnchorKind {
    pub fn as_str(self) -> &'static str {
        KIND_NAMES
            .iter()
            .find(|(kind, _)| *kind == self)
            .map(|(_, name)| *name)
            .expect("KIND_NAMES covers every AnchorKind")
    }
    pub fn parse(raw: &str) -> Option<Self> {
        KIND_NAMES
            .iter()
            .find(|(_, name)| *name == raw)
            .map(|(kind, _)| *kind)
    }
}

impl Serialize for AnchorKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Anchor {
    pub kind: AnchorKind,
    pub value: String,
    pub display_value: String,
    pub specificity: u8,
}

impl Anchor {
    pub fn new(kind: AnchorKind, display: impl Into<String>, specificity: u8) -> Option<Self> {
        let display_value = display.into();
        let value = normalize_anchor_value(kind, &display_value);
        if value.is_empty() || value.len() > MAX_ANCHOR_CHARS {
            return None;
        }
        if kind == AnchorKind::Term && is_stop_word(&value) {
            return None;
        }
        if looks_like_secret(&value) {
            return None;
        }
        Some(Self {
            kind,
            display_value: display_value.chars().take(MAX_ANCHOR_CHARS).collect(),
            value,
            specificity: specificity.min(3),
        })
    }

    pub fn is_hard(&self) -> bool {
        self.specificity >= 3
            && matches!(
                self.kind,
                AnchorKind::Citation
                    | AnchorKind::Path
                    | AnchorKind::Symbol
                    | AnchorKind::ErrorCode
                    | AnchorKind::Ticket
            )
    }
}

pub fn normalize_anchor_value(kind: AnchorKind, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match kind {
        AnchorKind::Path => collapse_path_dots(&trimmed.replace('\\', "/").to_ascii_lowercase()),
        AnchorKind::UrlHost => trimmed.trim_end_matches('/').to_ascii_lowercase(),
        _ => trimmed.to_ascii_lowercase(),
    }
}

/// Strip trailing `/**`, `/*`, and `*` glob suffixes, then trailing `/`.
pub fn strip_path_globs(mut value: String) -> String {
    loop {
        let Some(stripped) = value
            .strip_suffix("/**")
            .or_else(|| value.strip_suffix("/*"))
            .or_else(|| value.strip_suffix('*'))
        else {
            break;
        };
        value = stripped.to_string();
    }
    value.trim_end_matches('/').to_string()
}

pub fn extract_anchors(text: &str, extra: &[QueryAnchor], cap: usize) -> Vec<Anchor> {
    let mut out: BTreeSet<Anchor> = BTreeSet::new();
    let push = |out: &mut BTreeSet<Anchor>, kind: AnchorKind, display: &str, specificity: u8| {
        if let Some(anchor) = Anchor::new(kind, display, specificity) {
            if let Some(existing) = out
                .iter()
                .find(|a| a.kind == anchor.kind && a.value == anchor.value)
                .cloned()
            {
                if anchor.specificity > existing.specificity {
                    out.remove(&existing);
                    out.insert(anchor);
                }
            } else {
                out.insert(anchor);
            }
        }
    };

    for phrase in extract_quoted_phrases(text) {
        push(&mut out, AnchorKind::QuotedPhrase, &phrase, 2);
    }

    for mention in extract_mentions(text) {
        push(
            &mut out,
            kind_for_mention(&mention),
            &mention.surface,
            mention_specificity(&mention),
        );
        if !mention.qualifier.is_empty() {
            push(&mut out, AnchorKind::Term, &mention.qualifier, 1);
        }
    }

    for token in tokenize(text) {
        classify_token(&token, &mut |kind, display, spec| {
            push(&mut out, kind, display, spec)
        });
    }
    let term_values: Vec<(String, u8)> = out
        .iter()
        .filter(|a| a.kind == AnchorKind::Term && a.specificity >= 1)
        .map(|a| (a.value.clone(), a.specificity))
        .collect();
    for (value, spec) in term_values {
        for variant in super::morph::morph_variants(&value) {
            if variant != value {
                push(&mut out, AnchorKind::Term, &variant, spec.min(1));
            }
        }
    }

    extract_content_bigrams(text, &mut |display| {
        push(&mut out, AnchorKind::Term, display, 2)
    });

    for extra_anchor in extra {
        push(
            &mut out,
            extra_anchor.kind,
            &extra_anchor.value,
            extra_anchor.specificity,
        );
    }

    let mut ranked: Vec<Anchor> = out.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.specificity
            .cmp(&a.specificity)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.value.cmp(&b.value))
    });
    ranked.truncate(cap.max(1));
    ranked
}

fn kind_for_mention(mention: &Mention) -> AnchorKind {
    match mention.kind.as_str() {
        "path" => AnchorKind::Path,
        "ticket" => AnchorKind::Ticket,
        _ => AnchorKind::Entity,
    }
}

fn mention_specificity(mention: &Mention) -> u8 {
    match mention.kind.as_str() {
        "path" | "ticket" => 3,
        _ => 2,
    }
}

pub(crate) fn is_stop_word(token: &str) -> bool {
    STOP_WORDS.binary_search(&token).is_ok()
}
