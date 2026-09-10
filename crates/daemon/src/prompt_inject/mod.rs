use std::ffi::OsString;
use std::path::{Path, PathBuf};
const DEFAULT_BUDGET: u32 = 400;
const USAGE: &str = "Usage: cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--watch]";
#[derive(Clone, Debug, PartialEq, Eq)]
struct PromptInjectConfig {
    file_path: PathBuf,
    agent: String,
    budget: u32,
    watch: bool,
}
fn parse_args(args: &[String]) -> Result<PromptInjectConfig, String> {
    let mut file_path: Option<PathBuf> = None;
    let mut agent = "prompt-inject".to_string();
    let mut budget = DEFAULT_BUDGET;
    let mut watch = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" | "-f" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("{USAGE}\nMissing value for --file"));
                }
                file_path = Some(PathBuf::from(&args[i]));
            }
            "--agent" | "-a" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("{USAGE}\nMissing value for --agent"));
                }
                agent = args[i].clone();
            }
            "--budget" | "-b" => {
                i += 1;
                if i >= args.len() {
                    return Err(format!("{USAGE}\nMissing value for --budget"));
                }
                budget = args[i].parse().map_err(|_| format!("{USAGE}\nInvalid --budget '{}'", args[i]))?;
            }
            "--watch" | "-w" => {
                watch = true;
            }
            other => {
                return Err(format!("{USAGE}\nUnknown option: {other}"));
            }
        }
        i += 1;
    }
    let Some(file_path) = file_path else {
        return Err(format!("{USAGE}\nMissing required --file <path>"));
    };
    Ok(PromptInjectConfig { file_path, agent, budget, watch })
}
fn compose_injected_prompt(base_prompt: &str, cortex_context: &str) -> String {
    format!("{base_prompt}\n\n{cortex_context}")
}
fn output_path_for(file_path: &Path) -> PathBuf {
    let mut out: OsString = file_path.as_os_str().to_os_string();
    out.push(".injected");
    PathBuf::from(out)
}
pub async fn run(cx: &asupersync::Cx, args: &[String]) {
    if args.iter().any(|arg| matches!(arg.as_str(), "--help" | "-h" | "help")) {
        println!("{USAGE}");
        return;
    }
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(1);
        }
    };
    if config.watch {
        run_watch_loop(cx, &config.file_path, &config.agent, config.budget).await;
    } else {
        if let Err(e) = inject_once(cx, &config.file_path, &config.agent, config.budget).await {
            eprintln!("[prompt-inject] Error: {e}");
            std::process::exit(1);
        }
    }
}
async fn inject_once(cx: &asupersync::Cx, file_path: &Path, agent: &str, budget: u32) -> Result<(), String> {
    let base_prompt = std::fs::read_to_string(file_path).map_err(|e| format!("Failed to read {}: {e}", file_path.display()))?;
    let cortex_context = fetch_boot_context(cx, agent, budget).await;
    let output = compose_injected_prompt(&base_prompt, &cortex_context);
    let out_path = output_path_for(file_path);
    std::fs::write(&out_path, &output).map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;
    eprintln!("[prompt-inject] Wrote {} ({} bytes)", out_path.display(), output.len());
    Ok(())
}
async fn run_watch_loop(cx: &asupersync::Cx, file_path: &Path, agent: &str, budget: u32) {
    let mut last_modified = None;
    loop {
        if cx.checkpoint().is_err() {
            return;
        }
        let current = file_modified(file_path);
        if last_modified != Some(current) {
            if let Err(err) = inject_once(cx, file_path, agent, budget).await {
                eprintln!("[prompt-inject] {err}");
            }
            last_modified = Some(current);
        }
        asupersync::time::sleep(cx.now(), std::time::Duration::from_secs(2)).await;
    }
}
fn file_modified(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
async fn fetch_boot_context(cx: &asupersync::Cx, agent: &str, budget: u32) -> String {
    let paths = crate::auth::CortexPaths::resolve();
    let runtime = match crate::CortexRuntime::open(&paths) {
        Ok(runtime) => runtime,
        Err(e) => return format!("<!-- Cortex: local runtime unavailable ({e}) -->"),
    };
    let state = runtime.state();
    if state.team_mode && state.default_owner_id.is_none() {
        return "<!-- Cortex: boot requires a local owner in team mode -->".to_string();
    }
    match runtime
        .boot(cx, crate::runtime::BootInput { agent: agent.to_string(), max_tokens: budget as usize, owner_id: state.default_owner_id })
        .await
    {
        Ok(result) => {
            let boot = result.boot_prompt;
            format!("<!-- Cortex context (auto-injected) -->\n{boot}\n<!-- end Cortex context -->")
        }
        Err(e) => format!("<!-- Cortex: boot failed ({e}) -->"),
    }
}
