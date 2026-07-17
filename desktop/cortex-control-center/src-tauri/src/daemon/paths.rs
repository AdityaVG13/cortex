use crate::constants::*;
use crate::daemon::process::apply_hidden_process_flags;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn cortex_home() -> Result<PathBuf, String> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not resolve home directory".to_string())
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedCortexPaths {
    pub home: Option<PathBuf>,
    pub token: Option<PathBuf>,
    pub db: Option<PathBuf>,
    pub pid: Option<PathBuf>,
    pub port: Option<u16>,
    #[allow(dead_code)]
    pub bind: Option<String>,
}
pub fn default_cortex_dir() -> Result<PathBuf, String> {
    Ok(cortex_home()?.join(".cortex"))
}
pub(crate) fn token_path() -> Result<PathBuf, String> {
    resolved_cortex_paths().token.ok_or_else(|| "Could not resolve Cortex token path".to_string())
}

pub fn cortex_db_path() -> Result<PathBuf, String> {
    resolved_cortex_paths().db.ok_or_else(|| "Could not resolve Cortex database path".to_string())
}

pub fn daemon_port() -> u16 {
    resolve_daemon_port()
}
fn cortex_binary_name() -> &'static str {
    if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    }
}

fn normalized_path_for_guard(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
}

fn path_is_under_root(path: &Path, root: &Path) -> bool {
    let normalized_path = normalized_path_for_guard(path);
    let mut normalized_root = normalized_path_for_guard(root);
    if !normalized_root.ends_with('/') {
        normalized_root.push('/');
    }
    normalized_path == normalized_root.trim_end_matches('/') || normalized_path.starts_with(&normalized_root)
}

fn is_allowed_isolated_target_dir(segment: &str) -> bool {
    segment == DEV_DAEMON_TARGET_DIR || segment == RELEASE_DAEMON_TARGET_DIR
}

fn is_non_runtime_test_artifact_path(path: &Path) -> bool {
    let mut in_target_tree = false;

    for component in path.components() {
        let segment = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        if segment.is_empty() {
            continue;
        }

        if matches!(segment.as_str(), "target-tests" | "target-test" | "nextest") {
            return true;
        }

        if segment == "target" {
            in_target_tree = true;
            continue;
        }

        if segment.starts_with("target-") {
            if !is_allowed_isolated_target_dir(&segment) {
                return true;
            }
            in_target_tree = true;
            continue;
        }

        if in_target_tree && matches!(segment.as_str(), "deps" | "build" | "incremental") {
            return true;
        }
    }

    false
}

fn is_shared_workspace_debug_runtime_path(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    if file_name != cortex_binary_name().to_ascii_lowercase() {
        return false;
    }

    let segments: Vec<String> = path.components().map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    segments.windows(3).any(|window| window == ["daemon-rs", "target", "debug"])
}

pub fn is_disallowed_daemon_binary_path(path: &Path) -> bool {
    let normalized = normalized_path_for_guard(path);
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();

    if file_name.starts_with("cortex-daemon-run") {
        return true;
    }
    if normalized.contains("/daemon-lifecycle-runtime/") {
        return true;
    }
    if is_non_runtime_test_artifact_path(path) {
        return true;
    }
    if is_shared_workspace_debug_runtime_path(path) {
        return true;
    }

    let mut temp_roots = vec![std::env::temp_dir()];
    if let Ok(temp) = std::env::var("TEMP") {
        temp_roots.push(PathBuf::from(temp));
    }
    if let Ok(tmp) = std::env::var("TMP") {
        temp_roots.push(PathBuf::from(tmp));
    }
    temp_roots.iter().any(|root| !root.as_os_str().is_empty() && path_is_under_root(path, root))
}

