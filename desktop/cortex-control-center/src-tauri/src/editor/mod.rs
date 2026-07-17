use crate::daemon::paths::{cortex_home, find_cortex_binary};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

// ─── MCP Auto-Registration ──────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorDetection {
    id: String,
    name: String,
    detected: bool,
    registered: bool,
    config_path: Option<String>,
    command_path: Option<String>,
    message: String,
}

#[derive(Clone, Copy)]
enum EditorConfigKind {
    Json,
    Toml,
}

#[derive(Clone)]
struct EditorTarget {
    id: &'static str,
    name: &'static str,
    agent_name: &'static str,
    config_kind: EditorConfigKind,
    config_path: PathBuf,
    fallback_config_paths: Vec<PathBuf>,
}

pub fn cortex_exe_path() -> Option<PathBuf> {
    find_cortex_binary()
}

pub fn editor_args(target: &EditorTarget) -> [&'static str; 3] {
    ["mcp", "--agent", target.agent_name]
}

pub fn editor_env_pairs(_target: &EditorTarget) -> [(&'static str, &'static str); 2] {
    [("CORTEX_APP_REQUIRED", "1"), ("CORTEX_DAEMON_OWNER_LOCAL_SPAWN", "0")]
}

fn editor_path_detected(path: &Path) -> bool {
    path.exists() || path.parent().map(|parent| parent.exists()).unwrap_or(false)
}

pub fn editor_config_path(target: &EditorTarget) -> PathBuf {
    if target.config_path.exists() {
        return target.config_path.clone();
    }
    for path in &target.fallback_config_paths {
        if path.exists() {
            return path.clone();
        }
    }
    target.config_path.clone()
}

pub fn cortex_mcp_registration(target: &EditorTarget, cortex_exe: &str) -> serde_json::Value {
    let mut env = serde_json::Map::new();
    for (key, value) in editor_env_pairs(target) {
        env.insert(key.to_string(), serde_json::Value::String(value.to_string()));
    }
    env.insert("CORTEX_APP_CLIENT".to_string(), serde_json::Value::String(target.agent_name.to_string()));
    serde_json::json!({
      "command": cortex_exe,
      "args": editor_args(target),
      "env": env
    })
}

fn claude_desktop_config_path(home: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        home.join("AppData").join("Roaming").join("Claude").join("claude_desktop_config.json")
    }

    #[cfg(target_os = "macos")]
    {
        home.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json")
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        home.join(".config").join("Claude").join("claude_desktop_config.json")
    }
}

pub fn editor_targets(home: &Path) -> Vec<EditorTarget> {
    vec![
        EditorTarget {
            id: "claude-code",
            name: "Claude Code",
            agent_name: "claude",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".claude").join("settings.json"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "claude-desktop",
            name: "Claude Desktop",
            agent_name: "claude",
            config_kind: EditorConfigKind::Json,
            config_path: claude_desktop_config_path(home),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "cursor",
            name: "Cursor",
            agent_name: "cursor",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".cursor").join("mcp.json"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "codex",
            name: "Codex",
            agent_name: "codex",
            config_kind: EditorConfigKind::Toml,
            config_path: home.join(".codex").join("config.toml"),
            fallback_config_paths: Vec::new(),
        },
        EditorTarget {
            id: "gemini",
            name: "Gemini CLI",
            agent_name: "gemini",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".gemini").join("settings").join("mcp.json"),
            fallback_config_paths: vec![home.join(".gemini").join("settings.json")],
        },
        EditorTarget {
            id: "droid",
            name: "Droid",
            agent_name: "droid",
            config_kind: EditorConfigKind::Json,
            config_path: home.join(".factory").join("mcp.json"),
            fallback_config_paths: Vec::new(),
        },
    ]
}

fn editor_detected(target: &EditorTarget) -> bool {
    editor_path_detected(&target.config_path) || target.fallback_config_paths.iter().any(|path| editor_path_detected(path))
}

fn editor_command_path(cortex_exe: Option<&str>) -> Option<String> {
    cortex_exe.map(|path| path.to_string())
}

pub fn editor_detection(target: &EditorTarget, detected: bool, registered: bool, cortex_exe: Option<&str>, message: String) -> EditorDetection {
    let config_path = editor_config_path(target);
    EditorDetection {
        id: target.id.into(),
        name: target.name.into(),
        detected,
        registered,
        config_path: Some(config_path.display().to_string()),
        command_path: editor_command_path(cortex_exe),
        message,
    }
}

fn json_registration_for(target: &EditorTarget, cortex_exe: &str) -> serde_json::Value {
    let mut registration = cortex_mcp_registration(target, cortex_exe);
    if let Some(object) = registration.as_object_mut() {
        match target.id {
            "gemini" => {
                object.insert("trust".into(), serde_json::Value::Bool(true));
            }
            "droid" => {
                object.insert("disabled".into(), serde_json::Value::Bool(false));
            }
            _ => {}
        }
    }
    registration
}

