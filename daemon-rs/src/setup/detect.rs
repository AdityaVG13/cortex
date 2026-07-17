use super::types::{ConfigMethod,DetectedTool};use std::path::PathBuf;use std::process::Command;pub(crate)fn step_detect()->Vec<
DetectedTool>{let mut found=Vec::new();if let Some(config_path)=find_claude_code_config(){found.push(DetectedTool{name:
"Claude Code",agent_name:"claude",config_path:Some(config_path),config_method:ConfigMethod::JsonMerge,});}else if command_exists(
"claude"){found.push(DetectedTool{name:"Claude Code",agent_name:"claude",config_path:None,config_method:ConfigMethod::CliCommand{
program:"claude",args:&["mcp","add","cortex","-s","user","--"],},});}if let Some(config_path)=find_claude_desktop_config(){found.
push(DetectedTool{name:"Claude Desktop",agent_name:"claude",config_path:Some(config_path),config_method:ConfigMethod::JsonMerge,})
;}if let Some(config_path)=find_codex_config(){found.push(DetectedTool{name:"Codex CLI",agent_name:"codex",config_path:Some(
config_path),config_method:ConfigMethod::TomlMerge,});}else if command_exists("codex"){found.push(DetectedTool{name:"Codex CLI",
agent_name:"codex",config_path:None,config_method:ConfigMethod::CliCommand{program:"codex",args:&["mcp","add","cortex","--"],},});
}if let Some(config_path)=find_cursor_config(){found.push(DetectedTool{name:"Cursor",agent_name:"cursor",config_path:Some(
config_path),config_method:ConfigMethod::JsonMerge,});}if let Some(config_path)=find_windsurf_config(){found.push(DetectedTool{
name:"Windsurf",agent_name:"windsurf",config_path:Some(config_path),config_method:ConfigMethod::JsonMerge,});}found}fn
find_claude_desktop_config()->Option<PathBuf>{find_first_config_path(claude_desktop_config_paths())}fn find_claude_code_config()->
Option<PathBuf>{let home=dirs::home_dir()?;find_existing_config(home.join(".claude").join("settings.json"))}fn find_codex_config()
->Option<PathBuf>{let home=dirs::home_dir()?;find_existing_config(home.join(".codex").join("config.toml"))}fn
claude_desktop_config_paths()->Vec<PathBuf>{let mut paths=Vec::new();#[cfg(windows)]{if let Ok(appdata)=std::env::var("APPDATA"){
paths.push(PathBuf::from(appdata).join("Claude").join("claude_desktop_config.json"));}}#[cfg(target_os="macos")]{if let Some(home)
=dirs::home_dir(){paths.push(home.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json"));}
}#[cfg(target_os="linux")]{if let Ok(config)=std::env::var("XDG_CONFIG_HOME"){paths.push(PathBuf::from(config).join("Claude").join
("claude_desktop_config.json"));}else if let Some(home)=dirs::home_dir(){paths.push(home.join(".config").join("Claude").join(
"claude_desktop_config.json"));}}paths}fn find_cursor_config()->Option<PathBuf>{let home=dirs::home_dir()?;find_existing_config(
home.join(".cursor").join("mcp.json"))}fn find_windsurf_config()->Option<PathBuf>{let home=dirs::home_dir()?;find_existing_config(
home.join(".windsurf").join("mcp.json"))}fn find_first_config_path(paths:Vec<PathBuf>)->Option<PathBuf>{paths.into_iter().find_map
(find_existing_config)}pub(crate)fn find_existing_config(path:PathBuf)->Option<PathBuf>{if path.exists()||path.parent().
is_some_and(|p|p.exists()){Some(path)}else{None}}fn command_exists(cmd:&str)->bool{#[cfg(windows)]{Command::new("where").arg(cmd).
output().map(|o|o.status.success()).unwrap_or(false)}#[cfg(not(windows))]{Command::new("which").arg(cmd).output().map(|o|o.status.
success()).unwrap_or(false)}}