pub fn workspace_binary_candidates(home: &Path, prefer_debug: bool) -> Vec<PathBuf> {
    let daemon_root = home.join("cortex").join("daemon-rs");
    let release_path = daemon_root.join("target").join("release").join(cortex_binary_name());
    let isolated_release_path = daemon_root.join(RELEASE_DAEMON_TARGET_DIR).join("release").join(cortex_binary_name());
    let isolated_debug_path = daemon_root.join(DEV_DAEMON_TARGET_DIR).join("debug").join(cortex_binary_name());

    if prefer_debug {
        vec![isolated_debug_path, isolated_release_path, release_path]
    } else {
        vec![isolated_release_path, release_path, isolated_debug_path]
    }
}

fn resolve_binary_on_path(binary_name: &str) -> Option<PathBuf> {
    let locator = if cfg!(windows) { "where.exe" } else { "which" };
    let output = Command::new(locator).arg(binary_name).output().ok()?;
    if !output.status.success() {
        return None;
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let candidate = PathBuf::from(line);
            if is_disallowed_daemon_binary_path(&candidate) {
                log_startup_path("resolve-binary-on-path", "reject-disallowed", &candidate.display().to_string());
                None
            } else {
                Some(candidate)
            }
        })
        .next()
}

pub fn path_binary_fallback_enabled_from_value(value: Option<&str>) -> bool {
    value
        .map(|raw| {
            let normalized = raw.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

fn path_binary_fallback_enabled() -> bool {
    path_binary_fallback_enabled_from_value(std::env::var(PATH_BINARY_FALLBACK_ENV).ok().as_deref())
}

pub fn service_ensure_fallback_enabled() -> bool {
    path_binary_fallback_enabled_from_value(std::env::var(SERVICE_ENSURE_FALLBACK_ENV).ok().as_deref())
}

fn parse_paths_json(output: &[u8]) -> Result<ResolvedCortexPaths, String> {
    let json: serde_json::Value = serde_json::from_slice(output).map_err(|err| format!("Invalid JSON from `cortex paths --json`: {err}"))?;
    let port = json
        .get("port")
        .and_then(|value| value.as_u64())
        .map(|value| u16::try_from(value).map_err(|err| format!("Port value out of range ({value}): {err}")))
        .transpose()?;

    Ok(ResolvedCortexPaths {
        home: json.get("home").and_then(|value| value.as_str()).map(PathBuf::from),
        token: json.get("token").and_then(|value| value.as_str()).map(PathBuf::from),
        db: json.get("db").and_then(|value| value.as_str()).map(PathBuf::from),
        pid: json.get("pid").and_then(|value| value.as_str()).map(PathBuf::from),
        port,
        bind: json.get("bind").and_then(|value| value.as_str()).map(|value| value.to_string()),
    })
}

fn resolve_paths_with_binary(binary: impl AsRef<std::ffi::OsStr>) -> Result<Option<ResolvedCortexPaths>, String> {
    let mut command = Command::new(binary);
    command.args(["paths", "--json"]);
    apply_hidden_process_flags(&mut command);
    let output = command.output().map_err(|err| format!("Failed to execute `cortex paths --json`: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Ok(None);
        }
        return Err(format!("`cortex paths --json` failed: {stderr}"));
    }
    parse_paths_json(&output.stdout).map(Some)
}

fn fallback_cortex_paths() -> ResolvedCortexPaths {
    let cortex_dir = env::var("CORTEX_HOME").ok().map(PathBuf::from).or_else(|| default_cortex_dir().ok());

    let port = match env::var("CORTEX_PORT") {
        Ok(value) => match value.parse::<u16>() {
            Ok(port) => Some(port),
            Err(err) => {
                eprintln!("[cortex-control-center] Invalid CORTEX_PORT '{value}': {err}");
                Some(DEFAULT_DAEMON_PORT)
            }
        },
        Err(env::VarError::NotPresent) => Some(DEFAULT_DAEMON_PORT),
        Err(err) => {
            eprintln!("[cortex-control-center] Failed to read CORTEX_PORT: {err}");
            Some(DEFAULT_DAEMON_PORT)
        }
    };

    ResolvedCortexPaths {
        home: cortex_dir.clone(),
        token: cortex_dir.as_ref().map(|dir| dir.join("cortex.token")),
        db: cortex_dir.as_ref().map(|dir| dir.join("cortex.db")),
        pid: cortex_dir.as_ref().map(|dir| dir.join("cortex.pid")),
        port,
        bind: env::var("CORTEX_BIND").ok().map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).or_else(|| Some("127.0.0.1".to_string())),
    }
}

pub fn resolved_cortex_paths() -> ResolvedCortexPaths {
    if let Some(binary) = find_cortex_binary() {
        match resolve_paths_with_binary(&binary) {
            Ok(Some(paths)) => return paths,
            Ok(None) => {}
            Err(err) => eprintln!("[cortex-control-center] {err}"),
        }
    }

    if path_binary_fallback_enabled() {
        if let Some(binary) = resolve_binary_on_path("cortex") {
            match resolve_paths_with_binary(&binary) {
                Ok(Some(paths)) => return paths,
                Ok(None) => {}
                Err(err) => eprintln!("[cortex-control-center] {err}"),
            }
        }
    }

    fallback_cortex_paths()
}

fn resolve_daemon_port() -> u16 {
    resolved_cortex_paths().port.unwrap_or(DEFAULT_DAEMON_PORT)
}

pub fn log_startup_path(context: &str, decision: &str, detail: &str) {
    eprintln!("[cortex-control-center] startup-path context={context} decision={decision} detail={detail}");
}

pub fn installed_plugin_binary_path(home: &Path) -> PathBuf {
    home.join(".cortex").join("bin").join(cortex_binary_name())
}

pub fn copy_if_changed(src: &Path, dest: &Path) -> Result<(), String> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => existing != fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(format!("read {}: {err}", dest.display())),
    };

    if needs_copy {
        fs::copy(src, dest).map_err(|e| format!("copy {} -> {}: {e}", src.display(), dest.display()))?;
    }

    Ok(())
}

