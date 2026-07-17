use crate::DEFAULT_CORTEX_PORT;use serde_json::{json,Value};pub(crate)const CLI_CAPABILITIES_CONTRACT_VERSION:&str="1";pub(crate)
fn cli_usage_text()->String{format!(
r#"Cortex v{} -- Universal AI Memory Daemon
Usage:
  cortex <command>
  cortex help
  cortex capabilities --json
  cortex robot-docs guide
Options:
  --version, -V      Print CLI version
  --help, -h         Print this help
Agent surfaces:
  capabilities --json  Print a deterministic machine-readable CLI contract
  robot-docs guide     Print a short operator guide for coding agents
Setup:
  status [--json]    Show memory readiness, checks, next action, and repair
  setup              First-run setup: detect AI tools, configure, verify
  setup --team       Team-mode setup + schema migration + owner API key
  migrate            Alias for setup --team (solo -> team migration)
  migrate --dry-run  Preview migration without modifying the database
Daemon:
  serve [--bind <addr>]  HTTP daemon on :{} (default bind 127.0.0.1)
  mcp [--url <base>] [--api-key <key>] [--agent <name>]  MCP stdio
  paths --json       Print resolved Cortex paths + port + bind as JSON
  boot [--agent <name>] [--budget <n>] [--json] [--url <base>] [--api-key <key>]
  plugin ensure-daemon [--agent <name>]  Ensure daemon is running, then print port
  plugin mcp [--url <base>] [--api-key <key>] [--agent <name>]
Hooks:
  hook-boot [AGENT]  SessionStart hook (default: claude-opus)
  hook-status        Statusline one-liner
Tools:
  prompt-inject      Inject Cortex context into system prompt files
  export             Export data (--format json|sql, --out <file>)
  import             Import JSON data (--file <path>, optional --user <username>)
  sync export        Export changeset JSON (--out <file>, optional --since <iso>)
  sync import        Import a sync changeset (--file <path>, optional --user/--visibility)
  sync watch         Watched-folder sync loop (--dir <path>, optional --once)
  eval               Run retrieval evaluation; supports --json and regression flags
  doctor             Validate DB schema, migrations, integrity, and FTS health
  reindex [--json]   Fully rebuild FTS indexes from canonical rows
  re-embed [...]     Alias for embeddings drain --until-exhausted
  recrystallize [--json]  Rebuild crystal graph and embeddings
  cleanup [--dry-run] [--events] [--max-passes <n>]
  backup             Create manual backup (stores in ~/.cortex/backups/)
  restore <file>     Restore from backup file (daemon must be stopped)
  admin rollback --session-id <id> [--apply] [--json]
Embeddings:
  embeddings status [--json]  Show active-model embedding backlog counts
  embeddings drain [--batch-size <n>] [--max-batches <n>] [--lock-wait-ms <n>] [--until-exhausted] [--json]
User Management (team mode):
  user add <name>    Add user [--role member|admin] [--display-name "..."]
  user rotate-key <name>  Rotate a user's API key
  user remove <name> Remove user (with confirmation)
  user list          List all users
Team Management (team mode):
  team create <name> Create a team
  team add <team> <user>  Add member [--role member|admin]
  team remove <team> <user>  Remove member (with confirmation)
  team list          List all teams
Admin (team mode):
  admin list-unowned List rows without an owner
  admin assign-owner [--from <user>] --to <user> [--table <t>]
  admin stats        Database and per-user statistics
  admin budgets status [--json]
  admin budgets validate --path <file> [--json]
Service:
  service install    Register as Windows Service (manual start by default)
  service uninstall  Remove Windows Service
  service start      Start the service
  service stop       Stop the service
  service status     Check service status
  service ensure     Ensure service is installed, running, and healthy
Troubleshooting:
  cortex doctor      Validate DB schema, migrations, integrity, and FTS state
  cortex boot        Preferred local boot path (auto-adds auth + SSRF headers)
  HTTP 403           Add header: X-Cortex-Request: true
  HTTP 401           Use Authorization: Bearer <token> from ~/.cortex/cortex.token
  MCP not visible    Restart the client after adding MCP servers; they do not hot-attach mid-session
  App-hosted daemon  Restart the daemon from Cortex Control Center instead of stopping/starting it manually
  More help          See Info/connecting.md for full connection and auth examples
"#
,env!("CARGO_PKG_VERSION"),DEFAULT_CORTEX_PORT)}pub(crate)fn cli_service_usage()->&'static str{
"Usage: cortex service <install|uninstall|start|stop|status|ensure>"}pub(crate)fn cli_capabilities_payload()->Value{json!({
"schema_version":1,"contract_version":CLI_CAPABILITIES_CONTRACT_VERSION,"tool":{"name":"cortex","version":env!("CARGO_PKG_VERSION"
),"default_port":DEFAULT_CORTEX_PORT,"default_bind":"127.0.0.1"},"agent_entrypoints":[{"name":"help","command":"cortex help",
"output":"human","side_effects":"none"},{"name":"capabilities","command":"cortex capabilities --json","output":"json",
"side_effects":"none"},{"name":"status","command":"cortex status --json","output":"json","side_effects":"none"},{"name":
"robot-docs","command":"cortex robot-docs guide","output":"text","side_effects":"none"}],"commands":{"serve":{"usage":
"cortex serve [--bind <addr>] [--port <n>]","purpose":"Run the HTTP daemon","output":"logs","side_effects":"starts_daemon"},"mcp":
{"usage":"cortex mcp [--url <base>] [--api-key <key>] [--agent <name>]","purpose":"Run the MCP stdio proxy","output":
"stdio_json_rpc","side_effects":"may_ensure_local_daemon"},"paths":{"usage":"cortex paths --json","purpose":
"Print resolved paths, port, and bind configuration","output":"json","side_effects":"none"},"status":{"usage":
"cortex status [--json]","purpose":"Report memory readiness, checks, next action, and repair without starting a daemon","output":
"human_or_json","side_effects":"none"},"boot":{"usage":"cortex boot [--agent <name>] [--budget <n>] [--json]","purpose":
"Preferred local boot path with auth and SSRF headers","output":"human_or_json","side_effects":"may_ensure_local_daemon"},"doctor"
:{"usage":"cortex doctor","purpose":"Validate database schema, migrations, integrity, and FTS health","output":"human",
"side_effects":"reads_database"},"admin budgets":{"usage":"cortex admin budgets status [--json]","purpose":
"Inspect or validate budget governance configuration","output":"human_or_json","side_effects":"none"},"admin rollback":{"usage":
"cortex admin rollback --session-id <id> [--apply] [--json]","purpose":"Dry-run or apply soft-delete rollback for one session",
"output":"human_or_json","side_effects":"mutates_database_with_--apply"}},"environment":{"CORTEX_HOME":
"Overrides the Cortex home directory","CORTEX_PORT":"Overrides the daemon port","CORTEX_BIND":
"Overrides daemon bind address; defaults to localhost","CORTEX_API_KEY":"Supplies API key for remote client commands",
"CORTEX_API_BASE":"Supplies base URL for remote client commands","NO_COLOR":"Requests plain output where color is supported"},
"exit_codes":{"0":"success","1":"user_input_or_runtime_error"},"dangerous_operations":[{"command":"cortex restore <file>","gate":
"requires explicit backup file and warns when daemon appears active"},{"command":"cortex admin rollback --session-id <id> --apply"
,"gate":"dry-run by default; --apply required to mutate"},{"command":"cortex user remove <name>","gate":"interactive confirmation"
},{"command":"cortex team remove <team> <user>","gate":"interactive confirmation"}],"recommended_agent_flow":[
"Run `cortex capabilities --json` to discover supported surfaces.",
"Run `cortex status --json` when you need readiness, next action, or repair without starting a daemon.",
"Use `cortex paths --json` before reading or writing Cortex files.",
"Use `cortex boot --json` for local attachment when a daemon may be needed.",
"Use JSON flags where available and treat non-zero exit as retryable only after inspecting stderr."]})}pub(crate)fn
cli_capabilities_summary()->String{format!(
"Cortex agent capabilities\n\
         JSON contract: cortex capabilities --json\n\
         Agent guide: cortex robot-docs guide\n\
         Core JSON commands: status --json, paths --json, boot --json, reindex --json, recrystallize --json, embeddings status --json, admin budgets status --json\n\
         Default daemon endpoint: http://127.0.0.1:{}\n\
         Exit codes: 0 success, 1 user-input or runtime error"
,DEFAULT_CORTEX_PORT)}pub(crate)fn cli_robot_docs_guide()->&'static str{
r#"Cortex robot guide
Discovery:
  cortex capabilities --json
  cortex help
  cortex status --json
  cortex paths --json
