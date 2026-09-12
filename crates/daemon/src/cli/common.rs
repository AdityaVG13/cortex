use crate::auth;
use crate::crystallize;
use crate::db;
use std::path::Path;

pub fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|idx| args.get(idx + 1)).cloned()
}

pub fn parse_flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        if args[i] == flag {
            if let Some(value) = args.get(i + 1) {
                if !is_cli_option_token(value) && !value.trim().is_empty() {
                    values.push(value.clone());
                }
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    values
}
const GLOBAL_VALUE_FLAGS: &[&str] = &["--home", "--db"];
pub(crate) fn is_cli_option_token(value: &str) -> bool {
    value.starts_with("--")
}
pub(crate) fn validate_cli_options(args: &[String], value_flags: &[&str], boolean_flags: &[&str]) -> Result<(), String> {
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if value_flags.contains(&arg) || GLOBAL_VALUE_FLAGS.contains(&arg) {
            let Some(value) = args.get(i + 1) else {
                return Err(format!("Missing value for {arg}"));
            };
            if is_cli_option_token(value) {
                return Err(format!("Missing value for {arg}"));
            }
            i += 2;
            continue;
        }
        if boolean_flags.contains(&arg) {
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            return Err(format!("Unknown option: {arg}"));
        }
        return Err(format!("Unexpected argument: {arg}"));
    }
    Ok(())
}
pub fn validate_cli_options_or_exit(args: &[String], value_flags: &[&str], boolean_flags: &[&str]) {
    if let Err(err) = validate_cli_options(args, value_flags, boolean_flags) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

pub fn apply_path_env(paths: &auth::CortexPaths) {
    // SAFETY: `apply_path_env` runs during single-threaded CLI startup, before
    // any runtime threads are spawned, so no other thread can read the
    // environment concurrently (`std::env::set_var` contract:
    // https://doc.rust-lang.org/std/env/fn.set_var.html).
    //
    // Classification (C) REFACTORABLE -- not (A). `set_var` is unsafe in
    // edition 2024 because env mutation races with concurrent `var`/`env`
    // reads; that is a soundness obligation for THIS call, not a proof that
    // env mutation is required. `main` already threads `CortexPaths` into
    // `run`. Alternatives that succeed: (1) stop writing CORTEX_HOME /
    // CORTEX_DB here and make remaining `env::var` readers take paths;
    // (2) set the vars only on child `Command`s that must inherit them;
    // (3) a Cortex-owned OnceLock path cell instead of the process env.
    // Later pass: set_var redesign. Do not execute in this classify pass.
    unsafe {
        std::env::set_var("CORTEX_HOME", &paths.home);
        std::env::set_var("CORTEX_DB", &paths.db);
    }
}
pub fn parse_flag_usize(args: &[String], flag: &str) -> Result<Option<usize>, String> {
    let Some(idx) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    let raw = args.get(idx + 1).ok_or_else(|| format!("missing value for {flag}"))?;
    if is_cli_option_token(raw) {
        return Err(format!("missing value for {flag}"));
    }
    let value = raw.parse::<usize>().map_err(|_| format!("invalid value for {flag}: '{raw}'"))?;
    if value == 0 {
        return Err(format!("{flag} must be >= 1"));
    }
    Ok(Some(value))
}
pub(crate) fn open_cli_connection(db_path: &Path) -> Result<rusqlite::Connection, String> {
    let conn = db::open(db_path).map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;
    db::configure(&conn).map_err(|e| format!("Failed to configure database: {e}"))?;
    db::initialize_schema(&conn).map_err(|e| format!("Failed to initialize schema: {e}"))?;
    db::run_pending_migrations_quiet(&conn);
    crystallize::migrate_crystal_tables(&conn);
    Ok(conn)
}
pub(crate) fn json_str(val: &serde_json::Value, key: &str) -> String {
    val.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
