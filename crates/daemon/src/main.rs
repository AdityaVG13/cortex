#![forbid(unsafe_code)]

use asupersync::{Cx, runtime::RuntimeBuilder};
use cortex_daemon::{cli, hook_boot, prompt_inject, setup};
use cortex_kernel::auth;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let paths = auth::CortexPaths::resolve_from_args(&args);
    auth::CortexPaths::install_process_paths(&paths);
    let runtime = match RuntimeBuilder::new().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("[cortex] runtime initialization failed: {err}");
            std::process::exit(1);
        }
    };
    let result = runtime.block_on(async {
        let cx = Cx::current().ok_or("runtime capability context unavailable")?;
        run(&cx, &paths, &args).await
    });
    if let Err(err) = result {
        eprintln!("[cortex] {err}");
        std::process::exit(1);
    }
}

async fn run(cx: &Cx, paths: &auth::CortexPaths, args: &[String]) -> Result<(), String> {
    let mode = args.get(1).map(String::as_str).unwrap_or("");
    let rest = args.get(2..).unwrap_or_default();
    match mode {
        "" | "help" | "--help" | "-h" => cli::print_usage_and_exit(0),
        "version" | "--version" | "-V" => println!("cortex {}", env!("CARGO_PKG_VERSION")),
        "capabilities" => {
            cli::validate_cli_options_or_exit(rest, &[], &["--json"]);
            if rest.iter().any(|arg| arg == "--json") {
                println!("{}", cli::cli_capabilities_payload());
            } else {
                println!("{}", cli::cli_capabilities_summary());
            }
        }
        "paths" => {
            cli::validate_cli_options_or_exit(rest, &[], &["--json"]);
            println!("{}", paths.to_json());
        }
        "status" => {
            cli::validate_cli_options_or_exit(rest, &[], &["--json"]);
            let code = cli::run_status_cli(paths, rest.iter().any(|arg| arg == "--json")).await;
            if code != 0 {
                std::process::exit(code);
            }
        }
        "mcp" => {
            cli::validate_cli_options_or_exit(rest, &["--agent"], &[]);
            cortex_daemon::mcp_native::run(cx, paths, cli::parse_flag_value(rest, "--agent").as_deref())
                .await
                .map_err(|err| err.to_string())?;
        }
        "plugin" if rest.first().is_some_and(|arg| arg == "mcp") => {
            cli::validate_cli_options_or_exit(&rest[1..], &["--agent"], &[]);
            cortex_daemon::mcp_native::run(cx, paths, cli::parse_flag_value(&rest[1..], "--agent").as_deref())
                .await
                .map_err(|err| err.to_string())?;
        }
        "capture" => cli::run_capture_cli(cx, paths, rest).await?,
        "boot" => cli::run_boot_cli(cx, paths, rest).await?,
        "op" => cli::run_op_cli(cx, paths, rest).await,
        "maintain" => cli::run_maintain_cli(cx, paths, rest).await,
        "serve" => {
            cli::validate_cli_options_or_exit(rest, &[], &[]);
            cli::run_daemon(cx, paths.clone(), shutdown_signal()).await?;
        }
        "hook-boot" => {
            cli::validate_cli_options_or_exit(rest, &["--agent"], &[]);
            hook_boot::run_boot(cx, cli::parse_flag_value(rest, "--agent").as_deref().unwrap_or("claude-code")).await
        }
        "hook-status" => {
            cli::validate_cli_options_or_exit(rest, &[], &[]);
            hook_boot::run_status(cx).await
        }
        "hook" => {
            cli::validate_cli_options_allowing_one_positional_or_exit(rest, &["--agent"], &[]);
            let kind = cli::first_positional(rest, &["--agent"]).unwrap_or("session_start");
            cortex_kernel::hook_event::run(cx, kind, cli::parse_flag_value(rest, "--agent").as_deref().unwrap_or("claude-code")).await?;
        }
        "prompt-inject" => prompt_inject::run(cx, rest).await,
        "setup" => {
            cli::validate_cli_options_or_exit(rest, &[], &[]);
            setup::run_setup(cx).await;
        }
        "doctor" => cli::run_doctor_cli(paths),
        "backup" => {
            cli::validate_cli_options_or_exit(rest, &[], &[]);
            cli::run_backup_cli(paths);
        }
        "cleanup" => {
            cli::validate_cli_options_or_exit(rest, &["--max-passes"], &["--dry-run", "--events"]);
            let passes = cli::parse_flag_usize(rest, "--max-passes")?.unwrap_or(3).clamp(1, 12);
            cli::run_cleanup_cli(paths, rest.iter().any(|arg| arg == "--dry-run"), rest.iter().any(|arg| arg == "--events"), passes);
        }
        "restore" => cli::run_restore_cli(paths, args),
        "reindex" => {
            cli::validate_cli_options_or_exit(rest, &[], &["--json"]);
            cli::run_reindex_cli(paths, rest.iter().any(|arg| arg == "--json"));
        }
        "rebuild-anchors" => cli::run_rebuild_anchors_cli(cx, paths, rest).await,
        "recrystallize" => cli::run_recrystallize_cli(cx, paths, rest.iter().any(|arg| arg == "--json")).await,
        "embeddings" => cli::run_embeddings_cli(cx, paths, rest).await,
        "eval" => cli::run_eval_cli(paths, rest),
        "robot-docs" => println!("{}", cli::cli_robot_docs_guide()),
        other => return Err(cli::unknown_cli_command_message(other)),
    }
    Ok(())
}

async fn shutdown_signal() {
    let interrupt = asupersync::signal::ctrl_c();
    let terminate = async {
        match asupersync::signal::signal(asupersync::signal::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                eprintln!("[cortex] SIGTERM registration failed: {err}");
                std::future::pending::<()>().await;
            }
        }
    };
    let _ = futures_util::future::select(Box::pin(interrupt), Box::pin(terminate)).await;
}
