use super::{
    cortex_readiness_state, health_state_with_identity_fallback, is_cortex_health_response, readiness_state_with_identity_fallback,
    should_use_partial_response_on_read_timeout, validate_cortex_request_path, FetchCortexResponse,
};
use crate::daemon::paths::ResolvedCortexPaths;
use crate::daemon::shutdown::extract_error_detail;
use std::path::PathBuf;

#[test]
fn validate_cortex_request_path_rejects_absolute_urls_and_injection() {
    assert!(validate_cortex_request_path("/health").is_ok());
    assert!(validate_cortex_request_path("/sessions?agent=foo").is_ok());
    assert!(validate_cortex_request_path("http://127.0.0.1:7437/sessions").is_err());
    assert!(validate_cortex_request_path("/bad path").is_err());
    assert!(validate_cortex_request_path("/bad\r\nInjected: true").is_err());
}

#[test]
fn partial_response_timeout_only_applies_when_bytes_exist() {
    let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
    let would_block = std::io::Error::new(std::io::ErrorKind::WouldBlock, "would block");
    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");

    assert!(should_use_partial_response_on_read_timeout(&timeout, 8));
    assert!(should_use_partial_response_on_read_timeout(&would_block, 8));
    #[cfg(windows)]
    {
        let winsock_timeout = std::io::Error::from_raw_os_error(10060);
        assert!(should_use_partial_response_on_read_timeout(&winsock_timeout, 8));
    }
    assert!(!should_use_partial_response_on_read_timeout(&timeout, 0));
    assert!(!should_use_partial_response_on_read_timeout(&reset, 8));
}

#[test]
fn extract_error_detail_prefers_json_error_field() {
    let detail = extract_error_detail("{\"error\":\"Unauthorized\"}").unwrap();
    assert_eq!(detail, "Unauthorized");
}

#[test]
fn cortex_health_probe_accepts_healthy_response_shape() {
    assert!(is_cortex_health_response(200, r#"{"status":"ok","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#, None, None));
    assert!(is_cortex_health_response(200, r#"{"status":"degraded","runtime":{"version":"0.5.0"},"stats":{"memories":1}}"#, None, None));
}

#[test]
fn cortex_health_probe_rejects_non_cortex_responses() {
    assert!(!is_cortex_health_response(200, "<html>ok</html>", None, None));
    assert!(!is_cortex_health_response(200, r#"{"status":"ok"}"#, None, None));
    assert!(!is_cortex_health_response(200, r#"{"status":"ok","runtime":{"version":"0.5.0"}}"#, None, None));
    assert!(!is_cortex_health_response(503, r#"{"status":"ok","runtime":{}}"#, None, None));
}

#[test]
fn cortex_readiness_probe_accepts_ready_and_starting_payloads() {
    assert_eq!(
        cortex_readiness_state(
            200,
            r#"{"status":"ready","ready":true,"runtime":{"port":7437},"stats":{"home":"C:/cortex-test/testuser/.cortex"}}"#,
            Some(7437),
            None
        ),
        Some(true)
    );
    assert_eq!(
        cortex_readiness_state(
            503,
            r#"{"status":"starting","ready":false,"runtime":{"port":7437},"stats":{"home":"C:/cortex-test/testuser/.cortex"}}"#,
            Some(7437),
            None
        ),
        Some(false)
    );
}

#[test]
fn cortex_readiness_probe_rejects_invalid_payloads() {
    assert_eq!(
        cortex_readiness_state(200, r#"{"status":"ready","runtime":{"port":7437},"stats":{"home":"C:/cortex-test/testuser/.cortex"}}"#, Some(7437), None),
        None
    );
    assert_eq!(
        cortex_readiness_state(
            500,
            r#"{"status":"starting","ready":false,"runtime":{"port":7437},"stats":{"home":"C:/cortex-test/testuser/.cortex"}}"#,
            Some(7437),
            None
        ),
        None
    );
}

#[test]
fn cortex_health_probe_rejects_identity_mismatch() {
    let expected = ResolvedCortexPaths {
        home: Some(PathBuf::from("C:/cortex-test/testuser/.cortex")),
        token: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.token")),
        db: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.db")),
        pid: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.pid")),
        port: Some(7437),
        bind: Some("127.0.0.1".to_string()),
    };
    assert!(!is_cortex_health_response(
        200,
        r#"{"status":"ok","runtime":{"port":7437,"token_path":"C:/other/cortex.token","db_path":"C:/cortex-test/testuser/.cortex/cortex.db","pid_path":"C:/cortex-test/testuser/.cortex/cortex.pid"},"stats":{"home":"C:/cortex-test/testuser/.cortex","memories":1}}"#,
        Some(7437),
        Some(&expected)
    ));
    assert!(is_cortex_health_response(
        200,
        r#"{"status":"ok","runtime":{"port":7437,"token_path":"C:/cortex-test/testuser/.cortex/cortex.token","db_path":"C:/cortex-test/testuser/.cortex/cortex.db","pid_path":"C:/cortex-test/testuser/.cortex/cortex.pid"},"stats":{"home":"C:/cortex-test/testuser/.cortex","memories":1}}"#,
        Some(7437),
        Some(&expected)
    ));
}

#[test]
fn readiness_identity_fallback_classifies_starting_payload_on_path_mismatch() {
    let expected = ResolvedCortexPaths {
        home: Some(PathBuf::from("C:/cortex-test/testuser/.cortex")),
        token: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.token")),
        db: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.db")),
        pid: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.pid")),
        port: Some(7437),
        bind: Some("127.0.0.1".to_string()),
    };
    let (state, mismatch) = readiness_state_with_identity_fallback(
        503,
        r#"{"status":"starting","ready":false,"runtime":{"port":7437,"token_path":"C:/other/cortex.token","db_path":"C:/other/cortex.db","pid_path":"C:/other/cortex.pid"},"stats":{"home":"C:/other","memories":1}}"#,
        Some(7437),
        Some(&expected),
    );
    assert_eq!(state, Some(false));
    assert!(mismatch);
}

#[test]
fn health_identity_fallback_detects_reachable_payload_on_path_mismatch() {
    let expected = ResolvedCortexPaths {
        home: Some(PathBuf::from("C:/cortex-test/testuser/.cortex")),
        token: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.token")),
        db: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.db")),
        pid: Some(PathBuf::from("C:/cortex-test/testuser/.cortex/cortex.pid")),
        port: Some(7437),
        bind: Some("127.0.0.1".to_string()),
    };
    let (healthy, mismatch) = health_state_with_identity_fallback(
        200,
        r#"{"status":"ok","runtime":{"port":7437,"token_path":"C:/other/cortex.token","db_path":"C:/other/cortex.db","pid_path":"C:/other/cortex.pid"},"stats":{"home":"C:/other","memories":1}}"#,
        Some(7437),
        Some(&expected),
    );
    assert!(healthy);
    assert!(mismatch);
}