Local attach:
  cortex boot --json
  cortex mcp --agent codex
Health checks:
  cortex doctor
  cortex embeddings status --json
  cortex admin budgets status --json
Maintenance:
  cortex backup
  cortex cleanup --dry-run
  cortex reindex --json
  cortex recrystallize --json
Danger gates:
  cortex restore <file> warns if a daemon appears active.
  cortex admin rollback --session-id <id> is dry-run unless --apply is present.
  cortex user remove and cortex team remove ask for confirmation.
Output contract:
  Prefer commands with --json when present.
  Treat stderr as diagnostic text.
  Treat exit code 0 as success and exit code 1 as user-input or runtime failure.
"#
}fn top_level_command_suggestion(command:&str)->Option<&'static str>{let normalized=command.trim().to_ascii_lowercase().replace([
'_','-'],"");match normalized.as_str(){"capability"|"capabilitiesjson"|"caps"=>Some("cortex capabilities --json"),"robotdoc"|
"robotdocs"|"agentdoc"|"agentdocs"|"docs"=>Some("cortex robot-docs guide"),"stat"|"statusjson"=>Some("cortex status --json"),
"path"=>Some("cortex paths --json"),"budget"|"budgets"=>Some("cortex admin budgets status --json"),"rollback"=>Some(
"cortex admin rollback --session-id <id> [--apply] [--json]"),_=>None,}}pub(crate)fn unknown_cli_command_message(command:&str)->
String{let prefix=if command.starts_with('-'){format!("Unknown option: {command}")}else{format!("Unknown command: {command}")};
match top_level_command_suggestion(command){Some(suggestion)=>format!(
"{prefix}\nDid you mean: `{suggestion}`?\nRun `cortex help` or `cortex capabilities --json` for supported commands."),None=>format
!("{prefix}\nRun `cortex help` or `cortex capabilities --json` for supported commands."),}}pub(crate)fn
unknown_robot_docs_subcommand_message(subcommand:&str)->String{format!(
"Unknown robot-docs command: {subcommand}\nDid you mean: `cortex robot-docs guide`?")}pub(crate)fn print_usage_and_exit(code:i32)
->!{let usage=cli_usage_text();if code==0{print!("{usage}");}else{eprint!("{usage}");}std::process::exit(code);}
