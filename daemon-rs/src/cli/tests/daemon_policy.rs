// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    #[test]
    fn resolve_boot_auth_header_prefers_api_key_and_falls_back_to_token_file() {
        let home_dir = temp_test_dir("boot_auth");
        fs::create_dir_all(&home_dir).unwrap();
        let token_path = home_dir.join("cortex.token");
        fs::write(&token_path, "local-token").unwrap();

        let explicit = resolve_boot_auth_header(&token_path, Some("ctx_remote"), true);
        assert_eq!(explicit, Some("Bearer ctx_remote".to_string()));

        let fallback = resolve_boot_auth_header(&token_path, None, true);
        assert_eq!(fallback, Some("Bearer local-token".to_string()));

        fs::write(&token_path, "   ").unwrap();
        assert_eq!(resolve_boot_auth_header(&token_path, None, true), None);
        assert_eq!(resolve_boot_auth_header(&token_path, None, false), None);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn parse_truthy_flag_accepts_expected_values() {
        assert!(parse_truthy_flag("1"));
        assert!(parse_truthy_flag("true"));
        assert!(parse_truthy_flag("YES"));
        assert!(parse_truthy_flag(" on "));
        assert!(!parse_truthy_flag("0"));
        assert!(!parse_truthy_flag("false"));
        assert!(!parse_truthy_flag(""));
    }

    #[test]
    fn single_daemon_test_bypass_flag_respects_debug_gate() {
        let _env_guard = env_guard();
        std::env::remove_var(SINGLE_DAEMON_TEST_BYPASS_ENV);
        assert!(!single_daemon_test_bypass_enabled());

        let _bypass = ScopedEnvVar::set(SINGLE_DAEMON_TEST_BYPASS_ENV, "1");
        assert_eq!(single_daemon_test_bypass_enabled(), cfg!(debug_assertions));
    }

    #[test]
    fn local_spawn_policy_fails_closed_for_marked_app_client_without_spawn_contract() {
        let _env_guard = env_guard();
        std::env::remove_var(APP_REQUIRED_ENV);
        std::env::remove_var(DAEMON_LOCAL_SPAWN_ENV);
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        assert!(
            !local_spawn_allowed_for_request(true),
            "app-marked clients should fail closed when spawn policy is missing"
        );
    }

    #[test]
    fn local_spawn_policy_allows_explicit_opt_in_for_marked_app_client() {
        let _env_guard = env_guard();
        std::env::remove_var(APP_REQUIRED_ENV);
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        let _local_spawn = ScopedEnvVar::set(DAEMON_LOCAL_SPAWN_ENV, "1");
        assert!(
            local_spawn_allowed_for_request(true),
            "explicit local spawn opt-in should allow startup when app-required is unset"
        );
    }

    #[test]
    fn local_spawn_policy_app_required_overrides_local_spawn_opt_in() {
        let _env_guard = env_guard();
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        let _local_spawn = ScopedEnvVar::set(DAEMON_LOCAL_SPAWN_ENV, "1");
        let _app_required = ScopedEnvVar::set(APP_REQUIRED_ENV, "1");
        assert!(
            !local_spawn_allowed_for_request(true),
            "app-required must force attach-only behavior even when local spawn is enabled"
        );
    }

    #[test]
    fn local_spawn_policy_respects_allow_service_ensure_short_circuit() {
        let _env_guard = env_guard();
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        let _local_spawn = ScopedEnvVar::set(DAEMON_LOCAL_SPAWN_ENV, "1");
        std::env::remove_var(APP_REQUIRED_ENV);
        assert!(
            !local_spawn_allowed_for_request(false),
            "service-ensure gate must disable local spawn regardless of env opt-ins"
        );
    }

    #[test]
    fn ensure_daemon_app_required_policy_returns_machine_readable_error() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("app_required_policy");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let _app_required = ScopedEnvVar::set(APP_REQUIRED_ENV, "1");
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        let err = run_ensure_daemon(&paths, Some("codex"), false, false).unwrap_err();
        assert!(err.contains("APP_INIT_REQUIRED"));
        assert!(err.contains("codex"));
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn ensure_daemon_app_required_policy_does_not_migrate_legacy_db() {
        let _env_guard = env_guard();
        let legacy_home = temp_test_dir("app_required_legacy_source");
        let legacy_dir = legacy_home.join("cortex");
        fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        let legacy_db = legacy_dir.join("cortex.db");
        {
            let conn = rusqlite::Connection::open(&legacy_db).expect("open legacy db");
            db::configure(&conn).expect("configure legacy db");
        }

        let home_dir = temp_test_dir("app_required_no_migration");
        fs::create_dir_all(&home_dir).expect("create target home");
        let home_str = home_dir.to_string_lossy().to_string();
        let legacy_home_str = legacy_home.to_string_lossy().to_string();
        let _home_env = ScopedEnvVar::set("HOME", &legacy_home_str);
        let _userprofile_env = ScopedEnvVar::set("USERPROFILE", &legacy_home_str);
        let _app_required = ScopedEnvVar::set(APP_REQUIRED_ENV, "1");
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "codex");
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let err = run_ensure_daemon(&paths, Some("codex"), false, false).unwrap_err();
        assert!(err.contains("APP_INIT_REQUIRED"));
        assert!(
            !paths.db.exists(),
            "attach-only ensure should not copy legacy db before returning APP_INIT_REQUIRED"
        );

        let _ = fs::remove_dir_all(&home_dir);
        let _ = fs::remove_dir_all(&legacy_home);
    }

    #[test]
    fn ensure_daemon_respects_local_spawn_disable_flag() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("local_spawn_disabled_policy");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let _local_spawn = ScopedEnvVar::set(DAEMON_LOCAL_SPAWN_ENV, "0");
        let _app_client = ScopedEnvVar::set(APP_CLIENT_ENV, "claude");
        let err = run_ensure_daemon(&paths, Some("claude"), false, true).unwrap_err();
        assert!(err.contains("APP_INIT_REQUIRED"));
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn ensure_daemon_blocks_local_spawn_when_control_center_lock_is_held() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("control_center_lock_blocks_spawn");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let _home_env = ScopedEnvVar::set("HOME", &home_str);
        let _userprofile_env = ScopedEnvVar::set("USERPROFILE", &home_str);
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        let ready_file = home_dir.join("control-center-lock-ready");
        std::env::remove_var(APP_REQUIRED_ENV);
        std::env::remove_var(APP_CLIENT_ENV);
        std::env::remove_var(DAEMON_LOCAL_SPAWN_ENV);

        let current_exe = std::env::current_exe().expect("resolve current test binary path");
        let mut child = Command::new(current_exe)
            .arg("--exact")
            .arg("tests::control_center_lock_holder_child_process")
            .arg("--nocapture")
            .env(CONTROL_CENTER_LOCK_TEST_CHILD_ENV, "1")
            .env(CONTROL_CENTER_LOCK_TEST_HOME_ENV, &home_str)
            .env(
                CONTROL_CENTER_LOCK_TEST_READY_ENV,
                ready_file.to_string_lossy().to_string(),
            )
            .env(CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV, "2000")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn lock-holder child");

        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready_file.exists() {
            if Instant::now() >= ready_deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("lock-holder child never reported readiness");
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        assert!(
            wait_for_control_center_lock(&paths, Duration::from_secs(3)),
            "control-center lock should be active before ensure_daemon call"
        );

        let err = run_ensure_daemon(&paths, Some("claude"), false, true).unwrap_err();
        assert!(
            err.contains("APP_INIT_REQUIRED"),
            "control-center lock should force attach-only error: {err}"
        );

        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn ensure_daemon_attach_only_policy_holds_under_concurrent_app_clients() {
        let _env_guard = env_guard();
        let _app_required = ScopedEnvVar::set(APP_REQUIRED_ENV, "1");
        std::env::remove_var(APP_CLIENT_ENV);
        std::env::remove_var(DAEMON_LOCAL_SPAWN_ENV);

        let agents = ["codex", "claude", "gpt5"];
        let workers: Vec<_> = agents
            .iter()
            .map(|agent| {
                let agent_name = (*agent).to_string();
                std::thread::spawn(move || {
                    let home_dir = temp_test_dir(&format!("app_required_concurrent_{agent_name}"));
                    fs::create_dir_all(&home_dir).expect("create temp home");
                    let home_str = home_dir.to_string_lossy().to_string();
                    let paths = auth::CortexPaths::resolve_with_overrides(
                        Some(&home_str),
                        None,
                        Some(7437),
                        None,
                    );
                    let err = run_ensure_daemon(&paths, Some(&agent_name), false, false)
                        .expect_err("attach-only clients should not spawn daemon");
                    let _ = fs::remove_dir_all(&home_dir);
                    (agent_name, err)
                })
            })
            .collect();

        for worker in workers {
            let (agent_name, err) = worker.join().expect("join worker");
            assert!(
                err.contains("APP_INIT_REQUIRED"),
                "missing machine-readable attach-only marker for {agent_name}: {err}"
            );
            assert!(
                err.contains(&agent_name),
                "attach-only error should identify requesting agent {agent_name}: {err}"
            );
        }
    }

    #[test]
    fn ensure_daemon_attach_only_policy_holds_under_cross_surface_concurrency() {
        let _env_guard = env_guard();
        let _app_required = ScopedEnvVar::set(APP_REQUIRED_ENV, "1");
        std::env::remove_var(APP_CLIENT_ENV);
        std::env::remove_var(DAEMON_LOCAL_SPAWN_ENV);

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
        let port = listener.local_addr().expect("listener addr").port();
        drop(listener);

        let home_dir = temp_test_dir("app_required_cross_surface_shared_home");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let home_str = home_dir.to_string_lossy().to_string();
        let pid_path = home_dir.join("cortex.pid");

        let workers = vec![
            ("cli-codex".to_string(), Some("codex".to_string()), false),
            (
                "plugin-claude".to_string(),
                Some("claude-code".to_string()),
                true,
            ),
            ("direct-cli".to_string(), None, false),
        ];

        let handles: Vec<_> = workers
            .into_iter()
            .map(|(label, agent, allow_service_ensure)| {
                let worker_home = home_str.clone();
                std::thread::spawn(move || {
                    let paths = auth::CortexPaths::resolve_with_overrides(
                        Some(&worker_home),
                        None,
                        Some(port),
                        None,
                    );
                    let err =
                        run_ensure_daemon(&paths, agent.as_deref(), false, allow_service_ensure)
                            .expect_err(
                                "cross-surface attach-only callers should not spawn daemon",
                            );
                    (label, err)
                })
            })
            .collect();

        for handle in handles {
            let (label, err) = handle.join().expect("join worker");
            assert!(
                err.contains("APP_INIT_REQUIRED"),
                "cross-surface attach-only marker missing for {label}: {err}"
            );
        }
        assert!(
            !pid_path.exists(),
            "attach-only cross-surface contention should not create daemon pid file"
        );
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
}
