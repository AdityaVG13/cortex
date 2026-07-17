use super::*;
use std::fs;
use std::path::Path;


    #[test]
    fn editor_registration_uses_explicit_agent_args() {
        let home = Path::new("C:/cortex-test/testuser");
        let targets = editor_targets(home);
        let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
        let claude = targets
            .iter()
            .find(|target| target.id == "claude-code")
            .unwrap();

        assert_eq!(editor_args(cursor), ["mcp", "--agent", "cursor"]);
        assert_eq!(editor_args(claude), ["mcp", "--agent", "claude"]);
    }


    #[test]
    fn editor_registration_includes_attach_only_env_contract() {
        let home = Path::new("C:/cortex-test/testuser");
        let targets = editor_targets(home);
        let codex = targets.iter().find(|target| target.id == "codex").unwrap();

        let registration =
            cortex_mcp_registration(codex, "C:/cortex-test/testuser/.cortex/bin/cortex.exe");
        let cortex_entry = registration
            .as_object()
            .expect("registration should be an object");
        let args = cortex_entry
            .get("args")
            .and_then(|value| value.as_array())
            .expect("args should exist");
        assert_eq!(
            args.iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec!["mcp", "--agent", "codex"]
        );

        let env = cortex_entry
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_eq!(
            env.get("CORTEX_APP_REQUIRED")
                .and_then(|value| value.as_str()),
            Some("1")
        );
        assert_eq!(
            env.get("CORTEX_DAEMON_OWNER_LOCAL_SPAWN")
                .and_then(|value| value.as_str()),
            Some("0")
        );
        assert_eq!(
            env.get("CORTEX_APP_CLIENT")
                .and_then(|value| value.as_str()),
            Some("codex")
        );
    }


    #[test]
    fn registration_matchers_require_attach_only_env_contract() {
        let home = Path::new("C:/cortex-test/testuser");
        let targets = editor_targets(home);
        let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
        let codex = targets.iter().find(|target| target.id == "codex").unwrap();

        let json_missing_env = serde_json::json!({
            "env": {
                "CORTEX_APP_REQUIRED": "1"
            }
        });
        assert!(!json_env_match(&json_missing_env, cursor));

        let json_ok = serde_json::json!({
            "env": {
                "CORTEX_APP_REQUIRED": "1",
                "CORTEX_DAEMON_OWNER_LOCAL_SPAWN": "0",
                "CORTEX_APP_CLIENT": "cursor"
            }
        });
        assert!(json_env_match(&json_ok, cursor));

        let toml_missing_env = toml::Value::Table(
            [(
                "env".to_string(),
                toml::Value::Table(
                    [(
                        "CORTEX_APP_REQUIRED".to_string(),
                        toml::Value::String("1".to_string()),
                    )]
                    .into_iter()
                    .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        );
        assert!(!toml_env_match(&toml_missing_env, codex));

        let toml_ok = toml::Value::Table(
            [(
                "env".to_string(),
                toml::Value::Table(
                    [
                        (
                            "CORTEX_APP_REQUIRED".to_string(),
                            toml::Value::String("1".to_string()),
                        ),
                        (
                            "CORTEX_DAEMON_OWNER_LOCAL_SPAWN".to_string(),
                            toml::Value::String("0".to_string()),
                        ),
                        (
                            "CORTEX_APP_CLIENT".to_string(),
                            toml::Value::String("codex".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            )]
            .into_iter()
            .collect(),
        );
        assert!(toml_env_match(&toml_ok, codex));
    }


    #[test]
    fn gemini_prefers_nested_mcp_config_when_present() {
        let temp_root = std::env::temp_dir().join(format!(
            "cortex_control_center_editor_test_{}",
            std::process::id()
        ));
        let gemini_nested = temp_root.join(".gemini").join("settings").join("mcp.json");
        let gemini_legacy = temp_root.join(".gemini").join("settings.json");
        fs::create_dir_all(gemini_nested.parent().unwrap()).expect("create gemini settings dir");
        fs::write(&gemini_nested, "{}").expect("write nested gemini config");
        fs::write(&gemini_legacy, "{}").expect("write legacy gemini config");

        let targets = editor_targets(&temp_root);
        let gemini = targets.iter().find(|target| target.id == "gemini").unwrap();

        assert_eq!(editor_config_path(gemini), gemini_nested);

        let _ = fs::remove_file(gemini_nested);
        let _ = fs::remove_file(gemini_legacy);
        let _ = fs::remove_dir_all(temp_root.join(".gemini"));
        let _ = fs::remove_dir(temp_root);
    }


    #[test]
    fn claude_desktop_uses_platform_specific_config_path() {
        let home = Path::new("/tmp/cortex-home");
        let expected = if cfg!(windows) {
            home.join("AppData")
                .join("Roaming")
                .join("Claude")
                .join("claude_desktop_config.json")
        } else if cfg!(target_os = "macos") {
            home.join("Library")
                .join("Application Support")
                .join("Claude")
                .join("claude_desktop_config.json")
        } else {
            home.join(".config")
                .join("Claude")
                .join("claude_desktop_config.json")
        };

        let targets = editor_targets(home);
        let claude_desktop = targets
            .iter()
            .find(|target| target.id == "claude-desktop")
            .unwrap();

        assert_eq!(claude_desktop.config_path, expected);
    }
