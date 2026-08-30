use super::query::QueryAnchor;
use crate::graph::{extract_mentions, Mention};
use serde::{Serialize, Serializer};
use std::collections::BTreeSet;

pub const MAX_ANCHORS_PER_TRACE: usize = 64;
pub const MAX_ANCHORS_PER_QUERY: usize = 32;
const MAX_ANCHOR_CHARS: usize = 128;

const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can", "to",
    "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "that", "this",
    "it", "its", "not", "but", "and", "or", "if", "then", "so", "what", "which", "who", "how",
    "when", "where", "why", "all", "each", "every", "both", "few", "more", "most", "some", "any",
    "no", "my", "your", "his", "her", "our", "their", "i", "me", "we", "you", "they", "did",
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

impl AnchorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Citation => "citation",
            Self::Path => "path",
            Self::Symbol => "symbol",
            Self::Entity => "entity",
            Self::Ticket => "ticket",
            Self::ErrorCode => "error_code",
            Self::Command => "command",
            Self::Flag => "flag",
            Self::UrlHost => "url_host",
            Self::QuotedPhrase => "quoted_phrase",
            Self::Term => "term",
            Self::Acronym => "acronym",
            Self::Goal => "goal",
            Self::Session => "session",
            Self::Source => "source",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "citation" => Some(Self::Citation),
            "path" => Some(Self::Path),
            "symbol" => Some(Self::Symbol),
            "entity" => Some(Self::Entity),
            "ticket" => Some(Self::Ticket),
            "error_code" => Some(Self::ErrorCode),
            "command" => Some(Self::Command),
            "flag" => Some(Self::Flag),
            "url_host" => Some(Self::UrlHost),
            "quoted_phrase" => Some(Self::QuotedPhrase),
            "term" => Some(Self::Term),
            "acronym" => Some(Self::Acronym),
            "goal" => Some(Self::Goal),
            "session" => Some(Self::Session),
            "source" => Some(Self::Source),
            _ => None,
        }
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
        AnchorKind::Path => {
            let replaced = trimmed.replace('\\', "/");
            replaced.trim_matches('/').to_ascii_lowercase()
        }
        AnchorKind::UrlHost => trimmed.trim_end_matches('/').to_ascii_lowercase(),
        AnchorKind::Flag | AnchorKind::ErrorCode | AnchorKind::Ticket | AnchorKind::Citation => {
            trimmed.to_ascii_lowercase()
        }
        _ => trimmed.to_ascii_lowercase(),
    }
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

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| {
            token.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

fn push_path_anchor(path: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) {
    push(
        AnchorKind::Path,
        path,
        if path.matches('/').count() >= 1 { 3 } else { 2 },
    );
    for part in path.split('/') {
        if part.len() > 1 && !is_stop_word(&part.to_ascii_lowercase()) {
            push(AnchorKind::Term, part, 1);
        }
    }
}

fn classify_token(token: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) {
    if looks_like_secret(token) {
        return;
    }
    let stripped = token.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '.'));
    if stripped.is_empty() {
        return;
    }
    if stripped.starts_with("http://") || stripped.starts_with("https://") {
        if let Some(host) = url_host(stripped) {
            push(AnchorKind::UrlHost, &host, 2);
        }
        return;
    }
    if let Some((left, right)) = stripped.split_once("::") {
        if left.contains('/') && left.len() > 3 && !left.starts_with("http") {
            push_path_anchor(left, push);
            if !right.is_empty() {
                push(AnchorKind::Symbol, right, 3);
                if is_rare_term(right) {
                    push(AnchorKind::Term, right, 2);
                }
            }
            return;
        }
        push(AnchorKind::Symbol, stripped, 3);
        if !right.is_empty() {
            push(AnchorKind::Term, right, 1);
        }
        return;
    }
    if stripped.contains('/') && stripped.len() > 3 && !stripped.starts_with("http") {
        push_path_anchor(stripped, push);
        return;
    }
    if stripped.starts_with("--") && stripped.len() > 3 {
        push(AnchorKind::Flag, stripped, 2);
        return;
    }
    if stripped.len() >= 2
        && stripped.starts_with('-')
        && stripped.as_bytes()[1].is_ascii_alphabetic()
        && !stripped[1..].contains('/')
    {
        push(AnchorKind::Flag, stripped, 1);
        return;
    }
    if is_ticket(stripped) {
        push(AnchorKind::Ticket, stripped, 3);
        return;
    }
    if is_error_code(stripped) {
        push(AnchorKind::ErrorCode, stripped, 3);
        return;
    }
    if stripped.starts_with("memory::") || stripped.starts_with("decision::") {
        push(AnchorKind::Citation, stripped, 3);
        return;
    }
    if is_acronym(stripped) {
        push(AnchorKind::Acronym, stripped, 1);
    }
    if is_rare_term(stripped) {
        let spec = if stripped.len() >= 10 || stripped.chars().any(|c| c.is_ascii_digit()) {
            2
        } else {
            1
        };
        push(AnchorKind::Term, stripped, spec);
    }
}

