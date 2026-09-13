use crate::crystallize;
use crate::db;
use std::path::Path;

pub fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    let mut i = 0usize;
    while i < args.len() {
        if args[i] == flag {
            if let Some((value, _)) = take_flag_value(args, i) {
                return Some(value);
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    None
}

pub fn parse_flag_values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        if args[i] == flag {
            if let Some((value, next)) = take_flag_value(args, i) {
                values.push(value);
                i = next;
                continue;
            }
            i += 1;
            continue;
        }
        i += 1;
    }
    values
}
const GLOBAL_VALUE_FLAGS: &[&str] = &["--home", "--db"];
pub(crate) fn is_cli_option_token(value: &str) -> bool {
    value.starts_with("--")
}

/// Value after a flag. `--` quotes the next token so a path or name may start with `--`.
/// Returns `(value, index_after_consumed_tokens)`.
fn take_flag_value(args: &[String], flag_index: usize) -> Option<(String, usize)> {
    let Some(value) = args.get(flag_index + 1) else {
        return None;
    };
    if value == "--" {
        let explicit = args.get(flag_index + 2)?;
        if explicit.trim().is_empty() {
            return None;
        }
        return Some((explicit.clone(), flag_index + 3));
    }
    if is_cli_option_token(value) || value.trim().is_empty() {
        return None;
    }
    Some((value.clone(), flag_index + 2))
}

/// Drop `--home` / `--db` pairs so a nested subcommand is not eaten as a flag value.
pub(crate) fn without_global_value_flags(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0usize;
    while i < args.len() {
        if GLOBAL_VALUE_FLAGS.contains(&args[i].as_str()) {
            if let Some((_, next)) = take_flag_value(args, i) {
                i = next;
            } else {
                i += 1;
            }
            continue;
        }
        out.push(args[i].clone());
        i += 1;
    }
    out
}

/// First non-flag token, skipping known value-flag pairs (so `hook --agent x Kind` still finds Kind).
pub fn first_positional<'a>(args: &'a [String], value_flags: &[&str]) -> Option<&'a str> {
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if value_flags.contains(&arg) || GLOBAL_VALUE_FLAGS.contains(&arg) {
            if let Some((_, next)) = take_flag_value(args, i) {
                i = next;
            } else {
                i += 1;
            }
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(arg);
    }
    None
}

pub fn validate_cli_options_allowing_one_positional_or_exit(args: &[String], value_flags: &[&str], boolean_flags: &[&str]) {
    let mut flags = Vec::new();
    let mut i = 0usize;
    let mut skipped_positional = false;
    while i < args.len() {
        let arg = args[i].as_str();
        if value_flags.contains(&arg) || GLOBAL_VALUE_FLAGS.contains(&arg) {
            if let Some((_, next)) = take_flag_value(args, i) {
                flags.extend(args[i..next].iter().cloned());
                i = next;
            } else {
                flags.push(args[i].clone());
                i += 1;
            }
            continue;
        }
        if boolean_flags.contains(&arg) || arg.starts_with('-') {
            flags.push(args[i].clone());
            i += 1;
            continue;
        }
        if !skipped_positional {
            skipped_positional = true;
            i += 1;
            continue;
        }
        flags.push(args[i].clone());
        i += 1;
    }
    validate_cli_options_or_exit(&flags, value_flags, boolean_flags);
}
pub(crate) fn validate_cli_options(args: &[String], value_flags: &[&str], boolean_flags: &[&str]) -> Result<(), String> {
    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_str();
        if value_flags.contains(&arg) || GLOBAL_VALUE_FLAGS.contains(&arg) {
            let Some((_, next)) = take_flag_value(args, i) else {
                return Err(format!("Missing value for {arg}"));
            };
            i = next;
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

pub fn parse_flag_usize(args: &[String], flag: &str) -> Result<Option<usize>, String> {
    let Some(idx) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    let Some((raw, _)) = take_flag_value(args, idx) else {
        return Err(format!("missing value for {flag}"));
    };
    let value = raw.parse::<usize>().map_err(|_| format!("invalid value for {flag}: '{raw}'"))?;
    if value == 0 {
        return Err(format!("{flag} must be >= 1"));
    }
    Ok(Some(value))
}
pub(crate) fn open_cli_connection(db_path: &Path) -> Result<rusqlite::Connection, String> {
    let mut conn = db::open(db_path).map_err(|e| format!("Failed to open database at {}: {e}", db_path.display()))?;
    db::configure(&conn).map_err(|e| format!("Failed to configure database: {e}"))?;
    db::initialize_schema(&conn).map_err(|e| format!("Failed to initialize schema: {e}"))?;
    db::run_pending_migrations_quiet(&mut conn);
    crystallize::migrate_crystal_tables(&conn);
    Ok(conn)
}
pub(crate) fn json_str(val: &serde_json::Value, key: &str) -> String {
    val.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
