use super::paths::{
    is_disallowed_daemon_binary_path, path_binary_fallback_enabled_from_value,
    workspace_binary_candidates,
};
use super::shutdown::{configure_shutdown_flush_connection, extract_error_detail, interpret_shutdown_response};
use super::spawn::{local_app_managed_start_timeout_message, local_probe_allows_starting_retry};
use super::state::{describe_daemon_state, DaemonState, LifecycleState};
use crate::constants::SQLITE_BUSY_TIMEOUT_MS;
use crate::cortex_http::{CortexReachabilityProbe, FetchCortexResponse};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

fn normalized_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn path_contains(path: &Path, needle: &str) -> bool {
    normalized_path_string(path).contains(needle)
}

fn cortex_binary_file_name() -> &'static str {
    if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    }
}


    fn spawn_test_sleep_process() -> Child {
        #[cfg(windows)]
        {
            Command::new("cmd")
                .args(["/C", "ping -n 30 127.0.0.1 > NUL"])
                .spawn()
                .expect("spawn windows sleep surrogate")
        }

        #[cfg(not(windows))]
        {
            Command::new("sleep")
                .arg("30")
                .spawn()
                .expect("spawn unix sleep")
        }
    }


    #[test]
    fn shutdown_flush_sets_busy_timeout_before_checkpoint() {
        let conn = Connection::open_in_memory().expect("open sqlite");
        configure_shutdown_flush_connection(&conn).expect("configure shutdown flush");
        let busy_timeout_ms: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .expect("read busy_timeout");
        assert_eq!(busy_timeout_ms, SQLITE_BUSY_TIMEOUT_MS as i64);
    }


    #[test]
    fn workspace_binary_candidates_prefers_debug_for_dev_builds() {
        let candidates = workspace_binary_candidates(Path::new("C:/cortex-test/testuser"), true);
        assert_eq!(candidates.len(), 3);
        assert!(path_contains(&candidates[0], "target-control-center-dev/debug"));
        assert!(path_contains(
            &candidates[1],
            "target-control-center-release/release",
        ));
        assert!(path_contains(&candidates[2], "target/release"));
        assert!(candidates
            .iter()
            .all(|path| !path_contains(path, "target/debug")));
    }


    #[test]
    fn workspace_binary_candidates_prefers_release_for_packaged_builds() {
        let candidates = workspace_binary_candidates(Path::new("C:/cortex-test/testuser"), false);
        assert_eq!(candidates.len(), 3);
        assert!(path_contains(
            &candidates[0],
            "target-control-center-release/release",
        ));
        assert!(path_contains(&candidates[1], "target/release"));
        assert!(path_contains(&candidates[2], "target-control-center-dev/debug"));
        assert!(candidates
            .iter()
            .all(|path| !path_contains(path, "target/debug")));
    }


    #[test]
    fn path_binary_fallback_requires_explicit_truthy_env_value() {
        assert!(!path_binary_fallback_enabled_from_value(None));
        assert!(!path_binary_fallback_enabled_from_value(Some("")));
        assert!(!path_binary_fallback_enabled_from_value(Some("0")));
        assert!(!path_binary_fallback_enabled_from_value(Some("false")));
        assert!(path_binary_fallback_enabled_from_value(Some("1")));
        assert!(path_binary_fallback_enabled_from_value(Some("true")));
        assert!(path_binary_fallback_enabled_from_value(Some("Yes")));
        assert!(path_binary_fallback_enabled_from_value(Some("on")));
    }


    #[test]
    fn disallowed_daemon_binary_path_blocks_wrappers_temp_and_test_artifacts() {
        let wrapper = PathBuf::from(
            "C:/repo/daemon-rs/target/debug/daemon-lifecycle-runtime/cortex-daemon-run.exe",
        );
        assert!(is_disallowed_daemon_binary_path(&wrapper));

        let wrapper_name_only = PathBuf::from("C:/repo/cortex-daemon-run");
        assert!(is_disallowed_daemon_binary_path(&wrapper_name_only));

        let temp_candidate = std::env::temp_dir().join("cortex").join("cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&temp_candidate));

        let target_tests = PathBuf::from("C:/repo/daemon-rs/target-tests/debug/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&target_tests));

        let target_test = PathBuf::from("C:/repo/daemon-rs/target-test/release/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&target_test));

        let nextest = PathBuf::from("C:/repo/daemon-rs/target/nextest/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&nextest));

        let target_deps = PathBuf::from("C:/repo/daemon-rs/target/debug/deps/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&target_deps));

        let isolated_target_deps =
            PathBuf::from("C:/repo/daemon-rs/target-control-center-dev/debug/deps/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&isolated_target_deps));

        let shared_workspace_runtime = PathBuf::from(format!(
            "C:/repo/daemon-rs/target/debug/{}",
            cortex_binary_file_name()
        ));
        assert!(is_disallowed_daemon_binary_path(&shared_workspace_runtime));

        let isolated_runtime = PathBuf::from(format!(
            "C:/repo/daemon-rs/target-control-center-dev/debug/{}",
            cortex_binary_file_name()
        ));
        assert!(!is_disallowed_daemon_binary_path(&isolated_runtime));

        let isolated_release_runtime = PathBuf::from(format!(
            "C:/repo/daemon-rs/target-control-center-release/release/{}",
            cortex_binary_file_name()
        ));
        assert!(!is_disallowed_daemon_binary_path(&isolated_release_runtime));

        let rtk_isolated = PathBuf::from("C:/repo/daemon-rs/target-rtk-isolated/debug/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&rtk_isolated));

        let codex_test = PathBuf::from("C:/repo/daemon-rs/target-codex-test/debug/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&codex_test));

        let target_build_script = PathBuf::from("C:/repo/daemon-rs/target/debug/build/cortex.exe");
        assert!(is_disallowed_daemon_binary_path(&target_build_script));

        let safe = PathBuf::from("C:/cortex-test/testuser/.cortex/bin/cortex.exe");
        assert!(!is_disallowed_daemon_binary_path(&safe));
    }


    #[test]
    fn local_start_timeout_cleanup_clears_managed_child_state() {
        let child = spawn_test_sleep_process();
        let pid = child.id();
        let state = DaemonState {
            exe_path: None,
            child: Mutex::new(Some(child)),
            intentional_stop: AtomicBool::new(false),
        };

        let (managed_before, _) = state.status().expect("initial status");
        assert!(managed_before);

        let message = local_app_managed_start_timeout_message(&state, Some(pid), 7437);
        assert!(message.contains("cleared the stale app-managed startup state"));

        let (managed_after, pid_after) = state.status().expect("post-cleanup status");
        assert!(!managed_after);
        assert_eq!(pid_after, None);
    }


    #[test]
    fn daemon_state_description_includes_starting_state() {
        let managed_message = describe_daemon_state(true, false, true, false, Some(42), 7437);
        assert!(managed_message.contains("still starting"));

        let external_message = describe_daemon_state(false, false, true, false, None, 7437);
        assert!(external_message.contains("still starting"));
    }


    #[test]
    fn local_probe_retry_requires_reachability_or_starting_signal() {
        assert!(local_probe_allows_starting_retry(
            &CortexReachabilityProbe {
                reachable: true,
                starting: false,
                identity_mismatch: false,
            }
        ));
        assert!(local_probe_allows_starting_retry(
            &CortexReachabilityProbe {
                reachable: false,
                starting: true,
                identity_mismatch: false,
            }
        ));
        assert!(!local_probe_allows_starting_retry(
            &CortexReachabilityProbe {
                reachable: false,
                starting: false,
                identity_mismatch: false,
            }
        ));
    }


    #[test]
    fn interpret_shutdown_response_surfaces_auth_rejection() {
        let err = interpret_shutdown_response(Ok(FetchCortexResponse {
            status: 401,
            body: "{\"error\":\"Unauthorized\"}".to_string(),
        }))
        .unwrap_err();

        assert!(err.contains("Refresh the token"));
    }
