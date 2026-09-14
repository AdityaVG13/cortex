use super::common::{parse_flag_usize, parse_flag_value, parse_flag_values, validate_cli_options};
use crate::{CortexRuntime, auth};
use asupersync::Cx;

pub async fn run_boot_cli(cx: &Cx, paths: &auth::CortexPaths, args: &[String]) -> Result<(), String> {
    validate_cli_options(args, &["--agent", "--budget", "--path"], &["--json"])?;
    let agent = parse_flag_value(args, "--agent").unwrap_or_else(|| "cli".into());
    let agent = agent.trim();
    if agent.is_empty() {
        return Err("agent cannot be empty".into());
    }
    let budget = parse_flag_usize(args, "--budget")?.unwrap_or(600);
    let runtime = CortexRuntime::open(paths).map_err(|err| err.to_string())?;
    let state = runtime.state();
    if state.team_mode && state.default_owner_id.is_none() {
        return Err("Team mode requires a local owner".into());
    }
    // BootInput.paths is the set of project roots for this compile.
    // `parse_flag_value` kept only the first `--path`; later roots were
    // accepted by the validator then dropped, so a two-checkout boot
    // scoped as if the second tree did not exist.
    let boot_paths = parse_flag_values(args, "--path");
    let result = runtime
        .boot(cx, crate::runtime::BootInput { agent: agent.into(), max_tokens: budget, owner_id: state.default_owner_id, paths: boot_paths })
        .await
        .map_err(|err| err.to_string())?;
    if args.iter().any(|arg| arg == "--json") {
        println!(
            "{}",
            serde_json::json!({"bootPrompt": result.boot_prompt, "tokenEstimate": result.token_estimate, "capsules": result.capsules, "savings": result.savings})
        );
    } else {
        println!("{}", result.boot_prompt);
    }
    Ok(())
}
