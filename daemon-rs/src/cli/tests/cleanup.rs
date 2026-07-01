// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    fn rotate_backups_keeps_three_most_recent_files() {
        let backup_dir = temp_test_dir("backup_rotation");
        fs::create_dir_all(&backup_dir).unwrap();

        for idx in 0..5 {
            let path = backup_dir.join(format!("cortex-2026040{}.db", idx + 1));
            fs::write(&path, format!("backup-{idx}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let removed = rotate_backups(&backup_dir, BACKUP_RETENTION_COUNT).unwrap();
        assert_eq!(removed, 2);

        let mut remaining: Vec<String> = fs::read_dir(&backup_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        remaining.sort();

        assert_eq!(
            remaining,
            vec![
                "cortex-20260403.db".to_string(),
                "cortex-20260404.db".to_string(),
                "cortex-20260405.db".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&backup_dir);
    }

    #[test]
    fn cleanup_bridge_backups_requires_schema_version_five_or_higher() {
        let home_dir = temp_test_dir("bridge_backups");
        let bridge_dir = home_dir.join("bridge-backups");
        fs::create_dir_all(&bridge_dir).unwrap();
        fs::write(bridge_dir.join("legacy.txt"), "legacy").unwrap();

        assert!(!cleanup_bridge_backups(&home_dir, 4));
        assert!(bridge_dir.exists());

        assert!(cleanup_bridge_backups(&home_dir, 5));
        assert!(!bridge_dir.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn rotate_log_file_replaces_existing_rotation_and_creates_fresh_log() {
        let home_dir = temp_test_dir("log_rotation");
        fs::create_dir_all(&home_dir).unwrap();

        let log_path = home_dir.join("daemon.log");
        let rotated_path = home_dir.join("daemon.log.1");
        fs::write(&rotated_path, "old-rotation").unwrap();
        fs::write(&log_path, vec![b'x'; (LOG_ROTATION_BYTES as usize) + 1]).unwrap();

        assert!(rotate_log_file(&home_dir, "daemon.log").unwrap());
        assert!(log_path.exists());
        assert_eq!(fs::metadata(&log_path).unwrap().len(), 0);
        assert_eq!(
            fs::metadata(&rotated_path).unwrap().len(),
            LOG_ROTATION_BYTES + 1
        );

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn run_backup_cleanup_dry_run_reports_files_without_deleting_them() {
        let backup_dir = temp_test_dir("backup_cleanup_dry_run");
        fs::create_dir_all(&backup_dir).unwrap();

        for idx in 0..4 {
            let path = backup_dir.join(format!("cortex-2026040{}.db", idx + 1));
            fs::write(&path, format!("backup-{idx}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }

        let lines = run_backup_cleanup(&backup_dir, true);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("DELETE backups/"));
        assert_eq!(fs::read_dir(&backup_dir).unwrap().count(), 4);

        let _ = fs::remove_dir_all(&backup_dir);
    }

    #[test]
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
}