fn read_json_config(config_path: &Path) -> Result<serde_json::Value, String> {
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        Ok(serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({})))
    } else {
        Ok(serde_json::json!({}))
    }
}

fn read_toml_config(config_path: &Path) -> Result<toml::Value, String> {
    if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| e.to_string())?;
        Ok(toml::from_str(&content).unwrap_or_else(|_| toml::Value::Table(Default::default())))
    } else {
        Ok(toml::Value::Table(Default::default()))
    }
}

fn json_args_match(config: &serde_json::Value, expected_args: &[&str]) -> bool {
    config
        .get("args")
        .and_then(|value| value.as_array())
        .map(|args| args.len() == expected_args.len() && args.iter().zip(expected_args.iter()).all(|(value, expected)| value.as_str() == Some(*expected)))
        .unwrap_or(false)
}

pub fn json_env_match(config: &serde_json::Value, target: &EditorTarget) -> bool {
    let Some(env) = config.get("env").and_then(|value| value.as_object()) else {
        return false;
    };
    let policy_matches = editor_env_pairs(target).iter().all(|(key, expected)| env.get(*key).and_then(|value| value.as_str()) == Some(*expected));
    let client_match = env.get("CORTEX_APP_CLIENT").and_then(|value| value.as_str()) == Some(target.agent_name);
    policy_matches && client_match
}

fn toml_args_match(config: &toml::Value, expected_args: &[&str]) -> bool {
    config
        .get("args")
        .and_then(|value| value.as_array())
        .map(|args| args.len() == expected_args.len() && args.iter().zip(expected_args.iter()).all(|(value, expected)| value.as_str() == Some(*expected)))
        .unwrap_or(false)
}

pub fn toml_env_match(config: &toml::Value, target: &EditorTarget) -> bool {
    let Some(env) = config.get("env").and_then(|value| value.as_table()) else {
        return false;
    };
    let policy_matches = editor_env_pairs(target).iter().all(|(key, expected)| env.get(*key).and_then(|value| value.as_str()) == Some(*expected));
    let client_match = env.get("CORTEX_APP_CLIENT").and_then(|value| value.as_str()) == Some(target.agent_name);
    policy_matches && client_match
}

fn is_editor_registered_at_path(target: &EditorTarget, cortex_exe: &str, config_path: &Path) -> Result<bool, String> {
    if !config_path.exists() {
        return Ok(false);
    }
    let expected_args = editor_args(target);

    match target.config_kind {
        EditorConfigKind::Json => {
            let config = read_json_config(config_path)?;
            Ok(config
                .get("mcpServers")
                .and_then(|value| value.get("cortex"))
                .map(|value| {
                    value.get("command").and_then(|command| command.as_str()).map(|command| command == cortex_exe).unwrap_or(false)
                        && json_args_match(value, &expected_args)
                        && json_env_match(value, target)
                })
                .unwrap_or(false))
        }
        EditorConfigKind::Toml => {
            let config = read_toml_config(config_path)?;
            Ok(config
                .get("mcp_servers")
                .and_then(|value| value.get("cortex"))
                .map(|value| {
                    value.get("command").and_then(|command| command.as_str()).map(|command| command == cortex_exe).unwrap_or(false)
                        && toml_args_match(value, &expected_args)
                        && toml_env_match(value, target)
                })
                .unwrap_or(false))
        }
    }
}

fn is_editor_registered(target: &EditorTarget, cortex_exe: &str) -> Result<bool, String> {
    let config_path = editor_config_path(target);
    is_editor_registered_at_path(target, cortex_exe, &config_path)
}

fn register_json_editor(target: &EditorTarget, cortex_exe: &str) -> Result<EditorDetection, String> {
    let config_path = editor_config_path(target);
    if !editor_detected(target) {
        return Ok(editor_detection(target, false, false, Some(cortex_exe), format!("{} not detected ({})", target.name, config_path.display())));
    }

    let mut config = read_json_config(&config_path)?;
    let servers = config
        .as_object_mut()
        .ok_or_else(|| format!("Invalid JSON config format in {}", config_path.display()))?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let action = if is_editor_registered_at_path(target, cortex_exe, &config_path)? {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };

    servers
        .as_object_mut()
        .ok_or_else(|| format!("Invalid mcpServers format in {}", config_path.display()))?
        .insert("cortex".into(), json_registration_for(target, cortex_exe));

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&config_path, out).map_err(|e| e.to_string())?;

    Ok(editor_detection(target, true, true, Some(cortex_exe), format!("{action} in {}", config_path.display())))
}

