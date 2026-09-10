//! Retired: /mcp-rpc Authorization, X-Auth-Header alias, Origin and
//! X-Cortex-Request precedence and their HTTP 401/403 responses. Local callers
//! do not send HTTP headers. Private health disclosure remains explicitly gated.
use cortex_daemon::handlers::health::build_health_payload;
use cortex_tests::support::{run_with_cx, solo_state};
use serde_json::json;

#[test]
fn health_runtime_paths_remain_scoped_to_requested_home() {
    run_with_cx(|cx| async move {
        let mut state = solo_state();
        state.token_path = state.home.join("cortex.token");
        state.pid_path = state.home.join("cortex.pid");
        let public = build_health_payload(&cx, &state, false).await.unwrap();
        assert!(public["stats"].get("home").is_none());
        for key in ["token_path", "db_path", "pid_path"] {
            assert!(public["runtime"].get(key).is_none(), "{public}");
        }
        let private = build_health_payload(&cx, &state, true).await.unwrap();
        assert_eq!(private["stats"]["home"], json!(state.home.display().to_string()));
        for (key, filename) in [("token_path", "cortex.token"), ("db_path", "cortex.db"), ("pid_path", "cortex.pid")] {
            assert_eq!(private["runtime"][key], json!(state.home.join(filename).display().to_string()));
        }
        assert_eq!(public["stats"]["decisions"], private["stats"]["decisions"]);
    });
}
