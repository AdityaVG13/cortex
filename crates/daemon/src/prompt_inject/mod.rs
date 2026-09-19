use crate::cli::{die, or_die};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
const DEFAULT_BUDGET: u32 = 400;
const MAX_PROMPT_FILE_BYTES: u64 = 2 * 1024 * 1024;
const USAGE: &str = "Usage: cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--path DIR] [--watch]";
#[derive(Clone, Debug, PartialEq, Eq)]
struct PromptInjectConfig {
    file_path: PathBuf,
    agent: String,
    budget: u32,
    watch: bool,
    paths: Vec<String>,
}
fn take_arg<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    *i += 1;
    let missing = format!("{USAGE}\nMissing value for {flag}");
    if *i >= args.len() || args[*i].starts_with("--") {
        return Err(missing);
    }
    Ok(&args[*i])
}
fn take_nonempty<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, String> {
    let raw = take_arg(args, i, flag)?;
    if raw.trim().is_empty() {
        return Err(format!("{USAGE}\nMissing value for {flag}"));
    }
    Ok(raw)
}
fn parse_args(args: &[String]) -> Result<PromptInjectConfig, String> {
    let mut file_path: Option<PathBuf> = None;
    let mut agent = "prompt-inject".to_string();
    let mut budget = DEFAULT_BUDGET;
    let mut watch = false;
    let mut paths = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" | "-f" => file_path = Some(PathBuf::from(take_nonempty(args, &mut i, "--file")?)),
            "--agent" | "-a" => agent = take_nonempty(args, &mut i, "--agent")?.to_string(),
            "--budget" | "-b" => {
                let raw = take_arg(args, &mut i, "--budget")?;
                budget = raw.parse().map_err(|_| format!("{USAGE}\nInvalid --budget '{raw}'"))?;
            }
            "--path" => paths.push(take_nonempty(args, &mut i, "--path")?.trim().to_string()),
            "--watch" | "-w" => watch = true,
            other => return Err(format!("{USAGE}\nUnknown option: {other}")),
        }
        i += 1;
    }
    let Some(file_path) = file_path else {
        return Err(format!("{USAGE}\nMissing required --file <path>"));
    };
    Ok(PromptInjectConfig { file_path, agent, budget, watch, paths })
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
    let config = or_die(parse_args(args), "");
    if config.watch {
        run_watch_loop(cx, &config).await;
    } else if let Err(e) = inject_once(cx, &config).await {
        die(format!("[prompt-inject] Error: {e}"));
    }
}
fn read_prompt_file(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut raw = String::new();
    file.take(MAX_PROMPT_FILE_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    if raw.len() as u64 > MAX_PROMPT_FILE_BYTES {
        return Err(format!("Failed to read {}: prompt file exceeds {MAX_PROMPT_FILE_BYTES} bytes", path.display()));
    }
    Ok(raw)
}

async fn inject_once(cx: &asupersync::Cx, config: &PromptInjectConfig) -> Result<(), String> {
    let base_prompt = read_prompt_file(&config.file_path)?;
    let cortex_context = fetch_boot_context(cx, config).await;
    let output = compose_injected_prompt(&base_prompt, &cortex_context);
    let out_path = output_path_for(&config.file_path);
    std::fs::write(&out_path, &output).map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;
    eprintln!("[prompt-inject] Wrote {} ({} bytes)", out_path.display(), output.len());
    Ok(())
}
async fn run_watch_loop(cx: &asupersync::Cx, config: &PromptInjectConfig) {
    let mut last_modified = None;
    loop {
        if cx.checkpoint().is_err() {
            return;
        }
        let current = file_modified(&config.file_path);
        if last_modified != Some(current) {
            if let Err(err) = inject_once(cx, config).await {
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

async fn fetch_boot_context(cx: &asupersync::Cx, config: &PromptInjectConfig) -> String {
    let paths = crate::auth::CortexPaths::resolve();
    let runtime = match crate::CortexRuntime::open(&paths) {
        Ok(runtime) => runtime,
        Err(e) => return format!("<!-- Cortex: local runtime unavailable ({e}) -->"),
    };
    match crate::hook_boot::boot_for_paths(cx, &runtime, &config.agent, config.budget, &config.paths).await {
        Ok(result) => format!("<!-- Cortex context (auto-injected) -->\n{}\n<!-- end Cortex context -->", result.boot_prompt),
        Err(e) => format!("<!-- Cortex: {e} -->"),
    }
}
