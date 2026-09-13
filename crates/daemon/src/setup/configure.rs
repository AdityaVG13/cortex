use super::types::{ConfigMethod, DetectedTool, StepResult};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
pub(crate) fn step_configure(tools: &[DetectedTool], cortex_exe: &str) -> Vec<(&'static str, StepResult)> {
    let mut results = Vec::new();
    for tool in tools {
        let result = configure_tool(tool, cortex_exe);
        results.push((tool.name, result));
    }
    results
}
fn configure_tool(tool: &DetectedTool, cortex_exe: &str) -> StepResult {
    match &tool.config_method {
        ConfigMethod::JsonMerge => {
            let Some(config_path) = &tool.config_path else {
                return StepResult::Fail("No config path".into());
            };
            match merge_mcp_config(config_path, cortex_exe, tool.agent_name) {
                Ok(action) => StepResult::Ok(action),
                Err(e) => StepResult::Warn(format!("Auto-config failed: {e}. Configure manually.")),
            }
        }
        ConfigMethod::TomlMerge => {
            let Some(config_path) = &tool.config_path else {
                return StepResult::Fail("No config path".into());
            };
            match merge_toml_config(config_path, cortex_exe, tool.agent_name) {
                Ok(action) => StepResult::Ok(action),
                Err(e) => StepResult::Warn(format!("Auto-config failed: {e}. Configure manually.")),
            }
        }
        ConfigMethod::CliCommand { program, args } => match run_mcp_add(program, args, cortex_exe, tool.agent_name) {
            Ok(()) => StepResult::Ok("Registered via CLI".into()),
            Err(e) => StepResult::Warn(format!(
                "CLI failed: {e}. Run manually: {} {} {cortex_exe} {}",
                program,
                args.join(" "),
                mcp_stdio_args(tool.agent_name).join(" ")
            )),
        },
        ConfigMethod::Manual(instructions) => StepResult::Ok(format!("Manual setup needed: {instructions}")),
    }
}
pub(crate) fn merge_mcp_config(config_path: &Path, cortex_exe: &str, agent_name: &str) -> Result<String, String> {
    let original: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| format!("Cannot read {}: {e}", config_path.display()))?;
        serde_json::from_str(&content).map_err(|e| format!("Invalid JSON in {}: {e}", config_path.display()))?
    } else {
        serde_json::json!({})
    };
    let mut config = original.clone();
    let mcp_servers = config
        .as_object_mut()
        .ok_or("Config is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let exe_path = PathBuf::from(cortex_exe).to_string_lossy().to_string();
    let desired_registration = serde_json::json!({"command": exe_path, "args": mcp_stdio_args(agent_name)});
    mcp_servers.as_object_mut().ok_or("mcpServers is not a JSON object")?.insert("cortex".to_string(), desired_registration);
    let action = if config == original {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };
    if config != original {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        let output = serde_json::to_string_pretty(&config).map_err(|e| format!("JSON serialize failed: {e}"))?;
        write_config_atomic(config_path, &output)?;
    }
    Ok(format!("{action} at {}", config_path.display()))
}
pub(crate) fn merge_toml_config(config_path: &Path, cortex_exe: &str, agent_name: &str) -> Result<String, String> {
    let original: toml::Value = if config_path.exists() {
        let content = fs::read_to_string(config_path).map_err(|e| format!("Cannot read {}: {e}", config_path.display()))?;
        toml::from_str(&content).map_err(|e| format!("Invalid TOML in {}: {e}", config_path.display()))?
    } else {
        toml::Value::Table(Default::default())
    };
    let mut config = original.clone();
    let root = config.as_table_mut().ok_or("Config is not a TOML table")?;
    let servers = root.entry("mcp_servers").or_insert_with(|| toml::Value::Table(Default::default()));
    let servers_table = servers.as_table_mut().ok_or("mcp_servers is not a TOML table")?;
    let mut server = toml::map::Map::new();
    server.insert("command".into(), toml::Value::String(PathBuf::from(cortex_exe).to_string_lossy().to_string()));
    server.insert(
        "args".into(),
        toml::Value::Array(mcp_stdio_args(agent_name).into_iter().map(toml::Value::String).collect()),
    );
    servers_table.insert("cortex".into(), toml::Value::Table(server));
    let action = if config == original {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };
    if config != original {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        let output = toml::to_string_pretty(&config).map_err(|e| format!("TOML serialize failed: {e}"))?;
        write_config_atomic(config_path, &output)?;
    }
    Ok(format!("{action} at {}", config_path.display()))
}

