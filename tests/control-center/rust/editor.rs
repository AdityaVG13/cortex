use super::*;
use std::fs;
use std::path::Path;

#[test]
fn editor_registration_uses_explicit_agent_args() {
    let home = Path::new("C:/cortex-test/testuser");
    let targets = editor_targets(home);
    let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
    let claude = targets.iter().find(|target| target.id == "claude-code").unwrap();

    assert_eq!(editor_args(cursor), ["mcp", "--agent", "cursor"]);
    assert_eq!(editor_args(claude), ["mcp", "--agent", "claude"]);
}

#[test]
fn editor_registration_includes_attach_only_env_contract() {
    let home = Path::new("C:/cortex-test/testuser");
    let targets = editor_targets(home);
    let codex = targets.iter().find(|target| target.id == "codex").unwrap();

    let registration = cortex_mcp_registration(codex, "C:/cortex-test/testuser/.cortex/bin/cortex.exe");
    let cortex_entry = registration.as_object().expect("registration should be an object");
    let args = cortex_entry.get("args").and_then(|value| value.as_array()).expect("args should exist");
    assert_eq!(args.iter().filter_map(|value| value.as_str()).collect::<Vec<_>>(), vec!["mcp", "--agent", "codex"]);

    let env = cortex_entry.get("env").and_then(|value| value.as_object()).expect("env should exist");
    assert_eq!(env.get("CORTEX_APP_REQUIRED").and_then(|value| value.as_str()), Some("1"));
    assert_eq!(env.get("CORTEX_DAEMON_OWNER_LOCAL_SPAWN").and_then(|value| value.as_str()), Some("0"));
    assert_eq!(env.get("CORTEX_APP_CLIENT").and_then(|value| value.as_str()), Some("codex"));
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
        [("env".to_string(), toml::Value::Table([("CORTEX_APP_REQUIRED".to_string(), toml::Value::String("1".to_string()))].into_iter().collect()))]
            .into_iter()
            .collect(),
    );
    assert!(!toml_env_match(&toml_missing_env, codex));

    let toml_ok = toml::Value::Table(
        [(
            "env".to_string(),
            toml::Value::Table(
                [
                    ("CORTEX_APP_REQUIRED".to_string(), toml::Value::String("1".to_string())),
                    ("CORTEX_DAEMON_OWNER_LOCAL_SPAWN".to_string(), toml::Value::String("0".to_string())),
                    ("CORTEX_APP_CLIENT".to_string(), toml::Value::String("codex".to_string())),
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

/// Exclusive home that is removed even if the test panics.
struct TestTempDir(std::path::PathBuf);

impl TestTempDir {
    fn new(prefix: &str) -> Self {
        let mut n = 0u32;
        loop {
            let path = std::env::temp_dir().join(format!(
                "{prefix}-{}-{}-{n}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    n = n.saturating_add(1);
                    continue;
                }
                Err(err) => panic!("create temp dir {}: {err}", path.display()),
            }
        }
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn gemini_prefers_nested_mcp_config_when_present() {
    let temp_root = TestTempDir::new("cortex_control_center_editor_test");
    let gemini_nested = temp_root.0.join(".gemini").join("settings").join("mcp.json");
    let gemini_legacy = temp_root.0.join(".gemini").join("settings.json");
    fs::create_dir_all(gemini_nested.parent().unwrap()).expect("create gemini settings dir");
    fs::write(&gemini_nested, "{}").expect("write nested gemini config");
    fs::write(&gemini_legacy, "{}").expect("write legacy gemini config");

    let targets = editor_targets(&temp_root.0);
    let gemini = targets.iter().find(|target| target.id == "gemini").unwrap();

    assert_eq!(editor_config_path(gemini), gemini_nested);
}

#[test]
fn claude_desktop_uses_platform_specific_config_path() {
    let home = Path::new("/tmp/cortex-home");
    let expected = if cfg!(windows) {
        home.join("AppData").join("Roaming").join("Claude").join("claude_desktop_config.json")
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json")
    } else {
        home.join(".config").join("Claude").join("claude_desktop_config.json")
    };

    let targets = editor_targets(home);
    let claude_desktop = targets.iter().find(|target| target.id == "claude-desktop").unwrap();

    assert_eq!(claude_desktop.config_path, expected);
}

#[test]
fn preferred_editor_command_path_prefers_installed_binary() {
    let temp_root = TestTempDir::new("cortex_control_center_editor_cmd");
    let installed_dir = temp_root.0.join(".cortex").join("bin");
    fs::create_dir_all(&installed_dir).expect("bin dir");
    let binary_name = if cfg!(windows) { "cortex.exe" } else { "cortex" };
    let installed = installed_dir.join(binary_name);
    fs::write(&installed, b"stub").expect("write installed");
    let found = temp_root.0.join("sidecar").join(binary_name);
    let preferred = preferred_editor_command_path(&temp_root.0, Some(found));
    assert_eq!(preferred.as_deref(), Some(installed.as_path()));
}

#[test]
fn already_registered_json_editor_does_not_rewrite_config() {
    let temp_root = TestTempDir::new("cortex_control_center_editor_skip");
    let home = &temp_root.0;
    fs::create_dir_all(home.join(".cursor")).expect("cursor dir");
    let exe = if cfg!(windows) { r"C:\cortex-test\bin\cortex.exe" } else { "/opt/cortex/bin/cortex" };
    let compact = serde_json::json!({
        "keep": "me",
        "mcpServers": {
            "cortex": {
                "command": exe,
                "args": ["mcp", "--agent", "cursor"],
                "env": {
                    "CORTEX_APP_REQUIRED": "1",
                    "CORTEX_DAEMON_OWNER_LOCAL_SPAWN": "0",
                    "CORTEX_APP_CLIENT": "cursor"
                }
            }
        }
    });
    let compact_text = serde_json::to_string(&compact).expect("serialize compact config");
    let config_path = home.join(".cursor").join("mcp.json");
    fs::write(&config_path, &compact_text).expect("write compact config");
    let targets = editor_targets(home);
    let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
    let result = register_editor(cursor, exe).expect("register");
    assert!(result.registered);
    let after = fs::read_to_string(&config_path).expect("reread config");
    assert_eq!(after, compact_text, "already-configured editor config must not be rewritten");
}

#[test]
fn write_config_atomic_replaces_existing_file() {
    let temp_root = TestTempDir::new("cortex_control_center_editor_atomic");
    let path = temp_root.0.join("mcp.json");
    fs::write(&path, "{\"keep\":\"old\"}").expect("seed config");
    write_config_atomic(&path, "{\"keep\":\"new\"}").expect("atomic replace");
    assert_eq!(fs::read_to_string(&path).expect("reread"), "{\"keep\":\"new\"}");
    assert!(temp_root.0.join("mcp.json").exists(), "replace must not leave the destination missing");
    let leftovers: Vec<_> = fs::read_dir(&temp_root.0)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.file_name().map(|name| name.to_string_lossy().contains(".tmp")).unwrap_or(false))
        .collect();
    assert!(leftovers.is_empty(), "atomic write must not leave tmp files: {leftovers:?}");
}

#[test]
fn empty_and_bom_json_configs_register_as_objects() {
    let temp_root = TestTempDir::new("cortex_control_center_editor_empty_json");
    let home = &temp_root.0;
    fs::create_dir_all(home.join(".cursor")).expect("cursor dir");
    let config_path = home.join(".cursor").join("mcp.json");
    fs::write(&config_path, "\u{feff}").expect("bom-only config");
    let exe = if cfg!(windows) { r"C:\cortex-test\bin\cortex.exe" } else { "/opt/cortex/bin/cortex" };
    let targets = editor_targets(home);
    let cursor = targets.iter().find(|target| target.id == "cursor").unwrap();
    let result = register_editor(cursor, exe).expect("register bom empty");
    assert!(result.registered, "{}", result.message);
    let after: serde_json::Value = serde_json::from_str(&fs::read_to_string(&config_path).expect("reread")).expect("json");
    assert!(after.get("mcpServers").and_then(|value| value.get("cortex")).is_some(), "{after}");
}