pub fn find_cortex_binary() -> Option<PathBuf> {
    let sidecar_candidate = env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(cortex_binary_name())).filter(|path| path.exists()));

    if let Ok(home) = cortex_home() {
        let plugin_path = home.join(".cortex").join("bin").join(cortex_binary_name());
        let mut candidates = Vec::new();
        if cfg!(debug_assertions) {
            // In dev builds prefer the Control Center's isolated daemon target
            // before falling back to the shared workspace target. This avoids
            // unrelated `target/debug` activity (for example MCP client shims)
            // from silently hijacking lifecycle verification.
            for candidate in workspace_binary_candidates(&home, true) {
                if !candidate.exists() {
                    continue;
                }
                if is_disallowed_daemon_binary_path(&candidate) {
                    log_startup_path("find-cortex-binary", "reject-disallowed", &candidate.display().to_string());
                    continue;
                }
                return Some(candidate);
            }
            candidates.push(plugin_path);
            if let Some(sidecar) = sidecar_candidate.clone() {
                candidates.push(sidecar);
            }
        } else {
            if let Some(sidecar) = sidecar_candidate.clone() {
                candidates.push(sidecar);
            }
            candidates.push(plugin_path);
            candidates.extend(workspace_binary_candidates(&home, false));
        }

        for candidate in candidates {
            if !candidate.exists() {
                continue;
            }
            if is_disallowed_daemon_binary_path(&candidate) {
                log_startup_path("find-cortex-binary", "reject-disallowed", &candidate.display().to_string());
                continue;
            }
            return Some(candidate);
        }
    }

    if let Some(sidecar) = sidecar_candidate.filter(|candidate| !is_disallowed_daemon_binary_path(candidate)) {
        return Some(sidecar);
    }

    if path_binary_fallback_enabled() {
        return resolve_binary_on_path(cortex_binary_name());
    }

    None
}
