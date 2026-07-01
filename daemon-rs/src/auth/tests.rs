#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const LOCK_CHILD_MODE_ENV: &str = "CORTEX_LOCK_TEST_CHILD_MODE";
    const LOCK_CHILD_HOME_ENV: &str = "CORTEX_LOCK_TEST_CHILD_HOME";
    const LOCK_CHILD_READY_ENV: &str = "CORTEX_LOCK_TEST_CHILD_READY_FILE";
    const LOCK_CHILD_HOLD_MS_ENV: &str = "CORTEX_LOCK_TEST_CHILD_HOLD_MS";

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_auth_{name}_{unique}"))
    }

    #[test]
    fn verify_ctx_api_key_checksum_accepts_generated_keys() {
        let key = generate_ctx_api_key();
        assert!(verify_ctx_api_key_checksum(&key));
        assert!(!verify_ctx_api_key_checksum("ctx_short"));
        assert!(!verify_ctx_api_key_checksum(&format!("ctx_{}", "A".repeat(46))));
    }

    #[test]
    fn cleanup_stale_pid_lock_removes_dead_process_pid_only() {
        let home_dir = temp_test_dir("stale_pid");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, None, None);
        fs::write(&paths.pid, "999999").unwrap();
        fs::write(&paths.lock, "locked").unwrap();

        let cleaned = cleanup_stale_pid_lock(&paths);
        assert_eq!(cleaned, Some(999999));
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn cleanup_stale_pid_lock_removes_pid_outside_platform_range() {
        let home_dir = temp_test_dir("stale_pid_large");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(Some(&home_str), None, None, None);
        let stale_pid = u32::MAX;
        fs::write(&paths.pid, stale_pid.to_string()).unwrap();
        fs::write(&paths.lock, "locked").unwrap();

        let cleaned = cleanup_stale_pid_lock(&paths);
        assert_eq!(cleaned, Some(stale_pid));
        assert!(!paths.pid.exists());
        assert!(paths.lock.exists());

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn token_helpers_respect_overridden_home() {
        let home_dir = temp_test_dir("token_home");
        fs::create_dir_all(&home_dir).unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let token = try_generate_token_for(&paths).expect("token generation should succeed");

        assert_eq!(read_token_from(&paths).as_deref(), Some(token.as_str()));
        assert_eq!(paths.token, home_dir.join("cortex.token"));
        assert!(paths.token.exists());
        assert_eq!(paths.bind, "127.0.0.1");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[cfg(unix)]
    #[test]
    fn generated_token_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let home_dir = temp_test_dir("token_permissions");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let _ = try_generate_token_for(&paths).expect("token generation should succeed");

        let mode = fs::metadata(&paths.token).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[cfg(windows)]
    #[test]
    fn generated_token_file_has_protected_owner_acl() {
        use std::ptr::null_mut;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::Security::Authorization::{
            GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
            SE_FILE_OBJECT, TRUSTEE_IS_SID,
        };
        use windows_sys::Win32::Security::{
            EqualSid, GetSecurityDescriptorControl, ACL, DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        };

        let home_dir = temp_test_dir("token_windows_acl");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let _ = try_generate_token_for(&paths).expect("token generation should succeed");

        let wide_path = windows_path_to_wide(&paths.token);
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `wide_path` is null-terminated and output pointers are valid
        // for GetNamedSecurityInfoW to initialize.
        let result = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(
            result,
            ERROR_SUCCESS,
            "GetNamedSecurityInfoW failed: {}",
            win32_error(result)
        );
        let _descriptor_guard = LocalMemory(descriptor.cast());
        assert!(!dacl.is_null(), "secret file DACL should be present");

        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `descriptor` is owned by `_descriptor_guard` and remains
        // valid until the guard is dropped.
        let ok = unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) };
        assert_ne!(
            ok,
            0,
            "GetSecurityDescriptorControl failed: {}",
            io::Error::last_os_error()
        );
        assert_eq!(control & SE_DACL_PROTECTED, SE_DACL_PROTECTED);

        let mut count = 0u32;
        let mut entries: *mut EXPLICIT_ACCESS_W = null_mut();
        // SAFETY: `dacl` points into the security descriptor returned above and
        // remains valid while `_descriptor_guard` is alive.
        let result = unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) };
        assert_eq!(
            result,
            ERROR_SUCCESS,
            "GetExplicitEntriesFromAclW failed: {}",
            win32_error(result)
        );
        let _entries_guard = LocalMemory(entries.cast());
        assert_eq!(count, 1, "secret file should have one explicit ACE");

        // SAFETY: GetExplicitEntriesFromAclW returned at least one entry.
        let entry = unsafe { *entries };
        assert_eq!(entry.grfAccessMode, GRANT_ACCESS);
        assert_eq!(entry.Trustee.TrusteeForm, TRUSTEE_IS_SID);
        let expected_access = FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE;
        assert_eq!(
            entry.grfAccessPermissions & expected_access,
            expected_access
        );

        let current_user = current_user_sid().expect("read current user SID");
        let trustee_sid: PSID = entry.Trustee.ptstrName.cast();
        assert!(!trustee_sid.is_null(), "trustee SID should be present");
        // SAFETY: both SIDs come from Windows security APIs and were validated
        // before this comparison.
        assert_ne!(unsafe { EqualSid(trustee_sid, current_user.sid) }, 0);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn try_generate_token_for_reports_directory_failures() {
        let home_dir = temp_test_dir("token_home_is_file");
        if let Some(parent) = home_dir.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&home_dir, "not a directory").unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let err = try_generate_token_for(&paths).expect_err("token generation should fail");
        assert!(
            err.contains("cannot create token directory"),
            "unexpected error: {err}"
        );
        assert!(!paths.token.exists());

        let _ = fs::remove_file(&home_dir);
    }

    #[test]
    fn resolve_bind_prefers_cli_then_env_then_default() {
        assert_eq!(resolve_bind(Some("0.0.0.0"), Some("10.10.0.5")), "0.0.0.0");
        assert_eq!(resolve_bind(Some("   "), Some("10.10.0.5")), "10.10.0.5");
        assert_eq!(resolve_bind(None, Some("   ")), "127.0.0.1");
    }

    #[test]
    fn resolve_from_args_parses_bind_flag() {
        let home_dir = temp_test_dir("bind_flag");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let args = vec![
            "cortex".to_string(),
            "serve".to_string(),
            "--home".to_string(),
            home_str,
            "--bind".to_string(),
            "0.0.0.0".to_string(),
        ];
        let paths = CortexPaths::resolve_from_args(&args);
        assert_eq!(paths.bind, "0.0.0.0");

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn acquire_global_daemon_lock_rejects_duplicate_instances() {
        let _guard = env_guard();
        let global_home = temp_test_dir("global_lock");
        fs::create_dir_all(&global_home).unwrap();
        let lock_path = global_home.join(CORTEX_GLOBAL_LOCK_NAME);

        let first =
            acquire_global_daemon_lock_at(&lock_path).expect("first global lock should succeed");
        let err =
            acquire_global_daemon_lock_at(&lock_path).expect_err("second global lock should fail");
        assert!(err.contains("another cortex instance"));

        drop(first);
        let second = acquire_global_daemon_lock_at(&lock_path)
            .expect("lock should be reacquired after release");
        drop(second);

        let _ = fs::remove_dir_all(&global_home);
    }

    #[test]
    fn acquire_global_daemon_lock_cross_process_child() {
        if std::env::var(LOCK_CHILD_MODE_ENV).ok().as_deref() != Some("1") {
            return;
        }

        let global_home = std::env::var(LOCK_CHILD_HOME_ENV).expect("child global lock home env");
        let lock_path = PathBuf::from(global_home).join(CORTEX_GLOBAL_LOCK_NAME);
        let ready_file =
            PathBuf::from(std::env::var(LOCK_CHILD_READY_ENV).expect("child ready file env"));
        let hold_ms = std::env::var(LOCK_CHILD_HOLD_MS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1500);

        let lock = acquire_global_daemon_lock_at(&lock_path).expect("child acquires global lock");
        fs::write(&ready_file, b"locked").expect("write ready marker");
        std::thread::sleep(Duration::from_millis(hold_ms));
        drop(lock);
    }

    #[test]
    fn acquire_global_daemon_lock_rejects_cross_process_duplicate_instances() {
        let _guard = env_guard();
        let global_home = temp_test_dir("global_lock_cross_process");
        fs::create_dir_all(&global_home).unwrap();
        let global_home_str = global_home.to_string_lossy().to_string();
        let lock_path = global_home.join(CORTEX_GLOBAL_LOCK_NAME);
        let ready_file = global_home.join("cross-process-ready");
        let hold_ms = 2000_u64;

        let current_exe = std::env::current_exe().expect("resolve current test binary path");
        let mut child = Command::new(current_exe)
            .arg("--exact")
            .arg("auth::tests::acquire_global_daemon_lock_cross_process_child")
            .arg("--nocapture")
            .env(LOCK_CHILD_MODE_ENV, "1")
            .env(LOCK_CHILD_HOME_ENV, &global_home_str)
            .env(
                LOCK_CHILD_READY_ENV,
                ready_file.to_string_lossy().to_string(),
            )
            .env(LOCK_CHILD_HOLD_MS_ENV, hold_ms.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn lock-holder child");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready_file.exists() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("child lock helper never reported readiness");
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        let duplicate = acquire_global_daemon_lock_at(&lock_path)
            .expect_err("cross-process duplicate must fail");
        assert!(duplicate.contains("another cortex instance"));

        let status = child.wait().expect("wait on lock-holder child");
        assert!(status.success(), "child process should exit successfully");

        let after_release = acquire_global_daemon_lock_at(&lock_path)
            .expect("lock should succeed after child exit");
        drop(after_release);

        let _ = fs::remove_dir_all(&global_home);
    }
}
