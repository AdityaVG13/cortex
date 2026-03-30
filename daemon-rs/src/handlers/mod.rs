pub mod boot;
pub mod conductor;
pub mod diary;
pub mod events;
pub mod feed;
pub mod health;
pub mod mutate;
pub mod recall;
pub mod store;

// ─── Shared helpers ──────────────────────────────────────────────────────────

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::state::RuntimeState;

/// Build an Axum JSON response with CORS / cache headers applied.
pub fn json_response(status: StatusCode, body: Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    apply_json_headers(response.headers_mut());
    response
}

/// Convenience error response.
pub fn json_error(status: StatusCode, msg: &str) -> Response {
    json_response(status, serde_json::json!({ "error": msg }))
}

/// Standard CORS + no-cache headers applied to every JSON response.
fn apply_json_headers(headers: &mut HeaderMap) {
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    headers.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
    headers.insert(
        "Access-Control-Allow-Headers",
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    headers.insert(
        "Access-Control-Allow-Methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
}

/// Validate the Bearer token on POST endpoints.  Returns `Err(Response)` when
/// the caller should short-circuit with a 401.
pub fn ensure_auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    let header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "));

    match token {
        Some(candidate) if candidate == state.token.as_str() => Ok(()),
        _ => Err(json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "Unauthorized" }),
        )),
    }
}

/// Current UTC time in ISO-8601 with millisecond precision.
pub fn now_iso() -> String {
    chrono::Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Insert an event row into the `events` table.
pub fn log_event(
    conn: &rusqlite::Connection,
    kind: &str,
    data: Value,
    source_agent: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        rusqlite::params![kind, data.to_string(), source_agent],
    )?;
    Ok(())
}

/// Estimate token count from character length (≈3.8 chars/token).
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() as f64 / 3.8).ceil() as usize
}

/// Truncate a string to at most `max` characters.
pub fn truncate_chars(input: &str, max: usize) -> String {
    input.chars().take(max).collect::<String>()
}
