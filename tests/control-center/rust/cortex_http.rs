use super::readiness::read_auth_token_from_path;
use super::request::{validate_cortex_auth_token, validate_cortex_http_method};
use super::{
    cortex_readiness_state, decode_chunked_bytes, health_state_with_identity_fallback, is_cortex_health_response, readiness_state_with_identity_fallback,
    should_use_partial_response_on_read_timeout, validate_cortex_request_path, FetchCortexResponse,
};
use crate::daemon::paths::ResolvedCortexPaths;
use crate::daemon::shutdown::extract_error_detail;
use std::fs;
use std::path::PathBuf;

#[test]
fn decode_chunked_bytes_rejects_overflowing_chunk_size_without_panic() {
    let huge = b"ffffffffffffffff\r\nxx";
    let err = decode_chunked_bytes(huge).expect_err("oversized hex chunk size must fail closed");
    assert!(err.contains("overflow"), "{err}");

    let almost_max = b"fffffffffffffffe\r\nxx";
    let err = decode_chunked_bytes(almost_max).expect_err("near-max hex chunk size must fail closed");
    assert!(err.contains("overflow"), "{err}");

    assert_eq!(decode_chunked_bytes(b"5\r\nhello\r\n0\r\n\r\n").expect("valid chunked body"), b"hello");
}

#[test]
fn validate_cortex_request_path_rejects_absolute_urls_and_injection() {
    assert!(validate_cortex_request_path("/health").is_ok());
    assert!(validate_cortex_request_path("/sessions?agent=foo").is_ok());
    assert!(validate_cortex_request_path("http://127.0.0.1:7437/sessions").is_err());
    assert!(validate_cortex_request_path("/bad path").is_err());
    assert!(validate_cortex_request_path("/bad\r\nInjected: true").is_err());
    assert!(validate_cortex_request_path("/bad\tHost: evil").is_err());
}

#[test]
fn validate_cortex_auth_token_rejects_header_injection() {
    assert!(validate_cortex_auth_token("").is_ok());
    assert!(validate_cortex_auth_token("a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4").is_ok());
    assert!(validate_cortex_auth_token("ok\r\nX-Injected: true").is_err());
    assert!(validate_cortex_auth_token("ok\nX-Injected: true").is_err());
    assert!(validate_cortex_http_method("GET").is_ok());
    assert!(validate_cortex_http_method("POST").is_ok());
    assert!(validate_cortex_http_method("GET / HTTP/1.1\r\nX-Injected: true").is_err());
    assert!(validate_cortex_http_method("PUT").is_err());
}

#[test]
fn read_auth_token_from_path_rejects_injection_symlink_and_oversize() {
    let mut n = 0u32;
    let dir = loop {
        let path = std::env::temp_dir().join(format!(
            "cortex-cc-token-{}-{}-{n}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_nanos()
        ));
        match fs::create_dir(&path) {
            Ok(()) => break path,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                n = n.saturating_add(1);
                continue;
            }
            Err(err) => panic!("create temp dir {}: {err}", path.display()),
        }
    };
    struct Guard(std::path::PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _guard = Guard(dir.clone());

    let token_path = dir.join("cortex.token");
    fs::write(&token_path, "good-token\n").expect("write token");
    assert_eq!(read_auth_token_from_path(&token_path).expect("read token"), "good-token");

    fs::write(&token_path, "good\r\nX-Injected: 1").expect("write injected token");
    assert!(read_auth_token_from_path(&token_path).is_err());

    let oversize = dir.join("oversize.token");
    fs::write(&oversize, "x".repeat((8 * 1024) + 1)).expect("write oversize");
    assert!(read_auth_token_from_path(&oversize).is_err());

    #[cfg(unix)]
    {
        let link = dir.join("link.token");
        std::os::unix::fs::symlink(&token_path, &link).expect("symlink");
        let err = read_auth_token_from_path(&link).expect_err("symlink token must fail closed");
        assert!(err.contains("symlink"), "{err}");
    }
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
fn cortex_health_probe_rejects_ready_false_as_reachable() {
    assert!(!is_cortex_health_response(
        200,
        r#"{"status":"ok","ready":false,"runtime":{"version":"0.6.0"},"stats":{"memories":0}}"#,
        None,
        None
    ));
    assert!(is_cortex_health_response(
        200,
        r#"{"status":"ok","ready":true,"runtime":{"version":"0.6.0"},"stats":{"memories":0}}"#,
        None,
        None
    ));
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
