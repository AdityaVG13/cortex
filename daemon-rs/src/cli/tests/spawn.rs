// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    fn spawned_owner_parent_probe_child_process() {
        if std::env::var(SPAWN_PARENT_TEST_CHILD_ENV).ok().as_deref() != Some("1") {
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    #[test]
    fn control_center_lock_holder_child_process() {
        if std::env::var(CONTROL_CENTER_LOCK_TEST_CHILD_ENV)
            .ok()
            .as_deref()
            != Some("1")
        {
            return;
        }
        let home = std::env::var(CONTROL_CENTER_LOCK_TEST_HOME_ENV)
            .expect("control-center lock test home env missing");
        let ready_file = std::env::var(CONTROL_CENTER_LOCK_TEST_READY_ENV)
            .expect("control-center lock ready marker env missing");
        let hold_ms = std::env::var(CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1500);
        let lock_path = PathBuf::from(home)
            .join("runtime")
            .join(CONTROL_CENTER_LOCK_FILE);
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent).expect("create lock parent dir");
        }
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .expect("open lock file");
        lock_file
            .try_lock_exclusive()
            .expect("acquire control-center lock");
        std::fs::write(ready_file, b"locked").expect("write lock ready marker");
        std::thread::sleep(Duration::from_millis(hold_ms));
    }

    fn wait_for_control_center_lock(paths: &auth::CortexPaths, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if control_center_is_active(paths).unwrap_or(false) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    }

    #[test]
    fn background_db_lock_wait_env_is_clamped() {
        let _env_guard = env_guard();
        let _small = ScopedEnvVar::set(BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, "1");
        assert_eq!(background_db_lock_max_wait(), Duration::from_millis(100));
        drop(_small);

        let _large = ScopedEnvVar::set(BACKGROUND_DB_LOCK_MAX_WAIT_MS_ENV, "70000");
        assert_eq!(background_db_lock_max_wait(), Duration::from_millis(60_000));
    }

    #[test]
    fn spawned_owner_runtime_claim_requires_parent_linkage_for_plugin_owner() {
        let home_dir = temp_test_dir("owner_runtime_parent");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let err =
            validate_spawned_owner_runtime_claim(&paths, Some("plugin-claude"), None, None, None)
                .unwrap_err();
        assert!(err.contains(SPAWN_PARENT_PID_ENV));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_missing_owner_token_when_parent_set() {
        let home_dir = temp_test_dir("owner_runtime_token");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let parent_start_time =
            process_pid_start_time(parent_pid).expect("current process start time should resolve");
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            Some(parent_start_time),
            None,
        )
        .unwrap_err();
        assert!(err.contains("missing ownership token"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_dead_parent_process() {
        let _env_guard = env_guard();
        let home_dir = temp_test_dir("owner_runtime_dead_parent");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let current_exe = std::env::current_exe().expect("resolve current test binary path");
        let mut child = Command::new(current_exe)
            .arg("--exact")
            .arg("tests::spawned_owner_parent_probe_child_process")
            .arg("--nocapture")
            .env(SPAWN_PARENT_TEST_CHILD_ENV, "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn parent probe child");
        let parent_pid = child.id();
        let deadline = Instant::now() + Duration::from_secs(5);
        let parent_start_time = loop {
            if let Some(start_time) = process_pid_start_time(parent_pid) {
                break start_time;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("failed to resolve child start time");
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let status = child.wait().expect("wait on parent probe child");
        assert!(
            status.success(),
            "parent probe child should exit successfully"
        );

        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            Some(parent_start_time),
            None,
        )
        .unwrap_err();
        assert!(err.contains("not running during ownership claim validation"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[tokio::test]
    async fn spawn_parent_orphan_watch_task_triggers_shutdown_when_parent_identity_breaks() {
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let shared_shutdown_tx = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
        let probe_count = Arc::new(AtomicUsize::new(0));
        let probe_count_for_task = Arc::clone(&probe_count);
        let watcher = spawn_parent_orphan_watch_task(
            Arc::clone(&shared_shutdown_tx),
            4242,
            123,
            Duration::from_millis(5),
            move |_, _| probe_count_for_task.fetch_add(1, Ordering::SeqCst) == 0,
        );

        tokio::time::timeout(Duration::from_millis(250), shutdown_rx)
            .await
            .expect("watcher should signal shutdown when parent identity breaks")
            .expect("shutdown channel should deliver signal");
        watcher
            .await
            .expect("spawn-parent watcher task should exit cleanly");
        assert!(
            probe_count.load(Ordering::SeqCst) >= 2,
            "watcher should probe parent identity more than once"
        );
        assert!(
            shared_shutdown_tx.lock().await.is_none(),
            "shutdown sender should be consumed after parent identity break"
        );
    }

    #[tokio::test]
    async fn spawn_parent_orphan_watch_task_detects_real_parent_exit() {
        let mut parent_probe_child = {
            let _env_guard = crate::test_env::lock_async().await;
            let current_exe = std::env::current_exe().expect("resolve current test binary path");
            Command::new(current_exe)
                .arg("--exact")
                .arg("tests::spawned_owner_parent_probe_child_process")
                .arg("--nocapture")
                .env(SPAWN_PARENT_TEST_CHILD_ENV, "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn parent probe child")
        };
        let parent_pid = parent_probe_child.id();
        let deadline = Instant::now() + Duration::from_secs(5);
        let parent_start_time = loop {
            if let Some(start_time) = process_pid_start_time(parent_pid) {
                break start_time;
            }
            if Instant::now() >= deadline {
                let _ = parent_probe_child.kill();
                let _ = parent_probe_child.wait();
                panic!("failed to resolve child process start time");
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let shared_shutdown_tx = Arc::new(tokio::sync::Mutex::new(Some(shutdown_tx)));
        let watcher = spawn_parent_orphan_watch_task(
            Arc::clone(&shared_shutdown_tx),
            parent_pid,
            parent_start_time,
            Duration::from_millis(20),
            process_pid_identity_matches,
        );

        let status = parent_probe_child
            .wait()
            .expect("wait on parent probe child");
        assert!(
            status.success(),
            "parent probe child should exit successfully"
        );
        tokio::time::timeout(Duration::from_secs(2), shutdown_rx)
            .await
            .expect("watcher should observe real parent process exit")
            .expect("shutdown signal should be delivered");
        watcher
            .await
            .expect("spawn-parent watcher task should exit cleanly");
        assert!(
            shared_shutdown_tx.lock().await.is_none(),
            "shutdown sender should be consumed after real parent exit"
        );
    }

    #[test]
    fn spawned_owner_runtime_claim_allows_unspawned_control_center_mode() {
        let home_dir = temp_test_dir("owner_runtime_unspawned");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        validate_spawned_owner_runtime_claim(&paths, Some("control-center"), None, None, None)
            .expect("direct control-center owner mode should remain compatible");

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_missing_parent_start_time_when_parent_set() {
        let home_dir = temp_test_dir("owner_runtime_parent_start");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            None,
            None,
        )
        .unwrap_err();
        assert!(err.contains(SPAWN_PARENT_START_TIME_ENV));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn spawned_owner_runtime_claim_rejects_parent_start_time_mismatch() {
        let home_dir = temp_test_dir("owner_runtime_parent_start_mismatch");
        std::fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let parent_pid = std::process::id();
        let err = validate_spawned_owner_runtime_claim(
            &paths,
            Some("plugin-claude"),
            Some(parent_pid),
            Some(0),
            Some("invalid-token"),
        )
        .unwrap_err();
        assert!(err.contains("start-time mismatch"));

        let _ = std::fs::remove_dir_all(&home_dir);
    }

}
