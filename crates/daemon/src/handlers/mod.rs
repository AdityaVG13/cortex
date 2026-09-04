use std::cell::RefCell;
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
    client_ip, ensure_admin, ensure_auth_rated, ensure_auth_with_caller_rated, ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget,
    ensure_events_stream_auth, ensure_ssrf_protection, log_budget_rejection, register_agent_presence, register_agent_presence_from_headers, resolve_caller_id,
    resolve_source_identity, runtime_token_matches, SourceIdentity, CORTEX_PEER_IP_HEADER,
};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::{HeaderMap, HeaderValue, StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use chrono::{NaiveDateTime, TimeZone, Utc};
pub use event_log::log_event;
pub use redaction::redact_secrets;
use serde_json::{json, Value};
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
pub fn json_response(status: StatusCode, body: Value) -> Response {
    let mut response = (status, axum::Json(body)).into_response();
    apply_json_headers(response.headers_mut());
    response
}
/// JSON body extractor with daemon-contract error envelopes.
///
/// Wire contract: every daemon error response is an `application/json` body
/// with a string `error` field. axum's default `Json` extractor rejections
/// (missing/wrong Content-Type, malformed JSON, type mismatch) answer with
/// axum's rejection STATUS but a `text/plain` body, which would leak a
/// non-JSON error from every POST handler that takes a JSON body. This
/// extractor preserves each rejection's status code (400/415/422) and
/// re-wraps axum's rejection message in the standard envelope.
pub struct Json<T>(pub T);
impl<S, T> FromRequest<S> for Json<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(Json(value)),
            Err(rejection) => Err(json_response(rejection.status(), json!({"error": rejection.body_text()}))),
        }
    }
}
/// Query-string extractor with the daemon-contract error envelope (same wire
/// contract as [`Json`], which only covered body parsing). axum's default
/// `Query` extractor answers deserialization failures — unknown enum variant,
/// non-numeric integer, duplicate key — with a `text/plain` body. This
/// wrapper preserves the rejection status and re-wraps the message.
pub struct Query<T>(pub T);
impl<S, T> FromRequestParts<S> for Query<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(value)) => Ok(Query(value)),
            Err(rejection) => Err(json_response(rejection.status(), json!({"error": rejection.body_text()}))),
        }
    }
}
/// Path extractor with the daemon-contract error envelope: axum's default
/// rejects invalid percent-decoding or type mismatches with `text/plain`.
pub struct Path<T>(pub T);
impl<S, T> FromRequestParts<S> for Path<T>
where
    T: serde::de::DeserializeOwned + Send + Sync,
    S: Send + Sync,
{
    type Rejection = Response;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Path(value)) => Ok(Path(value)),
            Err(rejection) => Err(json_response(rejection.status(), json!({"error": rejection.body_text()}))),
        }
    }
}
pub fn json_error(status: StatusCode, msg: &str) -> Response {
    json_response(status, serde_json::json!({"error":msg}))
}
pub(crate) fn require_team_caller(state: &crate::state::RuntimeState, caller_id: Option<i64>) -> Result<Option<i64>, Response> {
    if !state.team_mode || caller_id.is_some() {
        return Ok(caller_id);
    }
    Err(json_response(StatusCode::FORBIDDEN, json!({"error":"Team mode requires a caller-scoped ctx_ API key"})))
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
    char_count.saturating_mul(5).div_ceil(19)
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
