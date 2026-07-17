// SPDX-License-Identifier: MIT
use super::*;

use super::{
    build_owner_token, daemon_owner_signing_key_path, health_probe_base, is_cortex_health_payload, issue_owner_token_for_spawn,
    load_or_create_owner_signing_key, readiness_state_from_payload, validate_spawned_owner_claim, DAEMON_OWNER_TOKEN_TTL_SECS,
};
use crate::auth::CortexPaths;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    std::env::temp_dir().join(format!("cortex_lifecycle_{name}_{unique}"))
}
#[test]
fn cortex_health_payload_accepts_expected_shapes() {
    assert!(is_cortex_health_payload(
        200,
        r#"{"status":"ok","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#,
        Some(7437),
        None,
    ));
    assert!(is_cortex_health_payload(
        200,
        r#"{"status":"degraded","runtime":{"version":"0.5.0","port":7437},"stats":{"memories":1}}"#,
        Some(7437),
        None,
    ));
}
#[test]
fn cortex_health_payload_rejects_non_cortex_bodies() {
    assert!(!is_cortex_health_payload(200, r#"{"status":"ok"}"#, Some(7437), None));
    assert!(!is_cortex_health_payload(200, r#"{"status":"ok","runtime":{"version":"0.5.0"}}"#, Some(7437), None,));
    assert!(!is_cortex_health_payload(200, "<html>ok</html>", Some(7437), None));
    assert!(!is_cortex_health_payload(500, r#"{"status":"ok","runtime":{}}"#, Some(7437), None,));
    assert!(!is_cortex_health_payload(
        200,
        r#"{"status":"ok","runtime":{"version":"0.5.0","port":9000},"stats":{"memories":1}}"#,
        Some(7437),
        None,
    ));
}
#[test]
fn cortex_readiness_payload_reports_ready_and_starting_states() {
    let ready = serde_json::json!({
        "status": "ready",
        "ready": true,
        "runtime": { "port": 7437 },
        "stats": { "home": "C:/cortex-test/example/.cortex" }
    })
    .to_string();
    assert_eq!(readiness_state_from_payload(200, &ready, Some(7437), None), Some(true));
    let starting = serde_json::json!({
        "status": "starting",
        "ready": false,
        "runtime": { "port": 7437 },
        "stats": { "home": "C:/cortex-test/example/.cortex" }
    })
    .to_string();
    assert_eq!(readiness_state_from_payload(503, &starting, Some(7437), None), Some(false));
}
#[test]
fn cortex_readiness_payload_rejects_invalid_shapes() {
    assert_eq!(readiness_state_from_payload(200, r#"{"status":"ready"}"#, Some(7437), None), None);
    assert_eq!(
        readiness_state_from_payload(
            200,
            r#"{"status":"ready","ready":true,"runtime":{"port":9000},"stats":{"home":"C:/cortex-test/example/.cortex"}}"#,
            Some(7437),
            None
        ),
        None
    );
    assert_eq!(
        readiness_state_from_payload(
            500,
            r#"{"status":"starting","ready":false,"runtime":{"port":7437},"stats":{"home":"C:/cortex-test/example/.cortex"}}"#,
            Some(7437),
            None
        ),
        None
    );
}
#[test]
fn cortex_health_payload_rejects_identity_mismatch_for_local_expectations() {
    let home_dir = temp_test_dir("identity_mismatch");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    let valid_body = json!({
        "status": "ok",
        "stats": {
            "memories": 1,
            "home": paths.home.display().to_string()
        },
        "runtime": {
            "version": "0.5.0",
            "port": paths.port,
            "db_path": paths.db.display().to_string(),
            "token_path": paths.token.display().to_string(),
            "pid_path": paths.pid.display().to_string()
        }
    })
    .to_string();
    assert!(is_cortex_health_payload(200, &valid_body, Some(paths.port), Some(&paths),));
    let bad_token_body = json!({
        "status": "ok",
        "stats": {
            "memories": 1,
            "home": paths.home.display().to_string()
        },
        "runtime": {
            "version": "0.5.0",
            "port": paths.port,
            "db_path": paths.db.display().to_string(),
            "token_path": "C:/wrong/token",
            "pid_path": paths.pid.display().to_string()
        }
    })
    .to_string();
    assert!(!is_cortex_health_payload(200, &bad_token_body, Some(paths.port), Some(&paths),));
    let _ = std::fs::remove_dir_all(&home_dir);
}
#[test]
fn health_probe_base_formats_wildcard_and_ipv6_hosts() {
    assert_eq!(health_probe_base("", 7437), "http://127.0.0.1:7437");
    assert_eq!(health_probe_base("0.0.0.0", 7437), "http://127.0.0.1:7437");
    assert_eq!(health_probe_base("localhost", 7437), "http://localhost:7437");
    assert_eq!(health_probe_base("::", 7437), "http://127.0.0.1:7437");
    assert_eq!(health_probe_base("[::]", 7437), "http://127.0.0.1:7437");
    assert_eq!(health_probe_base("::1", 7437), "http://[::1]:7437");
    assert_eq!(health_probe_base("[::1]", 7437), "http://[::1]:7437");
}
#[test]
fn owner_token_round_trip_validates_for_spawned_owner() {
    let home_dir = temp_test_dir("owner_token_round_trip");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    let token = issue_owner_token_for_spawn(&paths, "plugin-claude", 4242).expect("issue owner token");
    validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(4242), Some(&token)).expect("validate owner token");
    let _ = std::fs::remove_dir_all(&home_dir);
}
#[cfg(unix)]
#[test]
fn owner_signing_key_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let home_dir = temp_test_dir("owner_key_permissions");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    let _ = load_or_create_owner_signing_key(&paths).expect("load owner signing key");
    let mode = std::fs::metadata(daemon_owner_signing_key_path(&paths)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let _ = std::fs::remove_dir_all(&home_dir);
}
#[test]
fn owner_token_validation_rejects_parent_or_owner_mismatch() {
    let home_dir = temp_test_dir("owner_token_mismatch");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    let token = issue_owner_token_for_spawn(&paths, "plugin-claude", 1111).expect("issue owner token");
    let wrong_parent = validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(2222), Some(&token)).unwrap_err();
    assert!(wrong_parent.contains("parent mismatch"));
    let wrong_owner = validate_spawned_owner_claim(&paths, Some("control-center"), Some(1111), Some(&token)).unwrap_err();
    assert!(wrong_owner.contains("signature mismatch"));
    let _ = std::fs::remove_dir_all(&home_dir);
}
#[test]
fn owner_token_validation_rejects_missing_or_stale_token() {
    let home_dir = temp_test_dir("owner_token_stale");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    let missing = validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(9999), None).unwrap_err();
    assert!(missing.contains("missing ownership token"));
    let key = load_or_create_owner_signing_key(&paths).expect("load owner signing key");
    let stale_issued = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
        .saturating_sub(DAEMON_OWNER_TOKEN_TTL_SECS + 10);
    let stale_token = build_owner_token(&key, "plugin-claude", 9999, stale_issued, "stale_nonce").expect("build stale token");
    let stale_error = validate_spawned_owner_claim(&paths, Some("plugin-claude"), Some(9999), Some(&stale_token)).unwrap_err();
    assert!(stale_error.contains("stale"));
    let _ = std::fs::remove_dir_all(&home_dir);
}
#[test]
fn owner_token_validation_skips_unspawned_owner_claims() {
    let home_dir = temp_test_dir("owner_token_unspawned");
    let home_str = home_dir.to_string_lossy().to_string();
    let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), Some("127.0.0.1"));
    validate_spawned_owner_claim(&paths, Some("control-center"), None, None)
        .expect("unspawned owner claims should remain backwards compatible");
    let _ = std::fs::remove_dir_all(&home_dir);
}
