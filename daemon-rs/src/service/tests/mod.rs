// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("cortex_service_{name}_{unique}"))
}
#[test]
fn build_sc_create_command_defaults_to_manual_start() {
    let cmd = build_sc_create_command(r"C:\Program Files\Cortex\cortex.exe", "alice");
    assert!(
        cmd.contains("start= demand"),
        "expected manual start mode: {cmd}"
    );
    assert!(
        !cmd.contains("start= auto"),
        "must not auto-start by default: {cmd}"
    );
}
#[test]
fn build_sc_create_command_includes_quoted_binpath_and_user() {
    let exe = r"C:\Program Files\Cortex\cortex.exe";
    let cmd = build_sc_create_command(exe, "alice");
    let expected_bin = format!("binPath= \"\\\"{}\\\" service-run\"", exe);
    assert!(cmd.contains(&format!("sc.exe create {}", SERVICE_NAME)));
    assert!(
        cmd.contains(&expected_bin),
        "missing binPath quoting: {cmd}"
    );
    assert!(
        cmd.contains("obj= \".\\alice\""),
        "missing user account object: {cmd}"
    );
}
#[test]
fn build_sc_create_command_escapes_cmd_expansion_in_exe_path() {
    let cmd = build_sc_create_command(r"C:\Tools\%PATH%\Cortex^Bin\cortex.exe", "alice");
    assert!(
        cmd.contains(r"C:\Tools\^%PATH^%\Cortex^^Bin\cortex.exe"),
        "executable path must survive cmd.exe parsing without expansion: {cmd}"
    );
}
#[test]
fn username_is_safe_for_cmd_fragment_rejects_shell_metacharacters() {
    assert!(username_is_safe_for_cmd_fragment("alice"));
    assert!(username_is_safe_for_cmd_fragment("alice.svc"));
    assert!(username_is_safe_for_cmd_fragment("alice svc"));
    assert!(!username_is_safe_for_cmd_fragment("alice&whoami"));
    assert!(!username_is_safe_for_cmd_fragment("alice|powershell"));
    assert!(!username_is_safe_for_cmd_fragment("alice%PATH%"));
    assert!(!username_is_safe_for_cmd_fragment("alice\"quoted"));
}
#[test]
fn resolve_service_username_from_env_falls_back_when_username_is_unsafe() {
    let _env_lock = crate::test_env::lock();
    let _username = crate::test_env::ScopedEnvVar::set("USERNAME", "alice&whoami");
    assert_eq!(resolve_service_username_from_env(), "cortex-user");
    std::env::set_var("USERNAME", "alice.svc");
    assert_eq!(resolve_service_username_from_env(), "alice.svc");
    std::env::remove_var("USERNAME");
    assert_eq!(resolve_service_username_from_env(), "cortex-user");
}
#[test]
fn service_exe_path_from_result_reports_resolution_failure() {
    let err = service_exe_path_from_result(Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "missing exe",
    )))
    .unwrap_err();
    assert!(
        err.contains("Failed to get exe path"),
        "expected contextual error: {err}"
    );
}
#[test]
#[cfg(windows)]
fn parse_service_state_recognizes_known_states() {
    assert_eq!(
        parse_service_state("STATE              : 4  RUNNING"),
        ServiceState::Running
    );
    assert_eq!(
        parse_service_state("STATE              : 1  STOPPED"),
        ServiceState::Stopped
    );
    assert_eq!(
        parse_service_state("STATE              : 2  START_PENDING"),
        ServiceState::StartPending
    );
    assert_eq!(
        parse_service_state("STATE              : 3  STOP_PENDING"),
        ServiceState::StopPending
    );
    assert_eq!(
        parse_service_state("STATE              : ???"),
        ServiceState::Unknown
    );
}
#[test]
#[cfg(windows)]
fn service_state_strings_are_stable() {
    assert_eq!(ServiceState::NotInstalled.as_str(), "NOT_INSTALLED");
    assert_eq!(ServiceState::Running.as_str(), "RUNNING");
    assert_eq!(ServiceState::Stopped.as_str(), "STOPPED");
    assert_eq!(ServiceState::StartPending.as_str(), "START_PENDING");
    assert_eq!(ServiceState::StopPending.as_str(), "STOP_PENDING");
    assert_eq!(ServiceState::Unknown.as_str(), "UNKNOWN");
}
#[test]
fn daemon_ready_payload_accepts_readiness_ready_and_health_ok() {
    let home_dir = temp_test_dir("ready_payload");
    let home = home_dir.to_string_lossy().to_string();
    let paths = crate::auth::CortexPaths::resolve_with_overrides(
        Some(&home),
        None,
        Some(7437),
        Some("127.0.0.1"),
    );
    let readiness = serde_json::json!({
        "status": "ready",
        "ready": true,
        "runtime": {
            "port": 7437,
            "token_path": paths.token.display().to_string(),
            "db_path": paths.db.display().to_string(),
            "pid_path": paths.pid.display().to_string(),
        },
        "stats": { "home": paths.home.display().to_string() }
    })
    .to_string();
    assert_eq!(
        daemon_ready_from_payload(200, &readiness, &paths),
        Some(true)
    );
    let health = serde_json::json!({
        "status": "ok",
        "runtime": {
            "port": 7437,
            "token_path": paths.token.display().to_string(),
            "db_path": paths.db.display().to_string(),
            "pid_path": paths.pid.display().to_string(),
        },
        "stats": { "home": paths.home.display().to_string(), "memories": 1 }
    })
    .to_string();
    assert_eq!(daemon_ready_from_payload(200, &health, &paths), Some(true));
}
#[test]
fn daemon_ready_payload_preserves_starting_state() {
    let home_dir = temp_test_dir("starting_payload");
    let home = home_dir.to_string_lossy().to_string();
    let paths = crate::auth::CortexPaths::resolve_with_overrides(
        Some(&home),
        None,
        Some(7437),
        Some("127.0.0.1"),
    );
    let readiness = serde_json::json!({
        "status": "starting",
        "ready": false,
        "runtime": {
            "port": 7437,
            "token_path": paths.token.display().to_string(),
            "db_path": paths.db.display().to_string(),
            "pid_path": paths.pid.display().to_string(),
        },
        "stats": { "home": paths.home.display().to_string() }
    })
    .to_string();
    assert_eq!(
        daemon_ready_from_payload(503, &readiness, &paths),
        Some(false)
    );
}
#[test]
fn parse_http_probe_response_extracts_status_and_body() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"ok\"}";
    let (status, body) = parse_http_probe_response(raw).expect("parse response");
    assert_eq!(status, 200);
    assert_eq!(body, "{\"status\":\"ok\"}");
}
#[test]
fn parse_http_probe_response_rejects_invalid_payloads() {
    let err = parse_http_probe_response(b"not-http").unwrap_err();
    assert!(err.contains("invalid HTTP response"));
    let err = parse_http_probe_response(b"not-http 200 OK\r\n\r\n{}").unwrap_err();
    assert!(err.contains("unsupported HTTP version"));
    let err = parse_http_probe_response(b"HTTP/1.1 099 TooLow\r\n\r\n{}").unwrap_err();
    assert!(err.contains("invalid status code"));
}
#[test]
fn partial_probe_timeout_only_applies_when_bytes_exist() {
    let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
    let would_block = std::io::Error::new(std::io::ErrorKind::WouldBlock, "would block");
    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
    assert!(should_use_partial_probe_response(&timeout, 16));
    assert!(should_use_partial_probe_response(&would_block, 16));
    assert!(!should_use_partial_probe_response(&timeout, 0));
    assert!(!should_use_partial_probe_response(&reset, 16));
}
