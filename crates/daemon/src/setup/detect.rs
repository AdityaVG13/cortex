use super::types::{ConfigMethod, DetectedTool};
use std::path::PathBuf;
use std::process::Command;
fn detected(name: &'static str, agent_name: &'static str, config_path: Option<PathBuf>, config_method: ConfigMethod) -> DetectedTool {
    DetectedTool { name, agent_name, config_path, config_method }
}

pub(crate) fn step_detect() -> Vec<DetectedTool> {
    let mut found = Vec::new();
    if let Some(config_path) = find_claude_code_config() {
        found.push(detected("Claude Code", "claude-code", Some(config_path), ConfigMethod::JsonMerge));
    } else if command_exists("claude") {
        found.push(detected(
            "Claude Code",
            "claude-code",
            None,
            ConfigMethod::CliCommand { program: "claude", args: &["mcp", "add", "cortex", "-s", "user", "--"] },
        ));
    }
    if let Some(config_path) = find_claude_desktop_config() {
        found.push(detected("Claude Desktop", "claude", Some(config_path), ConfigMethod::JsonMerge));
    }
    if let Some(config_path) = find_codex_config() {
        found.push(detected("Codex CLI", "codex", Some(config_path), ConfigMethod::TomlMerge));
    } else if command_exists("codex") {
        found.push(detected("Codex CLI", "codex", None, ConfigMethod::CliCommand { program: "codex", args: &["mcp", "add", "cortex", "--"] }));
    }
    for (find, name, slug) in [(find_cursor_config as fn() -> Option<PathBuf>, "Cursor", "cursor"), (find_windsurf_config, "Windsurf", "windsurf")] {
        if let Some(config_path) = find() {
            found.push(detected(name, slug, Some(config_path), ConfigMethod::JsonMerge));
        }
    }
    found
}
fn find_claude_desktop_config() -> Option<PathBuf> {
    find_first_config_path(claude_desktop_config_paths())
}
fn find_claude_code_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let user_json = home.join(".claude.json");
    if user_json.is_file() {
        return Some(user_json);
    }
    find_existing_config(home.join(".claude").join("settings.json"))
}
fn find_home_config(parts: &[&str]) -> Option<PathBuf> {
    let mut path = dirs::home_dir()?;
    for part in parts {
        path = path.join(part);
    }
    find_existing_config(path)
}
fn find_codex_config() -> Option<PathBuf> {
    find_home_config(&[".codex", "config.toml"])
}
fn claude_desktop_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            paths.push(PathBuf::from(appdata).join("Claude").join("claude_desktop_config.json"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(config) = std::env::var("XDG_CONFIG_HOME") {
            paths.push(PathBuf::from(config).join("Claude").join("claude_desktop_config.json"));
        } else if let Some(home) = dirs::home_dir() {
            paths.push(home.join(".config").join("Claude").join("claude_desktop_config.json"));
        }
    }
    paths
}
fn find_cursor_config() -> Option<PathBuf> {
    find_home_config(&[".cursor", "mcp.json"])
}
fn find_windsurf_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    find_first_config_path(vec![home.join(".codeium").join("windsurf").join("mcp_config.json"), home.join(".windsurf").join("mcp.json")])
}
fn find_first_config_path(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find_map(find_existing_config)
}
pub(crate) fn find_existing_config(path: PathBuf) -> Option<PathBuf> {
    (path.exists() || path.parent().is_some_and(|p| p.exists())).then_some(path)
}
fn command_exists(cmd: &str) -> bool {
    #[cfg(windows)]
    let finder = "where";
    #[cfg(not(windows))]
    let finder = "which";
    Command::new(finder).arg(cmd).output().map(|o| o.status.success()).unwrap_or(false)
}
