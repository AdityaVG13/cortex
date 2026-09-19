use super::Handles;
use std::collections::BTreeSet;

fn insert_capped(set: &mut BTreeSet<String>, value: String) {
    if set.len() < crate::clockwork::MAX_QUERY_TOKENS {
        set.insert(value);
    }
}

/// Deterministic handle extraction: paths (contain `/` and a dot or known
/// dir), `path::symbol`, ticket keys (`ABC-123`), quoted phrases, legacy refs
/// (`memory::12`, `decision::7`), and ISO date / "as of" cues.
pub fn extract_handles(text: &str) -> Handles {
    let text = crate::clockwork::bound_query_text(text);
    let mut handles = Handles::default();
    let mut in_quote: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c != '"' {
            continue;
        }
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
        if token.contains('/') && token.len() > 2 && !crate::graph::looks_like_http_url(token) {
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
