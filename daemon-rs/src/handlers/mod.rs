// SPDX-License-Identifier: MIT
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
use chrono::{Duration, Utc};
use serde_json::Value;
use std::net::IpAddr;

use crate::state::RuntimeState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub agent: String,
    pub model: Option<String>,
}

const MAX_SOURCE_LABEL_LEN: usize = 160;

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
/// Reject requests missing the `X-Cortex-Request` header.
/// Prevents SSRF attacks where a malicious website tricks the browser into
/// calling localhost:7437 -- browsers cannot add custom headers without CORS
/// preflight, and our CORS policy rejects non-localhost origins.
/// `/health` is exempt (unauthenticated monitoring endpoint).
pub fn ensure_ssrf_protection(headers: &HeaderMap) -> Result<(), Response> {
    match headers
        .get("x-cortex-request")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
    {
        Some(value) if !value.is_empty() => Ok(()),
        _ => Err(json_response(
            StatusCode::FORBIDDEN,
            serde_json::json!({
                "error": "Missing X-Cortex-Request header",
                "hint": "Include header X-Cortex-Request: true on all Cortex HTTP requests"
            }),
        )),
    }
}

fn is_local_web_origin(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    value.starts_with("http://127.0.0.1")
        || value.starts_with("https://127.0.0.1")
        || value.starts_with("http://localhost")
        || value.starts_with("https://localhost")
        || value.starts_with("http://[::1]")
        || value.starts_with("https://[::1]")
        || value.starts_with("tauri://localhost")
        || value.starts_with("https://tauri.localhost")
}

fn is_local_request_context(headers: &HeaderMap) -> bool {
    let origin_ok = header_text(headers, "origin")
        .map(|value| is_local_web_origin(&value))
        .unwrap_or(true);
    let referer_ok = header_text(headers, "referer")
        .map(|value| is_local_web_origin(&value))
        .unwrap_or(true);
    origin_ok && referer_ok
}

#[allow(clippy::result_large_err)]
/// Validate the Bearer token on protected endpoints.  Returns `Err(Response)`
/// when the caller should short-circuit with a 401.
/// Also enforces SSRF protection (X-Cortex-Request header).
pub fn ensure_auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    let _candidate = match extract_auth_token(headers) {
        Some(candidate) if token_matches_state(&candidate, state) => candidate,
        _ => {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({ "error": "Unauthorized" }),
            ));
        }
    };

    if let Err(resp) = ensure_ssrf_protection(headers) {
        if !is_local_request_context(headers) {
            return Err(resp);
        }
    }

    Ok(())
}

#[allow(clippy::result_large_err)]
/// Auth + caller identity in one pass. Returns Ok(Some(user_id)) in team mode,
/// Ok(None) in solo mode. Err(Response) if unauthorized. Avoids double argon2.
pub fn ensure_auth_with_caller(
    headers: &HeaderMap,
    state: &RuntimeState,
) -> Result<Option<i64>, Response> {
    let candidate = match extract_auth_token(headers) {
        Some(candidate) => candidate,
        None => {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({ "error": "Unauthorized" }),
            ));
        }
    };

    let caller = if candidate == state.token.as_str() {
        None
    } else if state.team_mode && candidate.starts_with("ctx_") {
        let hashes = match state.team_api_key_hashes.read() {
            Ok(hashes) => hashes,
            Err(poisoned) => {
                eprintln!("[cortex] recovering poisoned team_api_key_hashes lock during auth");
                poisoned.into_inner()
            }
        };
        let mut matched = None;
        for (user_id, hash) in hashes.iter() {
            if crate::auth::verify_api_key_argon2id(&candidate, hash) {
                matched = Some(*user_id);
                break;
            }
        }
        match matched {
            Some(user_id) => Some(user_id),
            None => {
                return Err(json_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({ "error": "Unauthorized" }),
                ));
            }
        }
    } else {
        return Err(json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "Unauthorized" }),
        ));
    };

    if let Err(resp) = ensure_ssrf_protection(headers) {
        if !is_local_request_context(headers) {
            return Err(resp);
        }
    }

    Ok(caller)
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
            ));
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
    let token = extract_auth_token(headers)?;
    if !token.starts_with("ctx_") {
        return None;
    }
    let hashes = match state.team_api_key_hashes.read() {
        Ok(hashes) => hashes,
        Err(poisoned) => {
            eprintln!(
                "[cortex] recovering poisoned team_api_key_hashes lock while resolving caller"
            );
            poisoned.into_inner()
        }
    };
    hashes
        .iter()
        .find(|(_, hash)| crate::auth::verify_api_key_argon2id(&token, hash))
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
    let hashes = match state.team_api_key_hashes.read() {
        Ok(hashes) => hashes,
        Err(poisoned) => {
            eprintln!(
                "[cortex] recovering poisoned team_api_key_hashes lock while matching auth token"
            );
            poisoned.into_inner()
        }
    };
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

