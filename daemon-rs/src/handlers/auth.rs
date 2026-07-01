// SPDX-License-Identifier: MIT
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use std::net::IpAddr;

use crate::budgets::{BudgetDecision, BudgetEndpoint};
use crate::rate_limit::RequestClass;
use crate::state::RuntimeState;

use super::{json_response, now_iso};
use super::event_log::log_event;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub agent: String,
    pub model: Option<String>,
}

const MAX_SOURCE_LABEL_LEN: usize = 160;
const CTX_API_KEY_LEN: usize = 50;
pub const CORTEX_PEER_IP_HEADER: &str = "x-cortex-peer-ip";

#[allow(clippy::result_large_err)]
/// Reject requests missing the `X-Cortex-Request` header.
/// Prevents SSRF attacks where a malicious website tricks the browser into
/// calling localhost:7437 -- browsers cannot add custom headers without CORS
/// preflight, and our CORS policy rejects non-localhost origins.
/// `/health` and `/readiness` are exempt (unauthenticated monitoring endpoints).
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

#[allow(clippy::result_large_err)]
/// Validate the Bearer token on protected endpoints.  Returns `Err(Response)`
/// when the caller should short-circuit with a 401.
/// Also enforces SSRF protection (X-Cortex-Request header).
pub fn ensure_auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    ensure_ssrf_protection(headers)?;

    let _candidate = match extract_auth_token(headers) {
        Some(candidate) if token_matches_state(&candidate, state) => candidate,
        _ => {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({ "error": "Unauthorized" }),
            ));
        }
    };

    Ok(())
}

