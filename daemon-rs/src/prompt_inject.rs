// SPDX-License-Identifier: MIT
//! System prompt injector CLI.
//!
//! `cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--watch]`
//!
//! Reads a base system prompt file, appends Cortex context (boot data),
//! and writes the result to `<file>.injected`. With `--watch`, re-injects
//! whenever the source file changes (file-based refresh).

use std::path::PathBuf;

const DEFAULT_BUDGET: u32 = 400;

pub async fn run(args: &[String]) {
    let mut file_path: Option<PathBuf> = None;
    let mut agent = "prompt-inject".to_string();
    let mut budget = DEFAULT_BUDGET;
    let mut watch = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" | "-f" => {
                i += 1;
                if i < args.len() {
                    file_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--agent" | "-a" => {
                i += 1;
                if i < args.len() {
                    agent = args[i].clone();
                }
            }
            "--budget" | "-b" => {
                i += 1;
                if i < args.len() {
                    budget = args[i].parse().unwrap_or(DEFAULT_BUDGET);
                }
            }
            "--watch" | "-w" => {
                watch = true;
            }
            _ => {}
        }
        i += 1;
    }

    let file_path = match file_path {
        Some(p) => p,
        None => {
            eprintln!(
                "Usage: cortex prompt-inject --file <path> [--agent NAME] [--budget N] [--watch]"
            );
            std::process::exit(1);
        }
    };

    if watch {
        run_watch_loop(&file_path, &agent, budget).await;
    } else {
        if let Err(e) = inject_once(&file_path, &agent, budget).await {
            eprintln!("[prompt-inject] Error: {e}");
            std::process::exit(1);
        }
    }
}

async fn inject_once(file_path: &PathBuf, agent: &str, budget: u32) -> Result<(), String> {
    let base_prompt = std::fs::read_to_string(file_path)
        .map_err(|e| format!("Failed to read {}: {e}", file_path.display()))?;

    let cortex_context = fetch_boot_context(agent, budget).await;

    let output = format!("{base_prompt}\n\n{cortex_context}");

    let out_path = file_path.with_extension("injected");
    std::fs::write(&out_path, &output)
        .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))?;

    eprintln!(
        "[prompt-inject] Wrote {} ({} bytes)",
        out_path.display(),
        output.len()
    );
    Ok(())
}

async fn run_watch_loop(file_path: &PathBuf, agent: &str, budget: u32) {
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

fn file_modified(path: &PathBuf) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
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
    let url = format!("http://127.0.0.1:{port}/boot?agent={agent}&budget={budget}");
    let mut req = client.get(&url).header("x-cortex-request", "true");
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

fn read_auth_token() -> Option<String> {
    let path = crate::auth::CortexPaths::resolve().token;
    match std::fs::read_to_string(&path) {
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
