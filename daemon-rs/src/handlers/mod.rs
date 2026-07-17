pub mod admin;
pub mod auth;
pub mod boot;
pub mod conductor;
pub mod diary;
pub mod event_log;
pub mod events;
pub mod export;
pub mod feed;
pub mod feedback;
pub mod health;
pub mod mcp;
pub mod mutate;
pub mod recall;
pub mod redaction;
pub mod store;
pub use auth::{
    client_ip, ensure_admin, ensure_auth_rated, ensure_auth_with_caller_rated,
    ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget, ensure_events_stream_auth,
    ensure_ssrf_protection, log_budget_rejection, register_agent_presence,
    register_agent_presence_from_headers, resolve_caller_id, resolve_source_identity,
    runtime_token_matches, SourceIdentity, CORTEX_PEER_IP_HEADER,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{NaiveDateTime, TimeZone, Utc};
pub use event_log::log_event;
pub use redaction::redact_secrets;
use serde_json::{json, Value};
const DEFAULT_PARSED_DURATION_SECONDS: i64 = 60 * 60;
const MAX_PARSED_DURATION_SECONDS: i64 = 100 * 365 * 24 * 60 * 60;
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
pub fn json_response(status: StatusCode, body: Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    apply_json_headers(response.headers_mut());
    response
}
pub fn json_error(status: StatusCode, msg: &str) -> Response {
    json_response(status, serde_json::json!({"error":msg}))
}
fn apply_json_headers(headers: &mut HeaderMap) {
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
}
pub(crate) fn parse_duration_to_seconds(raw: &str) -> i64 {
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
pub(crate) fn parse_json_array(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}
pub(crate) fn estimate_tokens_from_chars(char_count: usize) -> usize {
    (char_count as f64 / 3.8).ceil() as usize
}
pub(crate) fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_from_chars(text.len())
}
pub(crate) fn parse_timestamp_ms(value: &str) -> i64 {
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
#[cfg(test)]
mod tests;