fn normalize_agent_label(raw_agent: &str, raw_model: Option<&str>) -> Option<String> {
    let mut agent = raw_agent.trim().to_string();
    if agent.is_empty()
        || agent.len() > MAX_SOURCE_LABEL_LEN
        || agent.chars().any(|ch| ch.is_control())
    {
        return None;
    }

    if !agent.contains('(') {
        if let Some(model) = raw_model.and_then(normalize_model_label) {
            if agent.eq_ignore_ascii_case("droid") {
                agent = format!("DROID ({model})");
            } else {
                agent = format!("{agent} ({model})");
            }
        }
    }

    if agent.len() > MAX_SOURCE_LABEL_LEN || agent.chars().any(|ch| ch.is_control()) {
        return None;
    }

    Some(agent)
}

fn normalize_model_label(raw_model: &str) -> Option<String> {
    let model = raw_model.trim();
    if model.is_empty()
        || model.len() > MAX_SOURCE_LABEL_LEN
        || model.chars().any(|ch| ch.is_control())
    {
        return None;
    }
    Some(model.to_string())
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_auth_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let without_prefix = trimmed
        .strip_prefix("Authorization:")
        .or_else(|| trimmed.strip_prefix("authorization:"))
        .map(str::trim)
        .unwrap_or(trimmed);

    without_prefix
        .strip_prefix("Bearer ")
        .or_else(|| without_prefix.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if without_prefix.contains(' ') {
                None
            } else {
                Some(without_prefix.to_string())
            }
        })
}

pub fn extract_auth_token(headers: &HeaderMap) -> Option<String> {
    header_text(headers, "authorization")
        .and_then(|raw| parse_auth_token(&raw))
        .or_else(|| header_text(headers, "x-cortex-auth").and_then(|raw| parse_auth_token(&raw)))
        .or_else(|| header_text(headers, "x-auth-header").and_then(|raw| parse_auth_token(&raw)))
}

pub fn resolve_source_identity(headers: &HeaderMap, fallback_agent: &str) -> SourceIdentity {
    let model = header_text(headers, "x-source-model").and_then(|raw| normalize_model_label(&raw));
    let fallback = fallback_agent.trim();
    let fallback = if fallback.is_empty() {
        "unknown"
    } else {
        fallback
    };
    let agent = header_text(headers, "x-source-agent")
        .and_then(|raw| normalize_agent_label(&raw, model.as_deref()))
        .or_else(|| normalize_agent_label(fallback, model.as_deref()))
        .unwrap_or_else(|| fallback.to_string());

    SourceIdentity { agent, model }
}

/// Track active agent presence in `sessions` when source headers are provided.
/// Safe best-effort helper -- failures are intentionally ignored by callers.
pub async fn register_agent_presence_from_headers(
    state: &RuntimeState,
    headers: &HeaderMap,
    caller_id: Option<i64>,
) {
    if headers.get("x-source-agent").is_none() {
        return;
    }
    let source = resolve_source_identity(headers, "mcp");

    let owner_id = if state.team_mode {
        caller_id.or(state.default_owner_id)
    } else {
        None
    };
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("session-{}", uuid::Uuid::new_v4());
    let description = source
        .model
        .as_deref()
        .map(|model| format!("Connected via MCP · {model}"))
        .unwrap_or_else(|| "Connected via MCP".to_string());

    let conn = state.db.lock().await;
    if let Some(owner_id) = owner_id {
        let _ = conn.execute(
            "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, 'mcp', '[]', ?4, ?5, ?5, ?6)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![source.agent, owner_id, session_id, description, now, expires_at],
        );
    } else {
        let _ = conn.execute(
            "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, 'mcp', '[]', ?3, ?4, ?4, ?5)
             ON CONFLICT(agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![source.agent, session_id, description, now, expires_at],
        );
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn normalize_agent_label_rejects_overflow_after_model_append() {
        let agent = "codex";
        let model = "m".repeat(MAX_SOURCE_LABEL_LEN);
        assert!(normalize_agent_label(agent, Some(&model)).is_none());
    }

    #[test]
    fn resolve_source_identity_drops_invalid_source_model() {
        let mut headers = HeaderMap::new();
        headers.insert("x-source-agent", HeaderValue::from_static("codex"));
        let invalid_model = "x".repeat(MAX_SOURCE_LABEL_LEN + 1);
        headers.insert(
            "x-source-model",
            HeaderValue::from_str(&invalid_model).expect("valid header chars"),
        );

        let source = resolve_source_identity(&headers, "mcp");
        assert_eq!(source.agent, "codex");
        assert!(source.model.is_none());
    }
}
