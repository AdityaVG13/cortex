use crate::auth;
use crate::crystallize;
pub(crate) const SINGLE_DAEMON_TEST_BYPASS_ENV: &str = "CORTEX_SINGLE_DAEMON_TEST_BYPASS";
use crate::db;
use std::path::Path;

pub fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|idx| args.get(idx + 1)).cloned()
}
const GLOBAL_VALUE_FLAGS: &[&str] = &["--home", "--db", "--port", "--bind"];
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
pub(crate) fn required_cli_positional_or_exit(args: &[String], index: usize, usage: &str) -> String {
    match args.get(index) {
        Some(value) if !is_cli_option_token(value) => value.clone(),
        _ => {
            eprintln!("{usage}");
            std::process::exit(1);
        }
    }
}
pub(crate) fn env_trimmed(key: &str) -> Option<String> {
    std::env::var(key).ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty())
}
pub(crate) fn parse_truthy_flag(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}
pub(crate) fn single_daemon_test_bypass_enabled() -> bool {
    cfg!(debug_assertions) && std::env::var(SINGLE_DAEMON_TEST_BYPASS_ENV).ok().is_some_and(|value| parse_truthy_flag(&value))
}
pub(crate) fn normalize_option(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

pub fn apply_path_env(paths: &auth::CortexPaths) {
    // SAFETY: `apply_path_env` runs during single-threaded CLI startup, before
    // any runtime threads are spawned, so no other thread can read the
    // environment concurrently.
    unsafe {
        std::env::set_var("CORTEX_HOME", &paths.home);
        std::env::set_var("CORTEX_DB", &paths.db);
        std::env::set_var("CORTEX_PORT", paths.port.to_string());
        std::env::set_var("CORTEX_BIND", &paths.bind);
        match &paths.ipc_endpoint {
            Some(endpoint) => std::env::set_var("CORTEX_IPC_ENDPOINT", endpoint),
            None => std::env::remove_var("CORTEX_IPC_ENDPOINT"),
        }
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
pub(crate) fn parse_env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|raw| raw.trim().parse::<usize>().ok()).filter(|value| *value > 0).unwrap_or(default)
}
pub(crate) fn parse_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|raw| raw.trim().parse::<u64>().ok()).filter(|value| *value > 0).unwrap_or(default)
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