fn extract_quoted_phrases(text: &str) -> Vec<String> {
    let mut phrases = Vec::new();
    let mut chars = text.char_indices();
    while let Some((start, ch)) = chars.next() {
        if ch != '"' && ch != '`' {
            continue;
        }
        let quote = ch;
        let inner_start = start + quote.len_utf8();
        for (end, next) in chars.by_ref() {
            if next == quote {
                let phrase = text[inner_start..end].trim();
                if phrase.len() >= 3 {
                    phrases.push(phrase.to_string());
                }
                break;
            }
        }
    }
    phrases
}

fn extract_content_bigrams(text: &str, push: &mut impl FnMut(&str)) {
    let tokens: Vec<String> = tokenize(text)
        .into_iter()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .to_string()
        })
        .filter(|t| t.len() > 1 && !is_stop_word(&t.to_ascii_lowercase()))
        .collect();
    for window in tokens.windows(2) {
        if window[0].len() >= 2 && window[1].len() >= 2 {
            let joined = format!("{} {}", window[0], window[1]);
            if joined.len() <= MAX_ANCHOR_CHARS {
                push(&joined);
            }
        }
    }
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn is_ticket(token: &str) -> bool {
    let mut parts = token.splitn(2, '-');
    match (parts.next(), parts.next()) {
        (Some(prefix), Some(number)) => {
            prefix.len() >= 2
                && prefix.chars().all(|c| c.is_ascii_alphabetic())
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

fn is_error_code(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    if upper.starts_with('E') && upper.len() >= 3 && upper[1..].chars().all(|c| c.is_ascii_digit())
    {
        return true;
    }
    if upper.starts_with("ERR") && upper.len() >= 4 {
        return true;
    }
    false
}

fn is_acronym(token: &str) -> bool {
    let letters: String = token.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    letters.len() >= 2 && letters.len() <= 5 && letters.chars().all(|c| c.is_ascii_uppercase())
}

fn is_rare_term(token: &str) -> bool {
    let cleaned: String = token
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if cleaned.len() < 3 || is_stop_word(&cleaned.to_ascii_lowercase()) {
        return false;
    }
    cleaned.len() >= 6
        || cleaned.chars().any(|c| c.is_ascii_digit())
        || cleaned.contains('_')
        || cleaned.chars().any(|c| c.is_ascii_uppercase())
            && cleaned.chars().any(|c| c.is_ascii_lowercase())
}

fn is_stop_word(token: &str) -> bool {
    STOP_WORDS.contains(&token)
}
fn looks_like_secret(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.contains("bearer")
        || lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("xox")
        || lower.contains("api_key")
    {
        return true;
    }
    if token.len() >= 32 {
        let alphabet = token.chars().filter(|c| c.is_ascii_alphanumeric()).count();
        if alphabet * 100 / token.len() >= 90 {
            return true;
        }
    }
    false
}
