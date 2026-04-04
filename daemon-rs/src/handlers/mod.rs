pub mod admin;
pub mod boot;
pub mod conductor;
pub mod diary;
pub mod events;
pub mod export;
pub mod feed;
pub mod feedback;
pub mod health;
pub mod mcp;
pub mod mutate;
pub mod recall;
pub mod store;

// ─── Shared helpers ──────────────────────────────────────────────────────────

use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;
use std::net::IpAddr;

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

/// Standard cache headers applied to every JSON response.
/// CORS is handled by tower-http CorsLayer in server.rs -- do NOT set
/// Access-Control-* headers here or they will override the CORS policy.
fn apply_json_headers(headers: &mut HeaderMap) {
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
}

#[allow(clippy::result_large_err)]
/// Reject requests missing the `X-Cortex-Request: true` header.
/// Prevents SSRF attacks where a malicious website tricks the browser into
/// calling localhost:7437 -- browsers cannot add custom headers without CORS
/// preflight, and our CORS policy rejects non-localhost origins.
/// `/health` is exempt (unauthenticated monitoring endpoint).
pub fn ensure_ssrf_protection(headers: &HeaderMap) -> Result<(), Response> {
    match headers
        .get("x-cortex-request")
        .and_then(|v| v.to_str().ok())
    {
        Some("true") => Ok(()),
        _ => Err(json_response(
            StatusCode::FORBIDDEN,
            serde_json::json!({ "error": "Missing X-Cortex-Request header" }),
        )),
    }
}

#[allow(clippy::result_large_err)]
/// Validate the Bearer token on protected endpoints.  Returns `Err(Response)`
/// when the caller should short-circuit with a 401.
/// Also enforces SSRF protection (X-Cortex-Request header).
pub fn ensure_auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    ensure_ssrf_protection(headers)?;

    let header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "));

    match token {
        Some(candidate) if token_matches_state(candidate, state) => Ok(()),
        _ => Err(json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "Unauthorized" }),
        )),
    }
}

/// Auth + caller identity in one pass. Returns Ok(Some(user_id)) in team mode,
/// Ok(None) in solo mode. Err(Response) if unauthorized. Avoids double argon2.
#[allow(clippy::result_large_err)]
pub fn ensure_auth_with_caller(
    headers: &HeaderMap,
    state: &RuntimeState,
) -> Result<Option<i64>, Response> {
    ensure_ssrf_protection(headers)?;
    let header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "));
    match token {
        Some(candidate) => {
            if candidate == state.token.as_str() {
                return Ok(None);
            }
            if state.team_mode && candidate.starts_with("ctx_") {
                let hashes = state.team_api_key_hashes.read().unwrap();
                for (user_id, hash) in hashes.iter() {
                    if crate::auth::verify_api_key_argon2id(candidate, hash) {
                        return Ok(Some(*user_id));
                    }
                }
            }
            Err(json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({ "error": "Unauthorized" }),
            ))
        }
        _ => Err(json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "Unauthorized" }),
        )),
    }
}

/// Require team-mode admin/owner role. Caller must lock `state.db` first and
/// pass the connection. Returns Ok(user_id) for authorized admins, Err(Response) otherwise.
#[allow(clippy::result_large_err)]
pub fn ensure_admin(
    headers: &HeaderMap,
    state: &RuntimeState,
    conn: &rusqlite::Connection,
) -> Result<i64, Response> {
    let caller = ensure_auth_with_caller(headers, state)?;
    let user_id = match caller {
        Some(id) => id,
        None => {
            return Err(json_response(
                StatusCode::FORBIDDEN,
                serde_json::json!({ "error": "Admin endpoints require team mode" }),
            ))
        }
    };
    let role: String = conn
        .query_row(
            "SELECT role FROM users WHERE id = ?1",
            rusqlite::params![user_id],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if role != "owner" && role != "admin" {
        return Err(json_response(
            StatusCode::FORBIDDEN,
            serde_json::json!({ "error": "Insufficient permissions" }),
        ));
    }
    Ok(user_id)
}

/// Resolve which user is making this request. In solo mode returns None.
/// In team mode, iterates team API key hashes and returns the matching user_id.
/// Prefer ensure_auth_with_caller when you need both auth + caller in one pass.
#[allow(dead_code)]
pub fn resolve_caller_id(headers: &HeaderMap, state: &RuntimeState) -> Option<i64> {
    if !state.team_mode {
        return None;
    }
    let header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let token = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))?;
    if !token.starts_with("ctx_") {
        return None;
    }
    let hashes = state.team_api_key_hashes.read().unwrap();
    hashes
        .iter()
        .find(|(_, hash)| crate::auth::verify_api_key_argon2id(token, hash))
        .map(|(user_id, _)| *user_id)
}

fn token_matches_state(candidate: &str, state: &RuntimeState) -> bool {
    if candidate == state.token.as_str() {
        return true;
    }
    if !state.team_mode {
        return false;
    }
    if !candidate.starts_with("ctx_") {
        return false;
    }
    let hashes = state.team_api_key_hashes.read().unwrap();
    hashes
        .iter()
        .any(|(_, hash)| crate::auth::verify_api_key_argon2id(candidate, hash))
}

#[allow(dead_code)]
/// Extract client IP from headers (X-Forwarded-For, X-Real-IP) or default to loopback.
pub fn client_ip(headers: &HeaderMap) -> IpAddr {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse().ok())
        })
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

#[allow(dead_code)]
/// Rate-limited auth check. Returns Err(Response) on auth failure, rate limit
/// exceeded, or missing SSRF header. Handles both request-volume and
/// auth-failure buckets.
pub async fn ensure_auth_rated(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    let ip = client_ip(headers);

    // Check auth-failure block first
    if let Some(retry_after) = state.rate_limiter.is_auth_blocked(&ip).await {
        return Err(rate_limit_response(retry_after, 0));
    }

    // Check request volume
    match state.rate_limiter.check_request(ip).await {
        Err(retry_after) => return Err(rate_limit_response(retry_after, 0)),
        Ok(_remaining) => {}
    }

    // Run normal auth (SSRF + Bearer)
    match ensure_auth(headers, state) {
        Ok(()) => Ok(()),
        Err(resp) => {
            let _ = state.rate_limiter.record_auth_failure(ip).await;
            Err(resp)
        }
    }
}

#[allow(dead_code)]
fn rate_limit_response(retry_after: u64, remaining: usize) -> Response {
    let body = serde_json::json!({
        "error": "Too Many Requests",
        "retry_after": retry_after,
    });
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, Json(body)).into_response();
    let headers = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&retry_after.to_string()) {
        headers.insert("Retry-After", v);
    }
    if let Ok(v) = HeaderValue::from_str(&remaining.to_string()) {
        headers.insert("X-RateLimit-Remaining", v);
    }
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    resp
}

/// Current UTC time in ISO-8601 with millisecond precision.
pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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
