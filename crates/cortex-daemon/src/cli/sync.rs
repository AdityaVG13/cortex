use crate::auth;

use super::common::validate_cli_options_or_exit;

const SYNC_USAGE: &str = "Usage: cortex sync <export|import|watch> [options]";

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn validate_export_args(args: &[String]) {
    validate_cli_options_or_exit(args, &["--format", "--out", "--since"], &[]);
}

fn validate_import_args(args: &[String]) {
    validate_cli_options_or_exit(args, &["--file", "--user", "--visibility"], &["--dry-run"]);
}

pub fn run_sync_cli(_paths: &auth::CortexPaths, args: &[String]) {
    match args.first().map(String::as_str).unwrap_or("") {
        "export" => {
            validate_export_args(&args[1..]);
            fail("sync export requires the full sync feature in this build");
        }
        "import" => {
            validate_import_args(&args[1..]);
            fail("sync import requires the full sync feature in this build");
        }
        "watch" => {
            validate_cli_options_or_exit(&args[1..], &["--dir", "--interval-secs", "--out", "--since", "--user", "--visibility"], &["--once", "--dry-run"]);
            fail("sync watch requires the full sync feature in this build");
        }
        _ => fail(SYNC_USAGE),
    }
}

pub fn run_export_cli(_paths: &auth::CortexPaths, args: &[String]) {
    validate_export_args(args);
    fail("export requires the full sync feature in this build");
}

pub fn run_import_cli(_paths: &auth::CortexPaths, args: &[String]) {
    validate_import_args(args);
    fail("import requires the full sync feature in this build");
}