#[allow(clippy::result_large_err)]
/// Auth + caller identity in one pass. Returns Ok(Some(user_id)) in team mode,
/// Ok(None) in solo mode. Err(Response) if unauthorized. Avoids double argon2.
pub fn ensure_auth_with_caller(
    headers: &HeaderMap,
    state: &RuntimeState,
) -> Result<Option<i64>, Response> {
    ensure_ssrf_protection(headers)?;

    let candidate = match extract_auth_token(headers) {
        Some(candidate) => candidate,
        None => {
            return Err(json_response(
                StatusCode::UNAUTHORIZED,
                serde_json::json!({ "error": "Unauthorized" }),
            ));
        }
    };

    let caller = if constant_time_eq(&candidate, state.token.as_str()) {
        None
    } else if state.team_mode && is_well_formed_ctx_api_key(&candidate) {
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

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut diff = a.len() ^ b.len();
    let max_len = a.len().max(b.len());

    for idx in 0..max_len {
        let left = a.get(idx).copied().unwrap_or(0);
        let right = b.get(idx).copied().unwrap_or(0);
        diff |= usize::from(left ^ right);
    }

    diff == 0
}

fn token_matches_state(candidate: &str, state: &RuntimeState) -> bool {
    if constant_time_eq(candidate, state.token.as_str()) {
        return true;
    }
    if !state.team_mode {
        return false;
    }
    if !is_well_formed_ctx_api_key(candidate) {
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
/// Extract the server-observed peer IP stamped by trusted transport code.
/// Caller-provided forwarding headers are intentionally ignored.
pub fn client_ip(headers: &HeaderMap) -> IpAddr {
    headers
        .get(CORTEX_PEER_IP_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
}

fn should_apply_auth_failure_bucket(ip: IpAddr) -> bool {
    !ip.is_loopback()
}

#[allow(clippy::result_large_err)]
pub async fn ensure_endpoint_budget(
    headers: &HeaderMap,
    state: &RuntimeState,
    endpoint: BudgetEndpoint,
    request_source: &str,
) -> Result<(), Response> {
    let ip = client_ip(headers);
    let Some(decision) = state
        .rate_limiter
        .check_budget_for_endpoint(ip, endpoint)
        .await
    else {
        return Ok(());
    };
    if decision.allowed {
        return Ok(());
    }

    log_budget_rejection(state, &decision, request_source, &ip).await;
    Err(budget_denial_response(&decision))
}

pub async fn log_budget_rejection(
    state: &RuntimeState,
    decision: &BudgetDecision,
    request_source: &str,
    ip: &IpAddr,
) {
    let conn = state.db.lock().await;
    let _ = log_event(
        &conn,
        "budget_rejected",
        decision.event_json(request_source, &ip.to_string()),
        request_source,
    );
}

#[allow(dead_code)]
fn budget_denial_response(decision: &BudgetDecision) -> Response {
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        Json(decision.http_body_json()),
    )
        .into_response();
    let headers = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&decision.retry_after_seconds.to_string()) {
        headers.insert("Retry-After", v);
    }
    headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
    resp
}

#[allow(dead_code)]
/// Rate-limited auth check. Returns Err(Response) on auth failure, rate limit
/// exceeded, or missing SSRF header. Handles both request-volume and
/// auth-failure buckets.
pub async fn ensure_auth_rated(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
    ensure_auth_rated_for_class(headers, state, RequestClass::Default).await
}

#[allow(dead_code)]
pub async fn ensure_auth_rated_for_class(
    headers: &HeaderMap,
    state: &RuntimeState,
    class: RequestClass,
) -> Result<(), Response> {
    let ip = client_ip(headers);
    let apply_auth_failure_bucket = should_apply_auth_failure_bucket(ip);

    if apply_auth_failure_bucket {
        if let Some(retry_after) = state.rate_limiter.is_auth_blocked(&ip).await {
            return Err(rate_limit_response(retry_after, 0));
        }
    }

    match state.rate_limiter.check_request_for_class(ip, class).await {
        Err(retry_after) => return Err(rate_limit_response(retry_after, 0)),
        Ok(_remaining) => {}
    }

    match ensure_auth(headers, state) {
        Ok(()) => Ok(()),
        Err(resp) => {
            if apply_auth_failure_bucket {
                let _ = state.rate_limiter.record_auth_failure(ip).await;
            }
            Err(resp)
        }
    }
}

#[allow(dead_code)]
pub async fn ensure_auth_with_caller_rated(
    headers: &HeaderMap,
    state: &RuntimeState,
) -> Result<Option<i64>, Response> {
    ensure_auth_with_caller_rated_for_class(headers, state, RequestClass::Default).await
}

#[allow(dead_code)]
pub async fn ensure_auth_with_caller_rated_for_class(
    headers: &HeaderMap,
    state: &RuntimeState,
    class: RequestClass,
) -> Result<Option<i64>, Response> {
    let ip = client_ip(headers);
    let apply_auth_failure_bucket = should_apply_auth_failure_bucket(ip);

    if apply_auth_failure_bucket {
        if let Some(retry_after) = state.rate_limiter.is_auth_blocked(&ip).await {
            return Err(rate_limit_response(retry_after, 0));
        }
    }

    match state.rate_limiter.check_request_for_class(ip, class).await {
        Err(retry_after) => return Err(rate_limit_response(retry_after, 0)),
        Ok(_remaining) => {}
    }

    match ensure_auth_with_caller(headers, state) {
        Ok(caller) => Ok(caller),
        Err(resp) => {
            if apply_auth_failure_bucket {
                let _ = state.rate_limiter.record_auth_failure(ip).await;
            }
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
}

fn is_well_formed_ctx_api_key(candidate: &str) -> bool {
    candidate.len() == CTX_API_KEY_LEN && crate::auth::verify_ctx_api_key_checksum(candidate)
}

pub fn runtime_token_matches(candidate: &str, state: &RuntimeState) -> bool {
    constant_time_eq(candidate, state.token.as_str())
}

pub async fn ensure_events_stream_auth(
    headers: &HeaderMap,
    query_token: Option<&str>,
    state: &RuntimeState,
) -> Result<(), Response> {
    if extract_auth_token(headers).is_some() {
        return ensure_auth_rated(headers, state).await;
    }

    let provided = query_token.unwrap_or("");
    if provided.is_empty() || !token_matches_state(provided, state) {
        return Err(json_response(
            StatusCode::UNAUTHORIZED,
            serde_json::json!({ "error": "Unauthorized" }),
        ));
    }

    Ok(())
}

pub fn extract_auth_token(headers: &HeaderMap) -> Option<String> {
    header_text(headers, "authorization").and_then(|raw| parse_auth_token(&raw))
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

fn session_presence_description(source: &SourceIdentity, description_prefix: &str) -> String {
    source
        .model
        .as_deref()
        .map(|model| format!("{description_prefix} · {model}"))
        .unwrap_or_else(|| description_prefix.to_string())
}

fn upsert_agent_presence(
    conn: &rusqlite::Connection,
    source: &SourceIdentity,
    owner_id: Option<i64>,
    project: &str,
    description_prefix: &str,
) -> rusqlite::Result<()> {
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("session-{}", uuid::Uuid::new_v4());
    let description = session_presence_description(source, description_prefix);

    if let Some(owner_id) = owner_id {
        conn.execute(
            "INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?6, ?7)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![
                source.agent.as_str(),
                owner_id,
                session_id,
                project,
                description,
                now,
                expires_at
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, '[]', ?4, ?5, ?5, ?6)
             ON CONFLICT(agent) DO UPDATE SET
               description = excluded.description,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at",
            rusqlite::params![
                source.agent.as_str(),
                session_id,
                project,
                description,
                now,
                expires_at
            ],
        )?;
    }

    Ok(())
}

pub async fn register_agent_presence(
    state: &RuntimeState,
    source: &SourceIdentity,
    caller_id: Option<i64>,
    project: &str,
    description_prefix: &str,
) {
    let owner_id = if state.team_mode {
        caller_id.or(state.default_owner_id)
    } else {
        None
    };

    let conn = state.db.lock().await;
    let _ = upsert_agent_presence(&conn, source, owner_id, project, description_prefix);
}

/// Track active agent presence in `sessions` when source headers are provided.
pub async fn register_agent_presence_from_headers(
    state: &RuntimeState,
    headers: &HeaderMap,
    caller_id: Option<i64>,
) {
    if headers.get("x-source-agent").is_none() {
        return;
    }
    let source = resolve_source_identity(headers, "mcp");
    register_agent_presence(state, &source, caller_id, "mcp", "Connected via MCP").await;
}


#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn create_sessions_table(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                agent TEXT NOT NULL,
                owner_id INTEGER,
                session_id TEXT NOT NULL,
                project TEXT,
                files_json TEXT NOT NULL DEFAULT '[]',
                description TEXT,
                started_at TEXT NOT NULL,
                last_heartbeat TEXT NOT NULL,
                expires_at TEXT,
                UNIQUE(agent),
                UNIQUE(owner_id, agent)
            );",
        )
        .expect("create sessions table");
    }

    #[test]
    fn upsert_agent_presence_uses_project_and_model_aware_description() {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        create_sessions_table(&conn);

        let source = SourceIdentity {
            agent: "sdk-agent".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        upsert_agent_presence(&conn, &source, None, "http", "HTTP boot session")
            .expect("upsert session");

        let (agent, project, description): (String, String, String) = conn
            .query_row(
                "SELECT agent, project, description FROM sessions WHERE agent = ?1",
                rusqlite::params!["sdk-agent"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("fetch session row");

        assert_eq!(agent, "sdk-agent");
        assert_eq!(project, "http");
        assert_eq!(description, "HTTP boot session · gpt-5.4");
    }
    #[test]
    fn upsert_agent_presence_refreshes_existing_session_row() {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        create_sessions_table(&conn);

        let source = SourceIdentity {
            agent: "sdk-agent".to_string(),
            model: None,
        };

        upsert_agent_presence(&conn, &source, None, "mcp", "Connected via MCP")
            .expect("initial upsert");
        upsert_agent_presence(&conn, &source, None, "http", "HTTP boot session")
            .expect("refresh upsert");

        let (project, description, count): (String, String, i64) = conn
            .query_row(
                "SELECT project, description, (SELECT COUNT(*) FROM sessions WHERE agent = ?1)
                 FROM sessions
                 WHERE agent = ?1",
                rusqlite::params!["sdk-agent"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("fetch refreshed session");

        assert_eq!(project, "http");
        assert_eq!(description, "HTTP boot session");
        assert_eq!(count, 1);
    }
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

    #[test]
    fn ensure_ssrf_protection_requires_non_empty_header() {
        let mut headers = HeaderMap::new();
        headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:7437"));
        headers.insert(
            "referer",
            HeaderValue::from_static("http://localhost:7437/settings"),
        );
        assert!(ensure_ssrf_protection(&headers).is_err());

        headers.insert("x-cortex-request", HeaderValue::from_static("true"));
        assert!(ensure_ssrf_protection(&headers).is_ok());
    }

    #[test]
    fn loopback_ips_skip_auth_failure_bucket() {
        let empty_headers = HeaderMap::new();
        let fallback_ip = client_ip(&empty_headers);
        assert!(fallback_ip.is_loopback());
        assert!(!should_apply_auth_failure_bucket(fallback_ip));

        let mut headers = HeaderMap::new();
        headers.insert(CORTEX_PEER_IP_HEADER, HeaderValue::from_static("::1"));
        let ipv6_loopback = client_ip(&headers);
        assert!(ipv6_loopback.is_loopback());
        assert!(!should_apply_auth_failure_bucket(ipv6_loopback));
    }

    #[test]
    fn non_loopback_ips_apply_auth_failure_bucket() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CORTEX_PEER_IP_HEADER,
            HeaderValue::from_static("10.10.10.5"),
        );
        let ip = client_ip(&headers);
        assert!(!ip.is_loopback());
        assert!(should_apply_auth_failure_bucket(ip));
    }

    #[test]
    fn forwarded_headers_do_not_select_rate_limit_identity() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("10.10.10.5"));
        headers.insert("x-real-ip", HeaderValue::from_static("10.10.10.6"));

        let ip = client_ip(&headers);

        assert!(ip.is_loopback());
        assert!(!should_apply_auth_failure_bucket(ip));
    }

    #[test]
    fn extract_auth_token_accepts_only_standard_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer ctx_token"),
        );
        assert_eq!(extract_auth_token(&headers).as_deref(), Some("ctx_token"));

        let mut alias_headers = HeaderMap::new();
        alias_headers.insert(
            "x-cortex-auth",
            HeaderValue::from_static("Bearer ctx_token"),
        );
        assert!(extract_auth_token(&alias_headers).is_none());
    }

    #[test]
    fn constant_time_eq_matches_only_identical_strings() {
        assert!(constant_time_eq("cortex-token", "cortex-token"));
        assert!(!constant_time_eq("cortex-token", "cortex-tokeN"));
        assert!(!constant_time_eq("cortex-token", "cortex-token-extra"));
        assert!(!constant_time_eq("cortex-token-extra", "cortex-token"));
    }

    #[test]
    fn estimate_tokens_from_chars_matches_estimate_tokens() {
        for char_count in [0usize, 1, 3, 4, 38, 379, 10_000] {
            let text = "x".repeat(char_count);
            assert_eq!(
                estimate_tokens_from_chars(char_count),
                estimate_tokens(&text),
                "char-count estimator should match text estimator for {char_count} chars"
            );
        }
    }

    #[test]
    fn well_formed_ctx_api_key_shape_validation() {
        let valid = crate::auth::generate_ctx_api_key();
        assert!(is_well_formed_ctx_api_key(&valid));
        assert!(!is_well_formed_ctx_api_key("ctx_short"));
        assert!(!is_well_formed_ctx_api_key("ctx_!invalidchars"));
        assert!(!is_well_formed_ctx_api_key(&format!("ctx_{}", "A".repeat(46))));
    }
}
