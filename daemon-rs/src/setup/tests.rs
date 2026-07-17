mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_setup_{name}_{unique}"))
    }

    #[test]
    fn merge_mcp_config_preserves_explicit_agent_args() {
        let root = temp_test_dir("json_merge");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("mcp.json");

        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();

        let config: Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["cortex"]["args"],
            serde_json::json!(["mcp", "--agent", "cursor"])
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_mcp_config_is_idempotent_for_existing_registration() {
        let root = temp_test_dir("json_merge_idempotent");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("mcp.json");

        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();
        let first = fs::read_to_string(&config_path).unwrap();
        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();
        let second = fs::read_to_string(&config_path).unwrap();

        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_toml_config_writes_mcp_servers_without_clobbering_other_values() {
        let root = temp_test_dir("toml_merge");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.toml");
        fs::write(
            &config_path,
            r#"
title = "Codex"

[other]
enabled = true
"#,
        )
        .unwrap();

        merge_toml_config(&config_path, "/tmp/cortex", "codex").unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            config
                .get("mcp_servers")
                .and_then(|value| value.get("cortex"))
                .and_then(|value| value.get("args"))
                .and_then(|value| value.as_array())
                .map(|values| values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()),
            Some(vec!["mcp", "--agent", "codex"])
        );
        assert_eq!(
            config
                .get("other")
                .and_then(|value| value.get("enabled"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_detection_helpers_accept_existing_parent_directories() {
        let root = temp_test_dir("path_detection");
        let claude = root.join(".claude").join("settings.json");
        let codex = root.join(".codex").join("config.toml");
        let cursor = root.join(".cursor").join("mcp.json");

        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::create_dir_all(cursor.parent().unwrap()).unwrap();

        assert!(find_existing_config(claude).is_some());
        assert!(find_existing_config(codex).is_some());
        assert!(find_existing_config(cursor).is_some());
        assert!(find_first_config_path(vec![
            root.join(".missing"),
            root.join(".cursor").join("mcp.json"),
        ])
        .is_some());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn persist_team_owner_token_reports_directory_failures() {
        let home_dir = temp_test_dir("team_token_home_is_file");
        if let Some(parent) = home_dir.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&home_dir, "not a directory").unwrap();

        let home_str = home_dir.to_string_lossy().to_string();
        let paths = auth::CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54967),
            Some("127.0.0.1"),
        );

        let err = persist_team_owner_token(&paths, "ctx_test_owner_key")
            .expect_err("team owner token persistence should fail");
        assert!(
            err.contains("cannot create token directory"),
            "unexpected error: {err}"
        );
        assert!(!paths.token.exists());

        let _ = fs::remove_file(&home_dir);
    }
}
