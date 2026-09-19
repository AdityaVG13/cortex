//! In-process handler surface: the engines and helpers that need no HTTP.
//! The daemon re-exports this module and adds the axum adapters.

use std::cell::RefCell;
use std::collections::BTreeSet;
pub mod event_log;
pub mod feedback;
pub mod health;
pub mod mutate;
pub mod operations;
pub mod recall;
pub mod redaction;
pub mod store;
use crate::protocol::{CWD_KEYS, arg_str};
use chrono::{NaiveDateTime, TimeZone, Utc};
pub use cortex_logic::protocol::ResponseStatus;
pub use event_log::log_event;
pub use redaction::redact_secrets;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const DEFAULT_PARSED_DURATION_SECONDS: i64 = 60 * 60;
const MAX_PARSED_DURATION_SECONDS: i64 = 100 * 365 * 24 * 60 * 60;
thread_local! {
    static NOW_ISO_CACHE: RefCell<(i64, String)> = const { RefCell::new((0, String::new())) };
}
pub fn now_iso() -> String {
    let now = chrono::Utc::now();
    let ms = now.timestamp_millis();
    NOW_ISO_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.0 == ms && !cache.1.is_empty() {
            return cache.1.clone();
        }
        let formatted = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        cache.0 = ms;
        cache.1.clone_from(&formatted);
        formatted
    })
}
pub fn parse_duration_to_seconds(raw: &str) -> i64 {
    if raw.is_empty() {
        return DEFAULT_PARSED_DURATION_SECONDS;
    }
    let mut chars = raw.chars();
    let unit = chars.next_back().unwrap_or('h');
    let digits = chars.as_str();
    if digits.is_empty() {
        return DEFAULT_PARSED_DURATION_SECONDS;
    }
    let Ok(value) = digits.parse::<i64>() else {
        return DEFAULT_PARSED_DURATION_SECONDS;
    };
    if value <= 0 {
        return DEFAULT_PARSED_DURATION_SECONDS;
    }
    let multiplier = match unit {
        'm' => 60,
        'h' => 60 * 60,
        'd' => 24 * 60 * 60,
        _ => return DEFAULT_PARSED_DURATION_SECONDS,
    };
    value
        .checked_mul(multiplier)
        .filter(|seconds| *seconds <= MAX_PARSED_DURATION_SECONDS)
        .unwrap_or(DEFAULT_PARSED_DURATION_SECONDS)
}
pub fn parse_json_array(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}
pub fn estimate_tokens_from_chars(char_count: usize) -> usize {
    char_count.saturating_mul(5).div_ceil(19)
}
pub fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_from_chars(text.len())
}
pub fn parse_timestamp_ms(value: &str) -> i64 {
    if value.trim().is_empty() {
        return 0;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return dt.timestamp_millis();
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Utc.from_utc_datetime(&naive).timestamp_millis();
    }
    0
}
pub fn truncate_chars(input: &str, max: usize) -> String {
    input.chars().take(max).collect::<String>()
}

pub fn contains_ascii_ignore_case(haystack: &str, needle: &str) -> bool {
    needle.is_empty()
        || haystack
            .as_bytes()
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

pub fn starts_with_ascii_ignore_case(source: &str, prefix: &str) -> bool {
    let pb = prefix.as_bytes();
    let sb = source.as_bytes();
    sb.len() >= pb.len() && sb[..pb.len()].eq_ignore_ascii_case(pb)
}

pub fn looks_like_fs_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

pub fn cwd_root(args: &Value) -> Option<String> {
    arg_str(args, CWD_KEYS)
        .filter(|s| looks_like_fs_path(s))
        .map(str::to_string)
}

pub fn sorted_has(words: &[&str], token: &str) -> bool {
    words.binary_search(&token).is_ok()
}

/// Split on anything that is not alphanumeric and not kept by `keep`.
pub fn split_alnum_keep<'a>(
    text: &'a str,
    keep: impl Fn(char) -> bool + Copy,
) -> impl Iterator<Item = &'a str> {
    text.split(move |c: char| !(c.is_alphanumeric() || keep(c)))
        .filter(|s| !s.is_empty())
}

pub fn alnum_underscore_tokens(text: &str) -> impl Iterator<Item = &str> {
    split_alnum_keep(text, |c| c == '_')
}

pub fn alnum_underscore_lower_set(text: &str) -> BTreeSet<String> {
    alnum_underscore_tokens(text)
        .map(str::to_lowercase)
        .collect()
}

/// Operator identity matches Control Center `sessionMatchesAgent`: trim, drop a
/// trailing ` (model)` suffix, then ASCII-lowercase.
fn strip_trailing_model_suffix(raw: &str) -> &str {
    let s = raw.trim();
    if !s.ends_with(')') {
        return s;
    }
    let Some(open) = s.rfind('(') else {
        return s;
    };
    if open == 0 {
        return s;
    }
    let inner = &s[open + 1..s.len() - 1];
    if inner.is_empty() || inner.contains(')') {
        return s;
    }
    s[..open].trim()
}

pub fn agent_identity(raw: &str) -> String {
    strip_trailing_model_suffix(raw).to_ascii_lowercase()
}

pub fn same_agent(left: &str, right: &str) -> bool {
    let a = left.trim();
    let b = right.trim();
    !a.is_empty() && !b.is_empty() && agent_identity(a) == agent_identity(b)
}

/// Exact identity plus trailing ` (model)` for SQL `LIKE ? ESCAPE '\'`.
pub fn agent_match_params(agent: &str) -> Option<(String, String)> {
    let ident = agent_identity(agent);
    if ident.is_empty() {
        return None;
    }
    Some((
        ident.clone(),
        format!("{} (%", crate::db::like_escape(&ident)),
    ))
}
pub use crate::protocol::{ident_match_sql, optional_coalesce_like_sql, optional_ident_match_sql};

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
