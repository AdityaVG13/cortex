use crate::auth;
use crate::crystallize;
use crate::db;
use crate::transport;
use std::path::Path;
use std::time::Duration;
pub(crate) const SINGLE_DAEMON_TEST_BYPASS_ENV: &str = "CORTEX_SINGLE_DAEMON_TEST_BYPASS";
pub(crate) fn read_auth_token(paths: &auth::CortexPaths) -> Result<String, String> {
    let token_path = paths.token.clone();
    std::fs::read_to_string(&token_path)
        .map(|v| v.trim().to_string())
        .map_err(|_| format!("Cannot read auth token at {}. Is the daemon running?", token_path.display()))
}
pub(crate) fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
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
pub(crate) fn validate_cli_options_or_exit(args: &[String], value_flags: &[&str], boolean_flags: &[&str]) {
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
pub(crate) fn local_daemon_base_url(paths: &auth::CortexPaths) -> String {
    transport::local_http_base_url(paths)
}
pub(crate) fn is_local_client_base_url(base_url: &str, paths: &auth::CortexPaths) -> bool {
    transport::is_local_http_base_url(base_url, paths)
}
pub(crate) fn resolve_client_target_inputs(
    override_url: Option<&str>, override_api_key: Option<&str>, env_base_url: Option<&str>, env_api_key: Option<&str>, default_base_url: &str,
) -> (String, Option<String>, bool) {
    let resolved_base_url = normalize_option(override_url).or_else(|| normalize_option(env_base_url));
    let resolved_api_key = normalize_option(override_api_key).or_else(|| normalize_option(env_api_key));
    let local_owner_mode = resolved_base_url.is_none() && resolved_api_key.is_none();
    let base_url = resolved_base_url.unwrap_or_else(|| default_base_url.to_string());
    (base_url, resolved_api_key, local_owner_mode)
}
pub(crate) fn resolve_client_target(args: &[String], paths: &auth::CortexPaths) -> (String, Option<String>, bool) {
    let override_url = parse_flag_value(args, "--url");
    let override_api_key = parse_flag_value(args, "--api-key");
    let env_base_url = env_trimmed("CORTEX_API_BASE").or_else(|| env_trimmed("CORTEX_BASE_URL"));
    let env_api_key = env_trimmed("CORTEX_API_KEY");
    resolve_client_target_inputs(
        override_url.as_deref(),
        override_api_key.as_deref(),
        env_base_url.as_deref(),
        env_api_key.as_deref(),
        &local_daemon_base_url(paths),
    )
}
pub(crate) fn ensure_remote_target_has_api_key(base_url: &str, api_key: Option<&str>, paths: &auth::CortexPaths) -> Result<(), String> {
    let parsed = reqwest::Url::parse(base_url).map_err(|_| format!("Invalid Cortex target URL '{base_url}'. Use an absolute http:// or https:// URL."))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!("Unsupported Cortex target URL scheme '{}' in '{base_url}'. Use http or https.", parsed.scheme()));
    }
    if parsed.host_str().is_none() {
        return Err(format!("Invalid Cortex target URL '{base_url}': missing host."));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Cortex target URL must not include embedded credentials; pass --api-key instead.".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("Cortex target URL must not include query parameters or fragments.".to_string());
    }
    if api_key.is_none() && !is_local_client_base_url(base_url, paths) {
        return Err(format!("Remote Cortex target '{base_url}' requires an API key. Pass --api-key <key> or set CORTEX_API_KEY."));
    }
    Ok(())
}
pub(crate) fn apply_path_env(paths: &auth::CortexPaths) {
    std::env::set_var("CORTEX_HOME", &paths.home);
    std::env::set_var("CORTEX_DB", &paths.db);
    std::env::set_var("CORTEX_PORT", paths.port.to_string());
    std::env::set_var("CORTEX_BIND", &paths.bind);
    match &paths.ipc_endpoint {
        Some(endpoint) => std::env::set_var("CORTEX_IPC_ENDPOINT", endpoint),
        None => std::env::remove_var("CORTEX_IPC_ENDPOINT"),
    }
}
pub(crate) fn parse_flag_usize(args: &[String], flag: &str) -> Result<Option<usize>, String> {
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
pub(crate) async fn admin_request(paths: &auth::CortexPaths, method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let token = read_auth_token(paths)?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| format!("create admin client: {e}"))?;
    let base_url = local_daemon_base_url(paths);
    let payload = body.map(|value| value.to_string());
    let mut headers = vec![("authorization".to_string(), format!("Bearer {token}")), ("x-cortex-request".to_string(), "true".to_string())];
    if payload.is_some() {
        headers.push(("content-type".to_string(), "application/json".to_string()));
    }
    let (status, body_text) =
        transport::request_with_local_ipc_fallback(&client, method, &base_url, path, paths, &headers, payload.as_deref(), Duration::from_secs(10))
            .await
            .map_err(|e| {
                if e.to_ascii_lowercase().contains("connect") {
                    "Cortex daemon not running. Start with: cortex serve".to_string()
                } else {
                    format!("Request failed: {e}")
                }
            })?;
    if status.as_u16() == 403 {
        return Err("Admin commands require team mode. Run: cortex setup --team".to_string());
    }
    if status.as_u16() == 404 {
        return Err("Endpoint not found. Is the daemon up to date?".to_string());
    }
    let json: serde_json::Value = serde_json::from_str(&body_text).map_err(|_| {
        if body_text.is_empty() {
            format!("Empty response from daemon (HTTP {status})")
        } else {
            format!("Unexpected response (HTTP {status}): {body_text}")
        }
    })?;
    if !status.is_success() {
        let msg = json.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
        return Err(msg.to_string());
    }
    Ok(json)
}
pub(crate) fn json_str(val: &serde_json::Value, key: &str) -> String {
    val.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