fn register_toml_editor(target: &EditorTarget, cortex_exe: &str) -> Result<EditorDetection, String> {
    let config_path = editor_config_path(target);
    if !editor_detected(target) {
        return Ok(editor_detection(target, false, false, Some(cortex_exe), format!("{} not detected ({})", target.name, config_path.display())));
    }

    let mut config = read_toml_config(&config_path)?;
    let root = config.as_table_mut().ok_or_else(|| format!("Invalid TOML config format in {}", config_path.display()))?;
    let servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| format!("Invalid [mcp_servers] format in {}", config_path.display()))?;

    let action = if is_editor_registered_at_path(target, cortex_exe, &config_path)? {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };
    let args = editor_args(target);

    let mut server = toml::map::Map::new();
    server.insert("command".into(), toml::Value::String(cortex_exe.to_string()));
    server.insert("args".into(), toml::Value::Array(args.into_iter().map(|value| toml::Value::String(value.to_string())).collect()));
    let mut env_table = toml::map::Map::new();
    for (key, value) in editor_env_pairs(target) {
        env_table.insert(key.into(), toml::Value::String(value.to_string()));
    }
    env_table.insert("CORTEX_APP_CLIENT".into(), toml::Value::String(target.agent_name.to_string()));
    server.insert("env".into(), toml::Value::Table(env_table));
    servers.insert("cortex".into(), toml::Value::Table(server));

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let out = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&config_path, out).map_err(|e| e.to_string())?;

    Ok(editor_detection(target, true, true, Some(cortex_exe), format!("{action} in {}", config_path.display())))
}

pub fn register_editor(target: &EditorTarget, cortex_exe: &str) -> Result<EditorDetection, String> {
    match target.config_kind {
        EditorConfigKind::Json => register_json_editor(target, cortex_exe),
        EditorConfigKind::Toml => register_toml_editor(target, cortex_exe),
    }
}

pub fn ensure_editor_binary_path() -> Result<PathBuf, String> {
    use crate::daemon::paths::{copy_if_changed, installed_plugin_binary_path, is_disallowed_daemon_binary_path};
    let home = cortex_home()?;
    let source = find_cortex_binary().ok_or_else(|| {
        "Could not find cortex binary in sidecar directory, ~/.cortex/bin/, or ~/cortex/daemon-rs/{target-control-center-dev,target-control-center-release,target}/{debug,release}/".to_string()
    })?;
    if is_disallowed_daemon_binary_path(&source) {
        return Err(format!("Refusing disallowed daemon binary source path for editor registration: {}", source.display()));
    }
    let installed = installed_plugin_binary_path(&home);

    if source != installed {
        if let Some(parent) = installed.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        copy_if_changed(&source, &installed)?;
    }

    Ok(installed)
}

pub fn setup_editors(editor_ids: Option<Vec<String>>) -> Result<Vec<EditorDetection>, String> {
    let cortex_exe = ensure_editor_binary_path()?;
    let exe_str = cortex_exe.to_string_lossy().to_string();
    let home = cortex_home()?;
    let targets = editor_targets(&home);
    let requested_ids = editor_ids.unwrap_or_default().into_iter().collect::<HashSet<_>>();
    let use_selection = !requested_ids.is_empty();
    let mut results = Vec::new();

    for target in targets {
        let detected = editor_detected(&target);
        if use_selection && !requested_ids.contains(target.id) {
            continue;
        }
        if !use_selection && !detected {
            continue;
        }

        match register_editor(&target, &exe_str) {
            Ok(result) => results.push(result),
            Err(err) => results.push(editor_detection(&target, detected, false, Some(&exe_str), format!("Configuration failed: {err}"))),
        }
    }

    Ok(results)
}

pub fn detect_editors() -> Result<Vec<EditorDetection>, String> {
    let home = cortex_home()?;
    let cortex_exe = cortex_exe_path();
    let cortex_exe_string = cortex_exe.as_ref().map(|path| path.to_string_lossy().to_string());
    let mut results = Vec::new();

    for target in editor_targets(&home) {
        let detected = editor_detected(&target);
        let registered = if let Some(ref exe) = cortex_exe_string { is_editor_registered(&target, exe).unwrap_or(false) } else { false };
        let message = if cortex_exe_string.is_none() {
            "cortex.exe not found -- build daemon first".into()
        } else if registered {
            format!("Configured in {}", target.config_path.display())
        } else if detected {
            format!("Detected at {}", target.config_path.display())
        } else {
            format!("Not detected ({})", target.config_path.display())
        };

        results.push(editor_detection(&target, detected, registered, cortex_exe_string.as_deref(), message));
    }

    Ok(results)
}

#[cfg(test)]
mod tests;
