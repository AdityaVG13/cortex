#!/usr/bin/env python3
from pathlib import Path

MAIN = Path(__file__).resolve().parents[1] / "src" / "main.rs"

MAIN.write_text("""// SPDX-License-Identifier: MIT

/// Default TCP port the Cortex daemon binds to when no `--port` flag or
/// `CORTEX_PORT` env var is set.
pub const DEFAULT_CORTEX_PORT: u16 = 7437;

mod admin;
mod aging;
mod api_types;
mod auth;
mod budgets;
mod cli;
mod co_occurrence;
mod compaction;
mod compiler;
mod conflict;
mod crystallize;
mod daemon_lifecycle;
mod db;
mod embeddings;
mod eval;
mod export_data;
mod focus;
mod handlers;
mod hook_boot;
mod indexer;
mod mcp_proxy;
mod prompt_inject;
mod rate_limit;
mod rerank;
mod server;
mod service;
mod setup;
mod state;
#[cfg(test)]
mod test_env;
mod tls;
mod transport;
mod workspace;

use chrono::Utc;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cli::{
    apply_path_env, cli_capabilities_payload, cli_capabilities_summary, cli_robot_docs_guide,
    cli_service_usage, ensure_daemon, ensure_remote_target_has_api_key,
    is_disallowed_startup_binary_path, parse_flag_usize, parse_flag_value, print_usage_and_exit,
    resolve_client_target, run_admin_cli, run_backup_cli, run_boot_cli, run_cleanup_cli,
    run_doctor_cli, run_embeddings_cli, run_embeddings_drain_cli, run_eval_cli, run_export_cli,
    run_import_cli, run_recrystallize_cli, run_reindex_cli, run_restore_cli, run_status_cli,
    run_sync_cli, run_team_cli, run_user_cli, unknown_cli_command_message,
    unknown_robot_docs_subcommand_message, validate_cli_options_or_exit,
};

pub(crate) use cli::run_daemon;

pub(crate) fn install_daemon_panic_hook(paths: &auth::CortexPaths) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let panic_log_path = paths.home.join("panic.log");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        let location = info
            .location()
            .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
            .unwrap_or_else(|| "<unknown location>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!(
            "[{ts}] PANIC at {location}: {message}\\n{backtrace}\\n",
            ts = Utc::now().to_rfc3339(),
        );
        eprintln!("[cortex] {entry}");
        if let Some(parent) = panic_log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&panic_log_path)
        {
            let _ = file.write_all(entry.as_bytes());
        }
        previous(info);
    }));
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let paths = auth::CortexPaths::resolve_from_args(&args);
    if let Ok(current_exe) = std::env::current_exe() {
        if is_disallowed_startup_binary_path(&current_exe) {
            eprintln!(
                "[cortex] Refusing to run from disallowed runtime path: {}",
                current_exe.display()
            );
            std::process::exit(1);
        }
    }

    match mode {
        "" | "--help" | "-h" | "help" => print_usage_and_exit(0),
        "--version" | "-V" | "version" => println!("cortex {}", env!("CARGO_PKG_VERSION")),
        "capabilities" => {
            validate_cli_options_or_exit(&args[2..], &[], &["--json"]);
            if args.iter().any(|arg| arg == "--json") {
                println!("{}", serde_json::to_string_pretty(&cli_capabilities_payload()).unwrap());
            } else {
                println!("{}", cli_capabilities_summary());
            }
        }
        "status" => {
            validate_cli_options_or_exit(&args[2..], &[], &["--json"]);
            let exit_code = run_status_cli(&paths, args.iter().any(|arg| arg == "--json")).await;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        "robot-docs" => {
            let subcmd = args.get(2).map(String::as_str).unwrap_or("guide");
            match subcmd {
                "" | "guide" | "help" | "--help" | "-h" => println!("{}", cli_robot_docs_guide()),
                other => {
                    eprintln!("{}", unknown_robot_docs_subcommand_message(other));
                    std::process::exit(1);
                }
            }
        }
        "serve" => {
            validate_cli_options_or_exit(&args[2..], &[], &[]);
            #[cfg(unix)]
            async fn sigterm_future() {
                use tokio::signal::unix::{signal, SignalKind};
                let mut sigterm = match signal(SignalKind::terminate()) {
                    Ok(sigterm) => sigterm,
                    Err(err) => {
                        eprintln!("[cortex] Warning: failed to register SIGTERM handler: {err}");
                        std::future::pending::<()>().await;
                        return;
                    }
                };
                sigterm.recv().await;
            }
            #[cfg(not(unix))]
            async fn sigterm_future() {
                std::future::pending::<()>().await;
            }
            run_daemon(paths.clone(), async {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => eprintln!("[cortex] Received Ctrl+C, shutting down..."),
                    _ = sigterm_future() => eprintln!("[cortex] Received SIGTERM, shutting down..."),
                }
            }).await;
        }
        "mcp" => {
            let remaining = &args[2..];
            validate_cli_options_or_exit(remaining, &["--agent", "--url", "--api-key"], &[]);
            let agent = parse_flag_value(remaining, "--agent");
            let (base_url, api_key, local_owner_mode) = resolve_client_target(remaining, &paths);
            if let Err(e) = ensure_remote_target_has_api_key(&base_url, api_key.as_deref(), &paths) {
                eprintln!("[cortex-mcp] {e}");
                std::process::exit(1);
            }
            if local_owner_mode {
                apply_path_env(&paths);
                if let Err(e) = ensure_daemon(&paths, agent.as_deref(), false, false).await {
                    eprintln!("[cortex-mcp] {e}");
                    std::process::exit(1);
                }
            }
            if let Err(e) = mcp_proxy::run(&base_url, api_key.as_deref(), agent.as_deref()).await {
                eprintln!("[cortex-mcp] {e}");
                std::process::exit(1);
            }
        }
        "paths" => {
            validate_cli_options_or_exit(&args[2..], &[], &["--json"]);
            if args.iter().any(|a| a == "--json") {
                println!("{}", paths.to_json());
            } else {
                eprintln!("Usage: cortex paths --json");
                std::process::exit(1);
            }
        }
        "boot" => {
            if let Err(e) = run_boot_cli(&paths, &args[2..]).await {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        "plugin" => match args.get(2).map(|s| s.as_str()).unwrap_or("") {
            "ensure-daemon" => {
                validate_cli_options_or_exit(&args[3..], &["--agent"], &[]);
                let agent = parse_flag_value(&args[3..], "--agent");
                apply_path_env(&paths);
                if let Err(e) = ensure_daemon(&paths, agent.as_deref(), true, true).await {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
            "mcp" => {
                let remaining = &args[3..];
                validate_cli_options_or_exit(remaining, &["--agent", "--url", "--api-key"], &[]);
                let (base_url, api_key, local_owner_mode) = resolve_client_target(remaining, &paths);
                let agent = parse_flag_value(remaining, "--agent");
                if let Err(e) = ensure_remote_target_has_api_key(&base_url, api_key.as_deref(), &paths) {
                    eprintln!("[cortex-plugin] {e}");
                    std::process::exit(1);
                }
                if local_owner_mode {
                    apply_path_env(&paths);
                    if let Err(e) = ensure_daemon(&paths, agent.as_deref(), false, true).await {
                        eprintln!("[cortex-plugin] {e}");
                        std::process::exit(1);
                    }
                }
                if let Err(e) = mcp_proxy::run(&base_url, api_key.as_deref(), agent.as_deref()).await {
                    eprintln!("[cortex-plugin] {e}");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Usage: cortex plugin <ensure-daemon|mcp>");
                std::process::exit(1);
            }
        },
        "hook-boot" => {
            let agent = args
                .get(2)
                .and_then(|a| if a == "--agent" { args.get(3).map(|s| s.as_str()) } else { Some(a.as_str()) })
                .unwrap_or("claude-opus");
            hook_boot::run_boot(agent).await;
        }
        "hook-status" => hook_boot::run_status().await,
        "service" => {
            let subcmd = args.get(2).cloned().unwrap_or_default();
            if matches!(subcmd.as_str(), "help" | "--help" | "-h") {
                println!("{}", cli_service_usage());
                return;
            }
            let code = match tokio::task::spawn_blocking(move || match subcmd.as_str() {
                "install" => u8::from(service::install()),
                "uninstall" => u8::from(service::uninstall()),
                "start" => u8::from(service::start()),
                "stop" => u8::from(service::stop()),
                "status" => u8::from(service::status()),
                "ensure" => u8::from(service::ensure()),
                _ => { eprintln!("{}", cli_service_usage()); 1 }
            }).await {
                Ok(code) => code,
                Err(err) => { eprintln!("[cortex] Service command task failed: {err}"); 1 }
            };
            if code != 0 { std::process::exit(code as i32); }
        }
        "service-run" => service::dispatch_service(),
        "prompt-inject" => prompt_inject::run(&args[2..]).await,
        "setup" => {
            let remaining: Vec<String> = args[2..].to_vec();
            if remaining.iter().any(|a| a == "--team") {
                validate_cli_options_or_exit(&remaining, &["--owner", "--display-name"], &["--team", "--dry-run"]);
                setup::run_setup_team(&remaining, remaining.iter().any(|a| a == "--dry-run")).await;
            } else {
                if remaining.iter().any(|a| a == "--dry-run") {
                    eprintln!("--dry-run requires --team");
                    std::process::exit(1);
                }
                validate_cli_options_or_exit(&remaining, &[], &[]);
                setup::run_setup().await;
            }
        }
        "migrate" => {
            let remaining: Vec<String> = args[2..].to_vec();
            validate_cli_options_or_exit(&remaining, &["--owner", "--display-name"], &["--dry-run"]);
            setup::run_setup_team(&remaining, remaining.iter().any(|a| a == "--dry-run")).await;
        }
        "export" => run_export_cli(&paths, &args[2..]),
        "import" => run_import_cli(&paths, &args[2..]),
        "sync" => run_sync_cli(&paths, &args[2..]),
        "eval" => run_eval_cli(&paths, &args[2..]),
        "doctor" => {
            validate_cli_options_or_exit(&args[2..], &[], &[]);
            run_doctor_cli(&paths);
        }
        "reindex" => {
            validate_cli_options_or_exit(&args[2..], &[], &["--json"]);
            run_reindex_cli(&paths, args.iter().any(|a| a == "--json"));
        }
        "re-embed" | "reembed" => {
            let mut remaining: Vec<String> = args[2..].to_vec();
            if !remaining.iter().any(|arg| arg == "--until-exhausted") {
                remaining.push("--until-exhausted".to_string());
            }
            run_embeddings_drain_cli(&paths, &remaining).await;
        }
        "recrystallize" => run_recrystallize_cli(&paths, args.iter().any(|a| a == "--json")).await,
        "cleanup" => {
            validate_cli_options_or_exit(&args[2..], &["--max-passes"], &["--dry-run", "--events"]);
            let max_event_passes = match parse_flag_usize(&args[2..], "--max-passes") {
                Ok(Some(value)) => value.clamp(1, 12),
                Ok(None) => 3,
                Err(err) => { eprintln!("Error: {err}"); std::process::exit(1); }
            };
            run_cleanup_cli(&paths, args.iter().any(|a| a == "--dry-run"), args.iter().any(|a| a == "--events"), max_event_passes);
        }
        "embeddings" => run_embeddings_cli(&paths, &args[2..]).await,
        "backup" => {
            validate_cli_options_or_exit(&args[2..], &[], &[]);
            run_backup_cli(&paths);
        }
        "restore" => run_restore_cli(&paths, &args),
        "user" => run_user_cli(&paths, &args).await,
        "team" => run_team_cli(&paths, &args).await,
        "admin" => run_admin_cli(&paths, &args).await,
        other => {
            eprintln!("{}", unknown_cli_command_message(other));
            std::process::exit(1);
        }
    }
}
""")

print(f"Wrote {MAIN} ({len(MAIN.read_text().splitlines())} lines)")
