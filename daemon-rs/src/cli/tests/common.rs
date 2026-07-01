// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
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
}
