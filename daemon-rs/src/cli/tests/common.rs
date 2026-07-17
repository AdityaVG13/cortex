// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use crate::cli::common::{resolve_client_target_inputs, validate_cli_options};
    use crate::cli::tests::support::*;
    use crate::cli::*;
    use crate::*;
    use std::fs;
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
    fn openapi_spec_version_matches_cargo_pkg_version() {
        let spec = fs::read_to_string(openapi_spec_path()).expect("read OpenAPI spec");
        assert!(
            spec.contains(&format!("version: {}", env!("CARGO_PKG_VERSION"))),
            "OpenAPI version must match Cargo package version"
        );
    }
}