/// Stage beside the destination then rename. In-place `fs::write` truncates
/// the live inode first; a crash mid-write leaves a truncated Claude/Cursor
/// MCP config that the host then fails to parse.
fn write_config_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| format!("Invalid config path: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temp_path = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let write_tmp = (|| {
        let mut file = File::create(&temp_path).map_err(|e| format!("Cannot write {}: {e}", temp_path.display()))?;
        file.write_all(contents.as_bytes())
            .map_err(|e| format!("Cannot write {}: {e}", temp_path.display()))?;
        file.sync_all()
            .map_err(|e| format!("Cannot flush {}: {e}", temp_path.display()))?;
        Ok::<(), String>(())
    })();
    if let Err(err) = write_tmp {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    #[cfg(windows)]
    {
        if path.exists() {
            if let Err(e) = fs::remove_file(path) {
                let _ = fs::remove_file(&temp_path);
                return Err(format!("Cannot replace {}: {e}", path.display()));
            }
        }
    }
    if let Err(err) = fs::rename(&temp_path, path) {
        #[cfg(windows)]
        {
            if !path.exists() {
                let _ = fs::rename(&temp_path, path);
            } else {
                let _ = fs::remove_file(&temp_path);
            }
        }
        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&temp_path);
        }
        return Err(format!("Cannot write {}: {err}", path.display()));
    }
    Ok(())
}

fn mcp_stdio_args(agent_name: &str) -> Vec<String> {
    let paths = crate::auth::CortexPaths::resolve();
    let mut args = vec!["mcp".into(), "--agent".into(), agent_name.into(), "--home".into(), paths.home.to_string_lossy().into_owned()];
    let default_db = paths.home.join("cortex.db");
    if paths.db != default_db {
        args.push("--db".into());
        args.push(paths.db.to_string_lossy().into_owned());
    }
    args
}

fn run_mcp_add(program: &str, args: &[&str], cortex_exe: &str, agent_name: &str) -> Result<(), String> {
    let invocation = mcp_stdio_args(agent_name);
    let mut command = Command::new(program);
    command.args(args).arg(cortex_exe).args(&invocation);
    crate::auth::CortexPaths::resolve().apply_to_command(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("Failed to run {program} CLI: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already exists") || stderr.contains("Already") {
            Ok(())
        } else {
            Err(stderr.trim().to_string())
        }
    }
}
pub(crate) fn summarize_configs(results: &[(&str, StepResult)]) -> StepResult {
    if results.is_empty() {
        return StepResult::Warn("No tools to configure".into());
    }
    let ok_count = results.iter().filter(|(_, r)| matches!(r, StepResult::Ok(_))).count();
    let warn_count = results.iter().filter(|(_, r)| matches!(r, StepResult::Warn(_))).count();
    let fail_count = results.iter().filter(|(_, r)| matches!(r, StepResult::Fail(_))).count();
    if fail_count > 0 {
        StepResult::Warn(format!("{ok_count} configured, {warn_count} warnings, {fail_count} failed"))
    } else if warn_count > 0 {
        StepResult::Warn(format!("{ok_count} configured, {warn_count} need manual setup"))
    } else {
        StepResult::Ok(format!("{ok_count}/{} tools configured", results.len()))
    }
}
