use super::{AnchorKind, MAX_ANCHOR_CHARS, is_stop_word};

pub(super) fn tokenize(text: &str) -> Vec<String> {
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

pub(super) fn collapse_path_dots(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
            continue;
        }
        parts.push(part);
    }
    parts.join("/")
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

pub(super) fn classify_token(token: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) {
    if looks_like_secret(token) {
        return;
    }
    let stripped = token.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '.'));
    if stripped.is_empty() {
        return;
    }
    if classify_url(stripped, push)
        || classify_path_or_symbol(stripped, push)
        || classify_flag(stripped, push)
        || classify_ticket_or_error(stripped, push)
        || classify_citation(stripped, push)
    {
        return;
    }
    if is_acronym(stripped) {
        push(AnchorKind::Acronym, stripped, 1);
    }
    if is_rare_term(stripped) {
        let spec = if stripped.len() >= 10 || stripped.chars().any(|c: char| c.is_ascii_digit()) {
            2
        } else {
            1
        };
        push(AnchorKind::Term, stripped, spec);
    }
}

fn looks_like_path_anchor(s: &str) -> bool {
    s.contains('/') && s.len() > 3 && !crate::graph::looks_like_http_url(s)
}

fn classify_url(stripped: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) -> bool {
    if !crate::graph::looks_like_http_url(stripped) {
        return false;
    }
    if let Some(host) = url_host(stripped) {
        push(AnchorKind::UrlHost, &host, 2);
    }
    true
}

fn classify_path_or_symbol(stripped: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) -> bool {
    if let Some((left, right)) = stripped.split_once("::") {
        if looks_like_path_anchor(left) {
            push_path_anchor(left, push);
            if !right.is_empty() {
                push(AnchorKind::Symbol, right, 3);
                if is_rare_term(right) {
                    push(AnchorKind::Term, right, 2);
                }
            }
            return true;
        }
        push(AnchorKind::Symbol, stripped, 3);
        if !right.is_empty() {
            push(AnchorKind::Term, right, 1);
        }
        return true;
    }
    if looks_like_path_anchor(stripped) {
        push_path_anchor(stripped, push);
        return true;
    }
    false
}

fn classify_flag(stripped: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) -> bool {
    if stripped.starts_with("--") && stripped.len() > 3 {
        push(AnchorKind::Flag, stripped, 2);
        return true;
    }
    if stripped.len() >= 2
        && stripped.starts_with('-')
        && stripped.as_bytes()[1].is_ascii_alphabetic()
        && !stripped[1..].contains('/')
    {
        push(AnchorKind::Flag, stripped, 1);
        return true;
    }
    false
}

fn classify_ticket_or_error(stripped: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) -> bool {
    if crate::graph::is_ticket(stripped) {
        push(AnchorKind::Ticket, stripped, 3);
        return true;
    }
    if is_error_code(stripped) {
        push(AnchorKind::ErrorCode, stripped, 3);
        return true;
    }
    false
}

fn classify_citation(stripped: &str, push: &mut impl FnMut(AnchorKind, &str, u8)) -> bool {
    if stripped.starts_with("memory::") || stripped.starts_with("decision::") {
        push(AnchorKind::Citation, stripped, 3);
        return true;
    }
    false
}

pub(super) fn extract_quoted_phrases(text: &str) -> Vec<String> {
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

pub(super) fn extract_content_bigrams(text: &str, push: &mut impl FnMut(&str)) {
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
    let host = url
        .split_once("://")?
        .1
        .split(['/', '?', '#'])
        .next()?
        .trim();
    (!host.is_empty()).then(|| host.to_string())
}

fn is_error_code(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    (upper.starts_with('E') && upper.len() >= 3 && upper[1..].chars().all(|c| c.is_ascii_digit()))
        || (upper.starts_with("ERR") && upper.len() >= 4)
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

pub(super) fn looks_like_secret(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    if lower.contains("bearer")
        || lower.starts_with("sk-")
        || lower.starts_with("ghp_")
        || lower.starts_with("xox")
        || lower.contains("api_key")
    {
        return true;
    }
    token.len() >= 32
        && token.chars().filter(|c| c.is_ascii_alphanumeric()).count() * 100 / token.len() >= 90
}
