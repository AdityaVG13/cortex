// SPDX-License-Identifier: MIT
use super::*;
use axum::http::{HeaderMap, HeaderValue};
use serde_json::json;
#[test]
fn public_health_payload_redacts_private_runtime_paths() {
    let mut payload = json!({
        "runtime": {
            "version": "0.6.0",
            "mode": "team",
            "port": 7437,
            "db_path": "C:/Users/example/.cortex/cortex.db",
            "token_path": "C:/Users/example/.cortex/cortex.token",
            "pid_path": "C:/Users/example/.cortex/cortex.pid",
            "ipc_endpoint": r"\\.\pipe\cortex-daemon-7437",
            "ipc_kind": "named-pipe",
            "executable": "C:/Users/example/cortex.exe",
            "owner": "control-center"
        },
        "stats": {
            "home": "C:/Users/example/.cortex",
            "memories": 3
        }
    });
    redact_private_runtime_details(&mut payload);
    let runtime = payload["runtime"].as_object().unwrap();
    assert_eq!(runtime["version"], "0.6.0");
    assert!(!runtime.contains_key("db_path"));
    assert!(!payload["stats"].as_object().unwrap().contains_key("home"));
    assert_eq!(payload["stats"]["memories"], 3);
}
#[test]
fn private_runtime_details_require_cortex_header_and_loopback_peer() {
    let mut headers = HeaderMap::new();
    assert!(!include_private_runtime_details(&headers));
    headers.insert("x-cortex-request", HeaderValue::from_static("true"));
    assert!(include_private_runtime_details(&headers));
    headers.insert(
        crate::handlers::CORTEX_PEER_IP_HEADER,
        HeaderValue::from_static("203.0.113.9"),
    );
    assert!(!include_private_runtime_details(&headers));
}
