// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use crate::cli::*;
    use crate::*;
    use std::fs;
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const SPAWN_PARENT_TEST_CHILD_ENV: &str = "CORTEX_SPAWN_PARENT_TEST_CHILD";
    const CONTROL_CENTER_LOCK_TEST_CHILD_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_CHILD";
    const CONTROL_CENTER_LOCK_TEST_HOME_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOME";
    const CONTROL_CENTER_LOCK_TEST_READY_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_READY";
    const CONTROL_CENTER_LOCK_TEST_HOLD_MS_ENV: &str = "CORTEX_CONTROL_CENTER_LOCK_TEST_HOLD_MS";

    fn openapi_spec_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("specs")
            .join("cortex-openapi.yaml")
    }

    struct ScopedEnvVar {
        key: &'static str,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            std::env::set_var(key, value);
            Self { key }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            std::env::remove_var(self.key);
        }
    }

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_{name}_{unique}"))
    }

    fn run_preflight(paths: &auth::CortexPaths) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
            .block_on(startup_single_daemon_preflight(paths))
    }

    fn run_ensure_daemon(
        paths: &auth::CortexPaths,
        agent: Option<&str>,
        emit_port: bool,
        allow_service_ensure: bool,
    ) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
            .block_on(ensure_daemon(paths, agent, emit_port, allow_service_ensure))
    }

    fn spawn_response_server(
        listener: TcpListener,
        status_line: &str,
        content_type: &str,
        body: String,
        max_requests: usize,
    ) -> std::thread::JoinHandle<()> {
        let status_line = status_line.to_string();
        let content_type = content_type.to_string();
        let max_requests = max_requests.max(1);
        std::thread::spawn(move || {
            let _ = listener.set_nonblocking(true);
            let deadline = Instant::now() + Duration::from_secs(15);
            let idle_grace_after_response = Duration::from_millis(500);
            let mut served = 0_usize;
            let mut last_served_at: Option<Instant> = None;
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request_buffer = [0_u8; 2048];
                        let _ = stream.read(&mut request_buffer);
                        let response = format!(
                            "HTTP/1.1 {status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        served += 1;
                        last_served_at = Some(Instant::now());
                        if served >= max_requests {
                            break;
                        }
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        let now = Instant::now();
                        if served > 0
                            && last_served_at.is_some_and(|last| {
                                now.duration_since(last) >= idle_grace_after_response
                            })
                        {
                            break;
                        }
                        if now >= deadline {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        })
    }

    fn spawn_preflight_response_server(
        listener: TcpListener,
        status_line: &str,
        content_type: &str,
        body: String,
    ) -> std::thread::JoinHandle<()> {
        spawn_response_server(listener, status_line, content_type, body, 4)
    }

    #[test]
    fn cli_usage_exposes_agent_entrypoints() {
        let usage = cli_usage_text();
        assert!(usage.contains("cortex capabilities --json"));
        assert!(usage.contains("status [--json]"));
        assert!(usage.contains("cortex robot-docs guide"));
        assert!(usage.contains("Agent surfaces:"));
    }

    #[test]
    fn cli_capabilities_payload_has_agent_contract() {
        let payload = cli_capabilities_payload();
        assert_eq!(
            payload["contract_version"],
            CLI_CAPABILITIES_CONTRACT_VERSION
        );
        assert_eq!(
            payload["tool"]["default_port"].as_u64(),
            Some(DEFAULT_CORTEX_PORT as u64)
        );
        assert_eq!(payload["commands"]["status"]["side_effects"], "none");
        assert_eq!(payload["commands"]["status"]["output"], "human_or_json");
        assert_eq!(payload["commands"]["paths"]["output"], "json");
        assert_eq!(payload["exit_codes"]["0"], "success");
    }

    fn status_test_paths(name: &str) -> auth::CortexPaths {
        let home = temp_test_dir(name);
        let home_str = home.to_string_lossy().to_string();
        auth::CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        )
    }

    fn status_check<'a>(payload: &'a Value, name: &str) -> &'a Value {
        payload["checks"]
            .as_array()
            .expect("checks array")
            .iter()
            .find(|check| check["name"] == name)
            .unwrap_or_else(|| panic!("missing status check {name}"))
    }

    #[test]
    fn status_report_ready_json_has_schema_next_action_and_checks() {
        let paths = status_test_paths("status_ready");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::Ready("Readiness endpoint reports ready.".to_string()),
            true,
            true,
        );

        assert_eq!(report.exit_code, 0);
        assert_eq!(report.payload["schemaVersion"], STATUS_SCHEMA_VERSION);
        assert_eq!(report.payload["status"], "ready");
        assert_eq!(
            report.payload["nextAction"]["kind"],
            "connect_tool_or_smoke"
        );
        assert_eq!(report.payload["repair"], Value::Null);
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["status"],
            "ok"
        );
        assert_eq!(status_check(&report.payload, "auth_token")["status"], "ok");
    }

    #[test]
    fn status_report_unavailable_returns_repair_action_and_nonzero() {
        let paths = status_test_paths("status_unavailable");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::Unavailable(
                "readiness failed: connection refused; health failed: connection refused"
                    .to_string(),
            ),
            true,
            false,
        );

        assert_eq!(report.exit_code, 1);
        assert_eq!(report.payload["status"], "needs_action");
        assert_eq!(report.payload["repair"]["kind"], "start_local_runtime");
        assert_eq!(report.payload["repair"]["command"], "cortex serve");
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["repair"]["kind"],
            "start_local_runtime"
        );
    }

    #[test]
    fn status_report_wrong_identity_is_error_not_ready() {
        let paths = status_test_paths("status_wrong_identity");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::WrongIdentity(
                "Health endpoint answered, but home/db/token paths do not match.".to_string(),
            ),
            true,
            true,
        );

        assert_eq!(report.exit_code, 1);
        assert_eq!(report.payload["status"], "error");
        assert_eq!(report.payload["repair"]["kind"], "repair_runtime_identity");
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["status"],
            "fail"
        );
    }

    #[test]
    fn robot_docs_guide_is_paste_ready_for_agents() {
        let guide = cli_robot_docs_guide();
        assert!(guide.contains("cortex capabilities --json"));
        assert!(guide.contains("cortex status --json"));
        assert!(guide.contains("cortex boot --json"));
        assert!(guide.contains("Danger gates:"));
        assert!(guide.contains("Treat exit code 0 as success"));
    }

    #[test]
    fn unknown_command_message_suggests_likely_agent_surface() {
        let message = unknown_cli_command_message("capability");
        assert!(message.contains("Unknown command: capability"));
        assert!(message.contains("Did you mean: `cortex capabilities --json`?"));
        assert!(message.contains("cortex help"));
    }

    #[test]
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
    fn backfill_batch_may_have_more_only_when_a_table_hits_limit() {
        assert!(!backfill_batch_may_have_more(0, 0, 32));
        assert!(!backfill_batch_may_have_more(31, 8, 32));
        assert!(!backfill_batch_may_have_more(8, 31, 32));
        assert!(backfill_batch_may_have_more(32, 8, 32));
        assert!(backfill_batch_may_have_more(8, 32, 32));
        assert!(backfill_batch_may_have_more(32, 32, 32));
    }

    #[test]
    fn collect_unembedded_targets_for_model_rebuilds_mismatched_embeddings() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["legacy memory", "memory::legacy"],
        )
        .expect("insert memory legacy");
        let memory_legacy_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["current memory", "memory::current"],
        )
        .expect("insert memory current");
        let memory_current_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["legacy decision", "ctx::legacy"],
        )
        .expect("insert decision legacy");
        let decision_legacy_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["current decision", "ctx::current"],
        )
        .expect("insert decision current");
        let decision_current_id = conn.last_insert_rowid();

        let sample_blob = crate::embeddings::vector_to_blob(&[0.1, 0.2, 0.3]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            rusqlite::params![memory_legacy_id, sample_blob.clone()],
        )
        .expect("insert legacy memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
            rusqlite::params![memory_current_id, sample_blob.clone()],
        )
        .expect("insert current memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'OTHER-MODEL')",
            rusqlite::params![decision_legacy_id, sample_blob.clone()],
        )
        .expect("insert legacy decision embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-minilm-l6-v2')",
            rusqlite::params![decision_current_id, sample_blob],
        )
        .expect("insert current decision embedding");

        let (memories, decisions) =
            collect_unembedded_targets_for_model(&conn, "all-minilm-l6-v2", 256);
        let memory_ids: std::collections::HashSet<i64> =
            memories.iter().map(|(id, _)| *id).collect();
        let decision_ids: std::collections::HashSet<i64> =
            decisions.iter().map(|(id, _)| *id).collect();

        assert!(
            memory_ids.contains(&memory_legacy_id),
            "mismatched memory model should be queued for re-embedding"
        );
        assert!(
            !memory_ids.contains(&memory_current_id),
            "matching memory model should not be queued"
        );
        assert!(
            decision_ids.contains(&decision_legacy_id),
            "mismatched decision model should be queued for re-embedding"
        );
        assert!(
            !decision_ids.contains(&decision_current_id),
            "matching decision model should not be queued"
        );
    }

    #[test]
    fn collect_unembedded_targets_for_model_respects_limit_per_table() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        for idx in 0..3 {
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
                 VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
                rusqlite::params![format!("memory-{idx}"), format!("memory::{idx}")],
            )
            .expect("insert memory");
        }
        for idx in 0..3 {
            conn.execute(
                "INSERT INTO decisions (decision, context, status, score, merged_count, quality, created_at, updated_at)
                 VALUES (?1, ?2, 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
                rusqlite::params![format!("decision-{idx}"), format!("decision::{idx}")],
            )
            .expect("insert decision");
        }

        let (memories, decisions) =
            collect_unembedded_targets_for_model(&conn, "all-minilm-l6-v2", 1);
        assert_eq!(memories.len(), 1, "memory queue should honor LIMIT");
        assert_eq!(decisions.len(), 1, "decision queue should honor LIMIT");
        assert_eq!(memories[0].0, 1, "memory selection should be deterministic");
        assert_eq!(
            decisions[0].0, 1,
            "decision selection should be deterministic"
        );
    }

    #[test]
    fn count_unembedded_targets_for_model_reports_model_specific_backlog() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-backlog", "tests::count"],
        )
        .expect("insert active backlog memory");
        let memory_backlog_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-current", "tests::count"],
        )
        .expect("insert active current memory");
        let memory_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'archived', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-archived", "tests::count"],
        )
        .expect("insert archived memory");

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-backlog", "tests::count"],
        )
        .expect("insert active backlog decision");
        let decision_backlog_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-current", "tests::count"],
        )
        .expect("insert active current decision");
        let decision_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'archived', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-archived", "tests::count"],
        )
        .expect("insert archived decision");

        let sample_blob = crate::embeddings::vector_to_blob(&[0.1, 0.2, 0.3]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            rusqlite::params![memory_backlog_id, sample_blob.clone()],
        )
        .expect("insert legacy memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-minilm-l6-v2')",
            rusqlite::params![memory_current_id, sample_blob.clone()],
        )
        .expect("insert current memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'other-model')",
            rusqlite::params![decision_backlog_id, sample_blob.clone()],
        )
        .expect("insert legacy decision embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
            rusqlite::params![decision_current_id, sample_blob],
        )
        .expect("insert current decision embedding");

        let (memory_count, decision_count) =
            count_unembedded_targets_for_model(&conn, "all-minilm-l6-v2");
        assert_eq!(
            memory_count, 1,
            "exactly one active memory should be pending"
        );
        assert_eq!(
            decision_count, 1,
            "exactly one active decision should be pending"
        );
    }

    #[test]
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

    #[test]
    fn parse_flag_usize_validates_and_parses_values() {
        let args = vec![
            "--agent".to_string(),
            "codex".to_string(),
            "--budget".to_string(),
            "900".to_string(),
        ];
        assert_eq!(parse_flag_usize(&args, "--budget").unwrap(), Some(900));

        let missing_value = vec!["--budget".to_string()];
        assert!(parse_flag_usize(&missing_value, "--budget")
            .unwrap_err()
            .contains("missing value"));

        let invalid_value = vec!["--budget".to_string(), "abc".to_string()];
        assert!(parse_flag_usize(&invalid_value, "--budget")
            .unwrap_err()
            .contains("invalid value"));

        let zero_value = vec!["--budget".to_string(), "0".to_string()];
        assert!(parse_flag_usize(&zero_value, "--budget")
            .unwrap_err()
            .contains("must be >= 1"));
    }

    #[test]
    fn sync_changeset_filename_filter_is_strict() {
        assert!(is_sync_changeset_file_name(
            "changeset-abc-20260419T101112000Z.json"
        ));
        assert!(is_sync_changeset_file_name("changeset-node-1.json"));
        assert!(!is_sync_changeset_file_name("changeset-node-1.txt"));
        assert!(!is_sync_changeset_file_name("metrics.json"));
    }

    #[test]
    fn resolve_sync_since_prefers_override_then_cursor_file() {
        let home_dir = temp_test_dir("sync_cursor_resolution");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let cursor_file = home_dir.join("cursor.txt");

        let override_since = "2026-04-19T00:00:00Z";
        assert_eq!(
            resolve_sync_since(Some(override_since), Some(&cursor_file)),
            Some(override_since.to_string())
        );
        assert_eq!(resolve_sync_since(None, Some(&cursor_file)), None);

        write_sync_cursor_file(&cursor_file, "2026-04-20T00:00:00Z").expect("write cursor");
        assert_eq!(
            resolve_sync_since(None, Some(&cursor_file)),
            Some("2026-04-20T00:00:00Z".to_string())
        );

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn atomic_text_write_replaces_existing_file() {
        let home_dir = temp_test_dir("atomic_text_write");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let path = home_dir.join("changeset.json");

        write_atomic_text_file(&path, "{\"old\":true}\n").expect("write initial file");
        write_atomic_text_file(&path, "{\"new\":true}\n").expect("replace file");

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"new\":true}\n");

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn general_import_metadata_allows_legacy_json_without_version() {
        let payload = json!({
            "memories": [{"text": "legacy memory"}],
            "decisions": []
        });

        validate_import_payload_metadata(&payload, ImportPayloadExpectation::GeneralJson)
            .expect("legacy JSON import remains compatible");
    }

    #[test]
    fn import_metadata_rejects_unsupported_version_marker() {
        let payload = json!({
            "version": 2,
            "memories": [],
            "decisions": []
        });

        let err = validate_import_payload_metadata(&payload, ImportPayloadExpectation::GeneralJson)
            .expect_err("unsupported version must fail");
        assert!(err.contains("unsupported version marker"));
    }

    #[test]
    fn sync_import_metadata_requires_changeset_mode_and_cursor() {
        let payload = json!({
            "version": 1,
            "memories": [],
            "decisions": [],
            "memories_count": 0,
            "decisions_count": 0
        });

        let err =
            validate_import_payload_metadata(&payload, ImportPayloadExpectation::SyncChangeset)
                .expect_err("sync import requires changeset metadata");
        assert!(err.contains("mode=\"changeset\""));

        let missing_cursor = json!({
            "version": 1,
            "mode": "changeset",
            "memories": [],
            "decisions": [],
            "memories_count": 0,
            "decisions_count": 0
        });
        let err = validate_import_payload_metadata(
            &missing_cursor,
            ImportPayloadExpectation::SyncChangeset,
        )
        .expect_err("sync import requires cursor marker");
        assert!(err.contains("missing cursor"));

        let missing_counts = json!({
            "version": 1,
            "mode": "changeset",
            "cursor": "2026-04-20T00:00:00Z",
            "memories": [],
            "decisions": []
        });
        let err = validate_import_payload_metadata(
            &missing_counts,
            ImportPayloadExpectation::SyncChangeset,
        )
        .expect_err("sync import requires count markers");
        assert!(err.contains("missing required memories_count marker"));
    }

    #[test]
    fn sync_import_metadata_rejects_count_marker_mismatch() {
        let payload = json!({
            "version": 1,
            "mode": "changeset",
            "cursor": "2026-04-20T00:00:00Z",
            "exported_at": "2026-04-20T00:00:00Z",
            "memories": [{"text": "one"}],
            "decisions": [],
            "memories_count": 2,
            "decisions_count": 0
        });

        let err =
            validate_import_payload_metadata(&payload, ImportPayloadExpectation::SyncChangeset)
                .expect_err("count marker mismatch must fail before import");
        assert!(err.contains("memories_count marker"));
    }

    #[test]
    fn cli_connection_applies_pending_migrations_for_fresh_db() {
        let home_dir = temp_test_dir("cli_connection_migrations");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let db_path = home_dir.join("cortex.db");

        let conn = open_cli_connection(&db_path).expect("open cli connection");
        let pending =
            db::pending_migration_versions(&conn).expect("read pending migration versions");
        assert!(
            pending.is_empty(),
            "fresh CLI DB should not have pending migrations: {pending:?}"
        );
        assert!(
            db::table_exists(&conn, "focus_sessions"),
            "fresh CLI DB should include migrated focus table"
        );

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn sync_watch_candidate_collection_skips_local_site_files() {
        let watch_dir = temp_test_dir("sync_watch_candidates");
        fs::create_dir_all(&watch_dir).expect("create watch dir");
        fs::write(
            watch_dir.join("changeset-local-site-20260419T100000000Z.json"),
            "{}",
        )
        .expect("write local file");
        fs::write(
            watch_dir.join("changeset-remote-site-20260419T100001000Z.json"),
            "{}",
        )
        .expect("write remote file");
        fs::write(watch_dir.join("notes.txt"), "ignore").expect("write noise file");

        let files = collect_sync_watch_import_candidates(&watch_dir, "local-site")
            .expect("collect sync watch files");
        assert_eq!(files.len(), 1);
        let name = files[0]
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        assert_eq!(name, "changeset-remote-site-20260419T100001000Z.json");

        let _ = std::fs::remove_dir_all(&watch_dir);
    }

    #[test]
    fn event_type_count_helpers_return_expected_rows() {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        conn.execute(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT,
                source_agent TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create events table");
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('recall_query', '{}', 'test')",
            [],
        )
        .unwrap();

        assert_eq!(event_type_count(&conn, "decision_stored"), 2);
        assert_eq!(event_type_count(&conn, "recall_query"), 1);
        assert_eq!(event_type_count(&conn, "missing"), 0);

        let top = top_event_type_counts(&conn, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0], ("decision_stored".to_string(), 2));
        assert_eq!(top[1], ("recall_query".to_string(), 1));
    }

    #[test]
    fn run_event_compaction_cleanup_dry_run_reports_preview_lines() {
        let home_dir = temp_test_dir("event_cleanup_dry_run");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let db_path = home_dir.join("cortex.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        db::configure(&conn).expect("configure db");
        conn.execute(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT,
                source_agent TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create events table");
        for _ in 0..5 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
                [],
            )
            .expect("insert event");
        }
        drop(conn);

        let lines =
            run_event_compaction_cleanup(&db_path, true, 2).expect("event cleanup dry run lines");
        assert!(
            lines.iter().any(|line| line.starts_with("EVENTS before:")),
            "missing before summary: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("dry-run only")),
            "missing dry-run hint: {lines:?}"
        );

        let _ = fs::remove_dir_all(&home_dir);
    }

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
    fn disallowed_startup_binary_path_blocks_runtime_wrappers_and_temp_paths() {
        let wrapper = PathBuf::from(
            "C:/repo/daemon-rs/target/debug/daemon-lifecycle-runtime/cortex-daemon-run.exe",
        );
        assert!(is_disallowed_startup_binary_path(&wrapper));

        let wrapper_name_only = PathBuf::from("C:/repo/cortex-daemon-run");
        assert!(is_disallowed_startup_binary_path(&wrapper_name_only));

        let binary_name = if cfg!(windows) {
            "cortex.exe"
        } else {
            "cortex"
        };
        let temp_candidate = std::env::temp_dir().join("cortex").join(binary_name);
        assert!(is_disallowed_startup_binary_path(&temp_candidate));

        let safe = PathBuf::from("C:/cortex-test/example/.cortex/bin/cortex.exe");
        assert!(!is_disallowed_startup_binary_path(&safe));
    }

    #[test]
    fn resolve_client_target_inputs_prefers_cli_over_env_values() {
        let (base_url, api_key, local_owner_mode) = resolve_client_target_inputs(
            Some("https://cli.example"),
            Some("ctx_cli"),
            Some("https://env.example"),
            Some("ctx_env"),
            "http://127.0.0.1:7437",
        );
        assert_eq!(base_url, "https://cli.example");
        assert_eq!(api_key.as_deref(), Some("ctx_cli"));
        assert!(!local_owner_mode);
    }

    #[test]
    fn resolve_client_target_inputs_uses_env_and_disables_local_owner_mode() {
        let (base_url, api_key, local_owner_mode) = resolve_client_target_inputs(
            None,
            None,
            Some("https://100.101.102.103:7437"),
            Some("ctx_remote"),
            "http://127.0.0.1:7437",
        );
        assert_eq!(base_url, "https://100.101.102.103:7437");
        assert_eq!(api_key.as_deref(), Some("ctx_remote"));
        assert!(!local_owner_mode);
    }

    #[test]
    fn validate_cli_options_rejects_missing_value_before_next_option() {
        let args = vec![
            "--url".to_string(),
            "--api-key".to_string(),
            "ctx_key".to_string(),
        ];
        let err =
            validate_cli_options(&args, &["--url", "--api-key"], &[]).expect_err("missing value");
        assert_eq!(err, "Missing value for --url");
    }

    #[test]
    fn validate_cli_options_rejects_unknown_options() {
        let args = vec![
            "--out".to_string(),
            "dump.json".to_string(),
            "--bogus".to_string(),
        ];
        let err = validate_cli_options(&args, &["--out"], &[]).expect_err("unknown option");
        assert_eq!(err, "Unknown option: --bogus");
    }

    #[test]
    fn validate_cli_options_allows_global_value_flags() {
        let args = vec![
            "--agent".to_string(),
            "codex".to_string(),
            "--home".to_string(),
            "C:/tmp/cortex-home".to_string(),
            "--port".to_string(),
            "9876".to_string(),
        ];
        validate_cli_options(&args, &["--agent"], &[]).expect("global flags should be valid");
    }

    #[test]
    fn parse_flag_usize_treats_option_token_as_missing_value() {
        let args = vec!["--budget".to_string(), "--json".to_string()];
        let err = parse_flag_usize(&args, "--budget").expect_err("missing numeric value");
        assert_eq!(err, "missing value for --budget");
    }

    #[test]
    fn local_daemon_base_url_uses_loopback_for_wildcard_bind() {
        let home_dir = temp_test_dir("bind_wildcard");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "0.0.0.0".to_string();
        assert_eq!(local_daemon_base_url(&paths), "http://127.0.0.1:7437");

        paths.bind = "100.64.0.12".to_string();
        assert_eq!(local_daemon_base_url(&paths), "http://100.64.0.12:7437");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn startup_preflight_rejects_non_canonical_health_payload() {
        let _env_guard = env_guard();
        let _bypass = ScopedEnvVar::set(SINGLE_DAEMON_TEST_BYPASS_ENV, "1");
        let home_dir = temp_test_dir("startup_preflight_noncanonical");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "127.0.0.1".to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("resolve listener addr").port();
        paths.port = port;
        // Isolate this preflight fixture from any live local daemon IPC endpoint.
        paths.ipc_endpoint = None;
        let server =
            spawn_preflight_response_server(listener, "404 Not Found", "text/plain", "nope".into());

        let err = run_preflight(&paths).unwrap_err();
        assert!(
            err.contains("non-canonical payload"),
            "unexpected preflight error: {err}"
        );

        server.join().expect("join response server");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn startup_preflight_rejects_different_cortex_runtime_identity() {
        let _env_guard = env_guard();
        let _bypass = ScopedEnvVar::set(SINGLE_DAEMON_TEST_BYPASS_ENV, "1");
        let home_dir = temp_test_dir("startup_preflight_wrong_runtime");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "127.0.0.1".to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("resolve listener addr").port();
        paths.port = port;
        // Isolate this preflight fixture from any live local daemon IPC endpoint.
        paths.ipc_endpoint = None;

        let payload = serde_json::json!({
            "status": "ready",
            "ready": true,
            "runtime": {
                "port": port,
                "token_path": "C:/other/cortex.token",
                "pid_path": "C:/other/cortex.pid",
                "db_path": "C:/other/cortex.db"
            },
            "stats": {
                "home": "C:/other"
            }
        })
        .to_string();
        let server =
            spawn_preflight_response_server(listener, "200 OK", "application/json", payload);

        let err = run_preflight(&paths).unwrap_err();
        assert!(
            err.contains("different Cortex runtime identity"),
            "unexpected preflight error: {err}"
        );

        server.join().expect("join response server");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn startup_preflight_rejects_canonical_ready_readiness_state() {
        let _env_guard = env_guard();
        let _bypass = ScopedEnvVar::set(SINGLE_DAEMON_TEST_BYPASS_ENV, "1");
        let home_dir = temp_test_dir("startup_preflight_ready_state");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);
        paths.bind = "127.0.0.1".to_string();

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
        let port = listener.local_addr().expect("resolve listener addr").port();
        paths.port = port;
        // Isolate this preflight fixture from any live local daemon IPC endpoint.
        paths.ipc_endpoint = None;
        let payload = serde_json::json!({
            "status": "ready",
            "ready": true,
            "runtime": {
                "port": port,
                "token_path": paths.token.display().to_string(),
                "pid_path": paths.pid.display().to_string(),
                "db_path": paths.db.display().to_string()
            },
            "stats": {
                "home": paths.home.display().to_string()
            }
        })
        .to_string();
        let server =
            spawn_preflight_response_server(listener, "200 OK", "application/json", payload);

        let err = run_preflight(&paths).unwrap_err();
        assert!(
            err.contains("canonical Cortex instance is already ready"),
            "unexpected preflight error: {err}"
        );

        server.join().expect("join response server");
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn is_local_client_base_url_accepts_configured_bind_host() {
        let home_dir = temp_test_dir("local_client_base");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let mut paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        paths.bind = "100.64.0.12".to_string();
        assert!(is_local_client_base_url("http://100.64.0.12:7437", &paths));
        assert!(!is_local_client_base_url("http://100.64.0.12:9999", &paths));
        assert!(!is_local_client_base_url(
            "https://example.com:7437",
            &paths
        ));

        paths.bind = "0.0.0.0".to_string();
        assert!(is_local_client_base_url("http://127.0.0.1:7437", &paths));

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn remote_target_without_api_key_is_rejected() {
        let home_dir = temp_test_dir("remote_target_auth_required");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let err =
            ensure_remote_target_has_api_key("https://100.64.0.12:7437", None, &paths).unwrap_err();
        assert!(err.contains("requires an API key"));

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn local_target_without_api_key_is_allowed() {
        let home_dir = temp_test_dir("local_target_no_key");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        assert!(ensure_remote_target_has_api_key("http://127.0.0.1:7437", None, &paths).is_ok());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn invalid_client_target_url_is_rejected_cleanly() {
        let home_dir = temp_test_dir("invalid_client_target_url");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths =
            auth::CortexPaths::resolve_with_overrides(Some(&home_str), None, Some(7437), None);

        let invalid_scheme =
            ensure_remote_target_has_api_key("ftp://example.com", Some("ctx_key"), &paths)
                .unwrap_err();
        assert!(invalid_scheme.contains("Unsupported Cortex target URL scheme"));

        let embedded_creds = ensure_remote_target_has_api_key(
            "https://user:pass@example.com",
            Some("ctx_key"),
            &paths,
        )
        .unwrap_err();
        assert!(embedded_creds.contains("must not include embedded credentials"));

        let query_url = ensure_remote_target_has_api_key(
            "https://example.com?debug=1",
            Some("ctx_key"),
            &paths,
        )
        .unwrap_err();
        assert!(query_url.contains("must not include query parameters"));

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn openapi_spec_version_matches_cargo_pkg_version() {
        let spec = fs::read_to_string(openapi_spec_path()).expect("read OpenAPI spec");
        assert!(
            spec.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))),
            "OpenAPI version must match Cargo package version"
        );
    }

    #[test]
    fn openapi_spec_declares_readiness_recall_explain_and_stats_paths() {
        let spec = fs::read_to_string(openapi_spec_path()).expect("read OpenAPI spec");
        assert!(spec.contains("/readiness:"), "missing /readiness in spec");
        assert!(
            spec.contains("/recall/explain:"),
            "missing /recall/explain in spec"
        );
        assert!(spec.contains("/stats:"), "missing /stats in spec");
        assert!(spec.contains("/boot/audit:"), "missing /boot/audit in spec");
    }

    #[test]
    fn openapi_spec_documents_export_pagination_and_import_auth_failures() {
        let spec = fs::read_to_string(openapi_spec_path()).expect("read OpenAPI spec");
        let export_block = spec
            .split("  /export:")
            .nth(1)
            .and_then(|rest| rest.split("  /import:").next())
            .expect("export block in OpenAPI spec");
        let import_block = spec
            .split("  /import:")
            .nth(1)
            .and_then(|rest| rest.split("components:").next())
            .expect("import block in OpenAPI spec");

        for expected in [
            "name: limit",
            "maximum: 5000",
            "name: offset",
            "name: memories_offset",
            "name: decisions_offset",
            "SQL export is available through the CLI export command",
            "'400':",
            "'401':",
            "'403':",
        ] {
            assert!(
                export_block.contains(expected),
                "missing export OpenAPI contract detail: {expected}"
            );
        }
        assert!(
            !export_block.contains("enum: [json, sql]"),
            "HTTP export must not advertise disabled SQL export as a success format"
        );
        assert!(
            !export_block.contains("text/plain:"),
            "HTTP export must not advertise SQL text/plain success content"
        );

        for expected in ["'400':", "'401':", "'403':"] {
            assert!(
                import_block.contains(expected),
                "missing import OpenAPI error response: {expected}"
            );
        }
    }
}
