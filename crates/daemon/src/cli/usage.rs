use serde_json::{Value, json};
pub(crate) fn cli_usage_text() -> String {
    format!(
        "Cortex v{} -- Local agent memory\n\nUsage: cortex <command> [--home <path>] [--db <path>]\n\n  capture <register|enable|disable|put|tail|file|get|inventory|prepare|host-cycle> [--quiet]\n  capabilities --json      Discover local operations\n  status [--json]           Open and inspect the local brain\n  setup                    Configure MCP clients and initialize memory\n  mcp [--agent <name>]      MCP stdio, in-process\n  boot [--agent <name>] [--budget <n>] [--path <cwd>] [--json]\n  op <operation> [--args '<json>'] [--agent <name>]\n  maintain [--jobs <n>]     Drain a bounded maintenance slice\n  serve                    Optional headless maintenance worker, no listener\n  paths --json             Show local paths\n  hook-boot [--agent <name>] | hook-status | hook <kind>\n  prompt-inject --file <path> [--agent <name>] [--budget <n>] [--watch]\n  doctor | backup | restore <file>\n  cleanup [--dry-run] [--events] [--max-passes <n>]\n  reindex [--json] | recrystallize [--json]\n  rebuild-anchors [--json] [--batch-size <n>]\n  embeddings status [--json] | eval | robot-docs guide\n\nNo network endpoint, background service, or remote API key is required.\n",
        env!("CARGO_PKG_VERSION")
    )
}
pub fn cli_service_usage() -> &'static str {
    "Usage: cortex service <install|uninstall|start|stop|status|ensure>"
}
pub fn cli_capabilities_payload() -> Value {
    json!({"schema_version":2,"contract_version":"2","tool":{"name":"cortex","version":env!("CARGO_PKG_VERSION"),"runtime":"asupersync","transport":"local"},
    "commands":{
      "capture":{"output":"json_or_silent","side_effects":"may_write_local_brain","usage":"cortex capture <register|enable|disable|put|tail|file|get|inventory|prepare|host-cycle>"},
      "mcp":{"usage":"cortex mcp [--agent <name>]","output":"stdio_json_rpc","side_effects":"may_write_local_brain"},
      "boot":{"usage":"cortex boot [--agent <name>] [--budget <n>] [--path <cwd>] [--json]","output":"human_or_json","side_effects":"opens_local_brain"},
      "op":{"usage":"cortex op <operation> [--args <json>] [--agent <name>]","output":"json","side_effects":"may_write_local_brain"},
      "status":{"usage":"cortex status [--json]","output":"human_or_json","side_effects":"opens_existing_local_brain"},
      "maintain":{"usage":"cortex maintain [--jobs <n>]","output":"json","side_effects":"writes_local_brain"},
      "serve":{"usage":"cortex serve","purpose":"Optional bounded maintenance worker; no network listener","side_effects":"writes_local_brain"}},
    "operations":["capabilities","orient","query","expand","commit","checkpoint","resolve","feedback"],
    "environment":{"CORTEX_HOME":"Local memory directory","CORTEX_DB":"Local SQLite database"},
    "exit_codes":{"0":"success","1":"runtime_or_input_error","2":"invalid_operation"}})
}
pub fn cli_capabilities_summary() -> String {
    "Cortex local agent memory\nRuntime: asupersync\nSurfaces: in-process library, MCP stdio, CLI\nDiscover: cortex capabilities --json\nNo HTTP listener or service dependency.".into()
}
pub fn cli_robot_docs_guide() -> &'static str {
    "Discover with cortex capabilities --json. Use cortex boot --json for initial context, cortex op for semantic operations, and cortex mcp --agent <name> for MCP stdio. All access is local. Run cortex maintain for bounded maintenance and cortex doctor for database diagnostics. Back up before restore or rebuilding derived data."
}
fn top_level_command_suggestion(command: &str) -> Option<&'static str> {
    match command {
        "caps" | "capability" => Some("cortex capabilities --json"),
        "stat" => Some("cortex status --json"),
        "path" => Some("cortex paths --json"),
        _ => None,
    }
}
pub fn unknown_cli_command_message(command: &str) -> String {
    let prefix = if command.starts_with('-') { format!("Unknown option: {command}") } else { format!("Unknown command: {command}") };
    match top_level_command_suggestion(command) {
        Some(suggestion) => {
            format!("{prefix}\nDid you mean: `{suggestion}`?\nRun `cortex help` or `cortex capabilities --json` for supported commands.")
        }
        None => format!("{prefix}\nRun `cortex help` or `cortex capabilities --json` for supported commands."),
    }
}
pub fn unknown_robot_docs_subcommand_message(subcommand: &str) -> String {
    format!("Unknown robot-docs command: {subcommand}\nDid you mean: `cortex robot-docs guide`?")
}
pub fn print_usage_and_exit(code: i32) -> ! {
    let usage = cli_usage_text();
    if code == 0 {
        print!("{usage}");
    } else {
        eprint!("{usage}");
    }
    std::process::exit(code);
}
