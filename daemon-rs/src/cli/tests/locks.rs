// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    fn acquire_runtime_lock_rejects_duplicate_serve_startup() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("runtime_lock");
        fs::create_dir_all(&home_dir).unwrap();
        let global_lock_home = temp_test_dir("runtime_lock_global");
        fs::create_dir_all(&global_lock_home).unwrap();
        let global_lock_home_str = global_lock_home.to_string_lossy().to_string();
        let _global_lock_home = ScopedEnvVar::set("CORTEX_GLOBAL_LOCK_HOME", &global_lock_home_str);

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let first_lock = acquire_runtime_lock(&paths).unwrap();
        let err = acquire_runtime_lock(&paths).unwrap_err();

        assert!(err.contains("another cortex instance"));

        drop(first_lock);
        let _ = fs::remove_dir_all(&home_dir);
        let _ = fs::remove_dir_all(&global_lock_home);
    }

    #[test]
    fn control_center_lock_detection_reports_cross_process_holder() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("control_center_lock_detection");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        let ready_file = home_dir.join("control-center-lock-ready");

        assert!(
            !control_center_is_active(&paths).expect("probe lock without holder"),
            "lock should not appear active before holder starts"
        );

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
            .env(CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV, "30000")
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
            "lock should appear active while child holds cross-process lock"
        );

        let status = child.wait().expect("wait lock-holder child");
        assert!(
            status.success(),
            "lock-holder child should exit successfully"
        );

        assert!(
            !control_center_is_active(&paths).expect("probe lock after holder exits"),
            "lock should be released after child exits"
        );

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn acquire_runtime_lock_waits_for_handoff_when_enabled() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("runtime_lock_handoff");
        fs::create_dir_all(&home_dir).unwrap();
        let global_lock_home = temp_test_dir("runtime_lock_handoff_global");
        fs::create_dir_all(&global_lock_home).unwrap();
        let global_lock_home_str = global_lock_home.to_string_lossy().to_string();
        let _global_lock_home = ScopedEnvVar::set("CORTEX_GLOBAL_LOCK_HOME", &global_lock_home_str);

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let first_lock = acquire_runtime_lock(&paths).unwrap();
        let _wait_lock_flag = ScopedEnvVar::set("CORTEX_WAIT_FOR_DAEMON_LOCK", "1");
        let _wait_secs_flag = ScopedEnvVar::set("CORTEX_DAEMON_LOCK_WAIT_SECS", "1");

        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first_lock);
        });

        let second_lock = acquire_runtime_lock(&paths).expect("lock handoff should succeed");
        drop(second_lock);
        releaser.join().unwrap();

        let _ = fs::remove_dir_all(&home_dir);
        let _ = fs::remove_dir_all(&global_lock_home);
    }

    #[test]
    fn acquire_runtime_lock_rejects_concurrent_startup_burst() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("runtime_lock_burst");
        fs::create_dir_all(&home_dir).unwrap();
        let global_lock_home = temp_test_dir("runtime_lock_burst_global");
        fs::create_dir_all(&global_lock_home).unwrap();
        let global_lock_home_str = global_lock_home.to_string_lossy().to_string();
        let _global_lock_home = ScopedEnvVar::set("CORTEX_GLOBAL_LOCK_HOME", &global_lock_home_str);

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = Arc::new(auth::CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            None,
        ));

        let first_lock = acquire_runtime_lock(&paths).expect("first runtime lock must succeed");
        let workers: Vec<_> = (0..12)
            .map(|_| {
                let worker_paths = Arc::clone(&paths);
                std::thread::spawn(move || acquire_runtime_lock(&worker_paths).is_err())
            })
            .collect();
        let failures = workers
            .into_iter()
            .map(|worker| worker.join().expect("join worker"))
            .filter(|failed| *failed)
            .count();
        assert_eq!(
            failures, 12,
            "all concurrent startups should fail while runtime lock is held"
        );

        drop(first_lock);
        let second_lock =
            acquire_runtime_lock(&paths).expect("lock should be reacquired after release");
        drop(second_lock);

        let _ = fs::remove_dir_all(&home_dir);
        let _ = fs::remove_dir_all(&global_lock_home);
    }

    #[test]
    fn run_stale_pid_cleanup_keeps_lock_file() {
        let home_dir = temp_test_dir("stale_pid_cleanup");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        fs::write(&paths.pid, "999999").unwrap();
        fs::write(&paths.lock, "lock-held").unwrap();

        let dry_run = run_stale_pid_cleanup(&paths, true);
        assert_eq!(
            dry_run,
            vec!["DELETE cortex.pid (process 999999 not running)"]
        );
        assert!(paths.pid.exists());
        assert!(paths.lock.exists());

        let apply = run_stale_pid_cleanup(&paths, false);
        assert_eq!(
            apply,
            vec!["DELETE cortex.pid (process 999999 not running)"]
        );
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_requires_parent_pid_only_for_non_control_center_owner() {
        assert!(spawned_owner_requires_parent_pid(Some("cli-mcp")));
        assert!(spawned_owner_requires_parent_pid(Some("plugin-claude")));
        assert!(!spawned_owner_requires_parent_pid(Some("control-center")));
        assert!(!spawned_owner_requires_parent_pid(None));
    }

    #[test]
    fn is_control_center_owner_is_case_insensitive() {
        assert!(is_control_center_owner(Some("control-center")));
        assert!(is_control_center_owner(Some("CoNtRoL-CeNtEr")));
        assert!(!is_control_center_owner(Some("plugin-claude")));
        assert!(!is_control_center_owner(None));
    }

    #[test]
    fn app_managed_startup_heavy_delay_only_applies_to_control_center_owner() {
        let _env_guard = env_guard();
        std::env::remove_var(APP_MANAGED_STARTUP_DELAY_ENV);
        assert_eq!(
            app_managed_startup_heavy_delay(Some("control-center")),
            Duration::from_secs(APP_MANAGED_STARTUP_HEAVY_DELAY_SECS)
        );
        assert_eq!(
            app_managed_startup_heavy_delay(Some("plugin-claude")),
            Duration::from_secs(0)
        );

        let _startup_delay = ScopedEnvVar::set(APP_MANAGED_STARTUP_DELAY_ENV, "0");
        assert_eq!(
            app_managed_startup_heavy_delay(Some("control-center")),
            Duration::from_secs(0)
        );

        drop(_startup_delay);
        let _excessive_delay = ScopedEnvVar::set(APP_MANAGED_STARTUP_DELAY_ENV, "777");
        assert_eq!(
            app_managed_startup_heavy_delay(Some("control-center")),
            Duration::from_secs(APP_MANAGED_STARTUP_HEAVY_DELAY_MAX_SECS)
        );
    }

    #[test]
    fn startup_schedule_uses_non_app_defaults_for_plugin_owner() {
        let _env_guard = env_guard();
        let _app_delay = ScopedEnvVar::set(APP_MANAGED_STARTUP_DELAY_ENV, "");
        let _index_delay = ScopedEnvVar::set(STARTUP_INDEX_DELAY_ENV, "");
        let _aging_delay = ScopedEnvVar::set(STARTUP_AGING_DELAY_ENV, "");
        let _embed_delay = ScopedEnvVar::set(STARTUP_EMBED_DELAY_ENV, "");
        let _crystallize_delay = ScopedEnvVar::set(STARTUP_CRYSTALLIZE_DELAY_ENV, "");
        let _storage_delay = ScopedEnvVar::set(STARTUP_STORAGE_GOVERNOR_DELAY_ENV, "");

        let schedule = startup_schedule(Some("plugin-claude"));
        assert_eq!(
            schedule.index,
            Duration::from_secs(DEFAULT_STARTUP_INDEX_DELAY_SECS)
        );
        assert_eq!(
            schedule.aging,
            Duration::from_secs(DEFAULT_STARTUP_AGING_DELAY_SECS)
        );
        assert_eq!(
            schedule.embed,
            Duration::from_secs(DEFAULT_STARTUP_EMBED_DELAY_SECS)
        );
        assert_eq!(
            schedule.crystallize,
            Duration::from_secs(DEFAULT_STARTUP_CRYSTALLIZE_DELAY_SECS)
        );
        assert_eq!(
            schedule.storage_governor_initial,
            Duration::from_secs(DEFAULT_STARTUP_STORAGE_GOVERNOR_DELAY_SECS)
        );
    }

    #[test]
    fn startup_schedule_applies_app_managed_offsets_for_control_center() {
        let _env_guard = env_guard();
        let _app_delay = ScopedEnvVar::set(APP_MANAGED_STARTUP_DELAY_ENV, "10");
        let _index_delay = ScopedEnvVar::set(STARTUP_INDEX_DELAY_ENV, "1");
        let _aging_delay = ScopedEnvVar::set(STARTUP_AGING_DELAY_ENV, "1");
        let _embed_delay = ScopedEnvVar::set(STARTUP_EMBED_DELAY_ENV, "1");
        let _crystallize_delay = ScopedEnvVar::set(STARTUP_CRYSTALLIZE_DELAY_ENV, "1");
        let _storage_delay = ScopedEnvVar::set(STARTUP_STORAGE_GOVERNOR_DELAY_ENV, "7");

        let schedule = startup_schedule(Some("control-center"));
        assert_eq!(schedule.index, Duration::from_secs(10));
        assert_eq!(
            schedule.aging,
            Duration::from_secs(10 + APP_MANAGED_AGING_STARTUP_OFFSET_SECS)
        );
        assert_eq!(
            schedule.embed,
            Duration::from_secs(10 + APP_MANAGED_EMBED_STARTUP_OFFSET_SECS)
        );
        assert_eq!(
            schedule.crystallize,
            Duration::from_secs(10 + APP_MANAGED_CRYSTALLIZE_STARTUP_OFFSET_SECS)
        );
        assert_eq!(schedule.storage_governor_initial, Duration::from_secs(7));
}
