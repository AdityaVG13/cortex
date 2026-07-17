// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_mcp_{name}_{unique}"))
    }

    #[test]
    fn persist_write_buffer_truncates_when_no_entries_remain() {
        let home_dir = temp_test_dir("write_buffer");
        fs::create_dir_all(&home_dir).unwrap();
        let buffer_path = home_dir.join("write_buffer.jsonl");
        fs::write(&buffer_path, "{\"old\":true}\n").unwrap();

        persist_write_buffer(&buffer_path, &[]).unwrap();

        assert_eq!(fs::read_to_string(&buffer_path).unwrap(), "");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn persist_write_buffer_replaces_with_remaining_entries() {
        let home_dir = temp_test_dir("write_buffer_remaining");
        fs::create_dir_all(&home_dir).unwrap();
        let buffer_path = home_dir.join("write_buffer.jsonl");
        fs::write(&buffer_path, "{\"old\":true}\n").unwrap();

        persist_write_buffer(
            &buffer_path,
            &["{\"id\":1}".to_string(), "{\"id\":2}".to_string()],
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(&buffer_path).unwrap(),
            "{\"id\":1}\n{\"id\":2}\n"
        );

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn read_auth_token_cache_can_be_invalidated() {
        let _cache_guard = auth_token_cache_test_lock().lock().unwrap();
        let home_dir = temp_test_dir("auth_token_cache");
        fs::create_dir_all(&home_dir).unwrap();
        let token_path = home_dir.join("cortex.token");

        invalidate_auth_token_cache_inner();
        fs::write(&token_path, "ctx_old").unwrap();
        assert_eq!(
            read_auth_token_with_cache_inner(&token_path),
            Some("ctx_old".to_string())
        );

        fs::write(&token_path, "ctx_new").unwrap();
        assert_eq!(
            read_auth_token_with_cache_inner(&token_path),
            Some("ctx_old".to_string())
        );

        invalidate_auth_token_cache_inner();
        assert_eq!(
            read_auth_token_with_cache_inner(&token_path),
            Some("ctx_new".to_string())
        );
        invalidate_auth_token_cache_inner();
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn detect_agent_hint_matches_known_clients() {
        assert_eq!(detect_agent_hint("Codex.exe"), Some("codex"));
        assert_eq!(detect_agent_hint("cursor-agent"), Some("cursor"));
        assert_eq!(detect_agent_hint("Gemini CLI"), Some("gemini"));
        assert_eq!(detect_agent_hint("Claude Code"), Some("claude-code"));
    }

    #[test]
    fn startup_idle_timeout_respects_env_override_and_floor() {
        let _env_lock = crate::test_env::lock();
        let _handshake_timeout =
            crate::test_env::ScopedEnvVar::remove("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS");
        assert_eq!(startup_idle_timeout().as_secs(), STARTUP_IDLE_TIMEOUT_SECS);

        std::env::set_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS", "0");
        assert_eq!(startup_idle_timeout().as_secs(), 1);

        std::env::set_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS", "75");
        assert_eq!(startup_idle_timeout().as_secs(), 75);

        std::env::remove_var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS");
    }

    #[test]
    fn is_cortex_health_response_validates_expected_port() {
        let body =
            r#"{"status":"ok","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#;
        assert!(is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "https://example.com:7437/health"
        ));
        assert!(!is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "https://example.com:9000/health"
        ));
        assert!(is_cortex_health_response(
            reqwest::StatusCode::OK,
            body,
            "invalid-url"
        ));
    }

    #[test]
    fn normalize_api_key_treats_blank_values_as_missing() {
        assert_eq!(normalize_api_key(None), None);
        assert_eq!(normalize_api_key(Some("")), None);
        assert_eq!(normalize_api_key(Some("   ")), None);
        assert_eq!(normalize_api_key(Some(" ctx_abc ")), Some("ctx_abc"));
    }

    #[test]
    fn normalize_header_value_rejects_invalid_characters() {
        assert_eq!(
            normalize_header_value("codex-cli", MAX_AGENT_HEADER_LEN),
            Some("codex-cli".to_string())
        );
        assert_eq!(
            normalize_header_value("bad\nvalue", MAX_AGENT_HEADER_LEN),
            None
        );
        assert_eq!(normalize_header_value("módèl", MAX_MODEL_HEADER_LEN), None);
    }

    #[test]
    fn custom_url_without_api_key_does_not_use_local_token_fallback() {
        let custom_base = "https://example.com";
        assert!(!is_local_daemon_base(custom_base));
        assert_eq!(build_auth_header(custom_base, None, true), None);
    }

    #[test]
    fn remote_target_requires_explicit_api_key() {
        let remote_base = "https://example.com";
        assert!(requires_explicit_api_key(remote_base, None));
        assert!(!requires_explicit_api_key(remote_base, Some("ctx_remote")));
    }

    #[test]
    fn validate_target_base_url_rejects_invalid_or_unsafe_values() {
        assert!(validate_target_base_url("https://example.com").is_ok());
        assert!(validate_target_base_url("ftp://example.com").is_err());
        assert!(validate_target_base_url("https://user:pass@example.com").is_err());
        assert!(validate_target_base_url("https://example.com?x=1").is_err());
        assert!(validate_target_base_url("not-a-url").is_err());
    }

    #[test]
    fn configured_bind_host_is_treated_as_local_for_token_fallback() {
        let _env_lock = crate::test_env::lock();
        let home_dir = temp_test_dir("configured_bind_local");
        fs::create_dir_all(&home_dir).unwrap();
        fs::write(home_dir.join("cortex.token"), "ctx_local").unwrap();

        let _home = crate::test_env::ScopedEnvVar::set("CORTEX_HOME", &home_dir);
        let _port = crate::test_env::ScopedEnvVar::set("CORTEX_PORT", "7437");
        let _bind = crate::test_env::ScopedEnvVar::set("CORTEX_BIND", "100.64.0.12");

        let local_base = "http://100.64.0.12:7437";
        assert!(is_local_daemon_base(local_base));
        assert_eq!(
            build_auth_header(local_base, None, true),
            Some("Bearer ctx_local".to_string())
        );
        assert_eq!(build_auth_header(local_base, None, false), None);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn split_base_and_path_extracts_query_path() {
        let (base, path) = split_base_and_path("https://example.com:8443/mcp-rpc?x=1")
            .expect("expected valid parsed URL");
        assert_eq!(base, "https://example.com:8443");
        assert_eq!(path, "/mcp-rpc?x=1");
    }

    #[test]
    fn split_base_and_path_rejects_invalid_urls() {
        assert!(split_base_and_path("not-a-url").is_none());
    }

    #[test]
    fn parse_http_response_parses_status_and_body() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"ok\":true}";
        let (status, body) = parse_http_response(raw).expect("expected valid parsed response");
        assert_eq!(status, reqwest::StatusCode::OK);
        assert_eq!(body, "{\"ok\":true}");
    }

    #[test]
    fn parse_http_response_rejects_missing_header_delimiter() {
        let raw = b"HTTP/1.1 200 OK\r\ncontent-type: application/json";
        assert!(parse_http_response(raw).is_err());
    }

    #[test]
    fn parse_http_response_rejects_malformed_status_line() {
        let raw = b"not-http 200 OK\r\ncontent-type: application/json\r\n\r\n{}";
        assert!(parse_http_response(raw).is_err());
    }

