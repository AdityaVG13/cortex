// SPDX-License-Identifier: MIT
//! System prompt injector CLI.
//!
//! `cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--watch]`
//!
//! Reads a base system prompt file, appends Cortex context (boot data),
//! and writes the result to `<file>.injected`. With `--watch`, re-injects
//! whenever the source file changes (file-based refresh).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

const DEFAULT_BUDGET: u32 = 400;
const USAGE: &str =
    "Usage: cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--watch]";

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
                budget = args[i]
                    .parse()
                    .map_err(|_| format!("{USAGE}\nInvalid --budget '{}'", args[i]))?;
            }
            "--watch" | "-w" => {
                watch = true;
            }
            _ => {}
        }
        i += 1;
    }

    let Some(file_path) = file_path else {
        return Err(format!("{USAGE}\nMissing required --file <path>"));
    };

    Ok(PromptInjectConfig {
        file_path,
        agent,
        budget,
        watch,
    })
}

fn compose_injected_prompt(base_prompt: &str, cortex_context: &str) -> String {
    format!("{base_prompt}\n\n{cortex_context}")
}

fn output_path_for(file_path: &Path) -> PathBuf {
    let mut out: OsString = file_path.as_os_str().to_os_string();
    out.push(".injected");
    PathBuf::from(out)
}

pub async fn run(args: &[String]) {
    let config = match parse_args(args) {
        Ok(config) => config,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(1);
        }
    };

    if config.watch {
        run_watch_loop(&config.file_path, &config.agent, config.budget).await;
    } else {
        if let Err(e) = inject_once(&config.file_path, &config.agent, config.budget).await {
            eprintln!("[prompt-inject] Error: {e}");
            std::process::exit(1);
        }
    }
}

async fn inject_once(file_path: &Path, agent: &str, budget: u32) -> Result<(), String> {
    let base_prompt = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read {}: {e}", file_path.display()))?;

    let cortex_context = fetch_boot_context(agent, budget).await;

    let output = compose_injected_prompt(&base_prompt, &cortex_context);

    let out_path = output_path_for(file_path);
    std::fs::write(&out_path, &output)
        .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;

    eprintln!(
        "[prompt-inject] Wrote {} ({} bytes)",
        out_path.display(),
        output.len()
    );
    Ok(())
}

async fn run_watch_loop(file_path: &Path, agent: &str, budget: u32) {
    eprintln!(
        "[prompt-inject] Watching {} for changes (Ctrl+C to stop)",
        file_path.display()
    );

    let mut last_modified = file_modified(file_path);

    // Initial injection
    if let Err(e) = inject_once(file_path, agent, budget).await {
        eprintln!("[prompt-inject] Initial inject error: {e}");
    }

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let current = file_modified(file_path);
        if current != last_modified {
            last_modified = current;
            eprintln!("[prompt-inject] File changed, re-injecting...");
            if let Err(e) = inject_once(file_path, agent, budget).await {
                eprintln!("[prompt-inject] Re-inject error: {e}");
            }
        }
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

async fn fetch_boot_context(agent: &str, budget: u32) -> String {
    let token = read_auth_token();
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(7))
        .build()
    {
        Ok(c) => c,
        Err(e) => return format!("<!-- Cortex: failed to create HTTP client ({e}) -->"),
    };

    let port = crate::auth::CortexPaths::resolve().port;
    let mut url = match reqwest::Url::parse(&format!("http://127.0.0.1:{port}/boot")) {
        Ok(u) => u,
        Err(e) => return format!("<!-- Cortex: invalid boot URL ({e}) -->"),
    };
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("agent", agent);
        query.append_pair("budget", &budget.to_string());
    }
    let mut req = client.get(url).header("x-cortex-request", "true");
    if let Some(t) = &token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }

    match req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(data) => {
                let boot = data
                    .get("bootPrompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no boot prompt)");
                format!(
                    "<!-- Cortex context (auto-injected) -->\n{boot}\n<!-- end Cortex context -->"
                )
            }
            Err(_) => "<!-- Cortex: boot response parse error -->".to_string(),
        },
        Ok(resp) => format!("<!-- Cortex: boot returned {} -->", resp.status()),
        Err(e) => format!("<!-- Cortex: daemon unreachable ({e}) -->"),
    }
}

fn read_auth_token_from_path(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(token) => {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Err(_) => None,
    }
}

fn read_auth_token() -> Option<String> {
    let path = crate::auth::CortexPaths::resolve().token;
    read_auth_token_from_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_prompt_inject_{name}_{unique}"))
    }

    #[test]
    fn parse_args_supports_short_and_long_flags() {
        let args = vec![
            "--file".to_string(),
            "C:/tmp/system.txt".to_string(),
            "-a".to_string(),
            "codex".to_string(),
            "--budget".to_string(),
            "512".to_string(),
            "-w".to_string(),
        ];
        let parsed = parse_args(&args).expect("args should parse");
        assert_eq!(parsed.file_path, PathBuf::from("C:/tmp/system.txt"));
        assert_eq!(parsed.agent, "codex");
        assert_eq!(parsed.budget, 512);
        assert!(parsed.watch);
    }

    #[test]
    fn parse_args_requires_file() {
        let args = vec!["--agent".to_string(), "codex".to_string()];
        let err = parse_args(&args).expect_err("missing file should error");
        assert!(err.contains("Missing required --file <path>"));
    }

    #[test]
    fn parse_args_missing_agent_value_errors() {
        let args = vec![
            "--file".to_string(),
            "prompt.txt".to_string(),
            "--agent".to_string(),
        ];
        let err = parse_args(&args).expect_err("missing agent value should error");
        assert!(err.contains("Missing value for --agent"));
    }

    #[test]
    fn parse_args_invalid_budget_errors() {
        let args = vec![
            "--file".to_string(),
            "prompt.txt".to_string(),
            "--budget".to_string(),
            "not-a-number".to_string(),
        ];
        let err = parse_args(&args).expect_err("invalid budget should error");
        assert!(err.contains("Invalid --budget"));
    }

    #[test]
    fn compose_injected_prompt_appends_cortex_context() {
        let output = compose_injected_prompt("base prompt", "<!-- context -->");
        assert_eq!(output, "base prompt\n\n<!-- context -->");
    }

    #[test]
    fn file_modified_returns_zero_for_missing_path() {
        let path = PathBuf::from("__missing_prompt_inject_file__.txt");
        assert_eq!(file_modified(&path), 0);
    }

    #[test]
    fn output_path_appends_injected_suffix() {
        let path = PathBuf::from("C:/tmp/system.txt");
        let out = output_path_for(&path);
        assert_eq!(out, PathBuf::from("C:/tmp/system.txt.injected"));

        let dotfile = PathBuf::from("C:/tmp/.env");
        let dot_out = output_path_for(&dotfile);
        assert_eq!(dot_out, PathBuf::from("C:/tmp/.env.injected"));
    }

    #[test]
    fn read_auth_token_from_path_reads_trimmed_token() {
        let temp_home = unique_temp_dir("token");
        std::fs::create_dir_all(&temp_home).expect("create temp home");
        let token_path = temp_home.join("cortex.token");
        std::fs::write(&token_path, "ctx_prompt_token\n").expect("write token file");

        let token = read_auth_token_from_path(&token_path);
        assert_eq!(token.as_deref(), Some("ctx_prompt_token"));

        let _ = std::fs::remove_dir_all(&temp_home);
    }
}
