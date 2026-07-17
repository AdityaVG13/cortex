// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};
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
    let source = SourceIdentity { agent: "sdk-agent".to_string(), model: Some("gpt-5.4".to_string()) };
    upsert_agent_presence(&conn, &source, None, "http", "HTTP boot session").expect("upsert session");
    let (agent, project, description): (String, String, String) = conn
        .query_row("SELECT agent, project, description FROM sessions WHERE agent = ?1", rusqlite::params!["sdk-agent"], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("fetch session row");
    assert_eq!(agent, "sdk-agent");
    assert_eq!(project, "http");
    assert_eq!(description, "HTTP boot session · gpt-5.4");
}
#[test]
fn upsert_agent_presence_refreshes_existing_session_row() {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
    create_sessions_table(&conn);
    let source = SourceIdentity { agent: "sdk-agent".to_string(), model: None };
    upsert_agent_presence(&conn, &source, None, "mcp", "Connected via MCP").expect("initial upsert");
    upsert_agent_presence(&conn, &source, None, "http", "HTTP boot session").expect("refresh upsert");
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
    headers.insert("x-source-model", HeaderValue::from_str(&invalid_model).expect("valid header chars"));
    let source = resolve_source_identity(&headers, "mcp");
    assert_eq!(source.agent, "codex");
    assert!(source.model.is_none());
}
#[test]
fn ensure_ssrf_protection_requires_non_empty_header() {
    let mut headers = HeaderMap::new();
    headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:7437"));
    headers.insert("referer", HeaderValue::from_static("http://localhost:7437/settings"));
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
    headers.insert(CORTEX_PEER_IP_HEADER, HeaderValue::from_static("10.10.10.5"));
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
    headers.insert("authorization", HeaderValue::from_static("Bearer ctx_token"));
    assert_eq!(extract_auth_token(&headers).as_deref(), Some("ctx_token"));
    let mut alias_headers = HeaderMap::new();
    alias_headers.insert("x-cortex-auth", HeaderValue::from_static("Bearer ctx_token"));
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
