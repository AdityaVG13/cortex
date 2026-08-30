use crate::constants::*;
use crate::daemon::paths::{default_cortex_dir, resolved_cortex_paths};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetConfigDraft {
    enabled: bool,
    endpoints: Vec<BudgetEndpointDraft>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetEndpointDraft {
    endpoint: String,
    enabled: bool,
    limit: Option<u64>,
    window_seconds: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetConfigSnapshot {
    config_loaded: bool,
    enabled: bool,
    source: String,
    error: Option<BudgetConfigErrorSnapshot>,
    endpoints: BTreeMap<String, BudgetEndpointSnapshot>,
    recent_denials: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetEndpointSnapshot {
    limit: u64,
    window_seconds: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetConfigErrorSnapshot {
    code: String,
    message: String,
    endpoint: Option<String>,
    field: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudgetFile {
    defaults: Option<RawBudgetDefaults>,
    endpoints: Option<BTreeMap<String, RawBudgetEndpoint>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudgetDefaults {
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudgetEndpoint {
    limit: Option<i64>,
    window_seconds: Option<i64>,
}

#[derive(Debug, Serialize)]
struct BudgetTomlFile {
    defaults: BudgetTomlDefaults,
    endpoints: BTreeMap<String, BudgetTomlEndpoint>,
}

#[derive(Debug, Serialize)]
struct BudgetTomlDefaults {
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct BudgetTomlEndpoint {
    limit: u64,
    window_seconds: u64,
}

pub fn budget_config_path() -> Result<PathBuf, String> {
    let home = resolved_cortex_paths().home.or_else(|| default_cortex_dir().ok()).ok_or_else(|| "Could not resolve Cortex home path".to_string())?;
    fs::create_dir_all(&home).map_err(|err| format!("Failed to create Cortex home {}: {err}", home.display()))?;
    let canonical_home = fs::canonicalize(&home).map_err(|err| format!("Failed to resolve Cortex home {}: {err}", home.display()))?;
    Ok(canonical_home.join(BUDGETS_FILE_NAME))
}

fn budget_error(code: &str, message: impl Into<String>, endpoint: Option<String>, field: Option<&str>) -> BudgetConfigErrorSnapshot {
    BudgetConfigErrorSnapshot { code: code.to_string(), message: message.into(), endpoint, field: field.map(str::to_string) }
}

fn parse_budget_endpoint_name(name: &str) -> Option<String> {
    let normalized = name.trim().to_ascii_lowercase();
    BUDGET_ENDPOINT_NAMES.contains(&normalized.as_str()).then_some(normalized)
}

fn budget_source_label() -> String {
    BUDGETS_FILE_NAME.to_string()
}

fn empty_budget_snapshot(_path: &Path) -> BudgetConfigSnapshot {
    BudgetConfigSnapshot { config_loaded: false, enabled: false, source: budget_source_label(), error: None, endpoints: BTreeMap::new(), recent_denials: 0 }
}

pub fn budget_snapshot_from_contents(_path: &Path, contents: &str) -> BudgetConfigSnapshot {
    let parsed = match toml::from_str::<RawBudgetFile>(contents) {
        Ok(parsed) => parsed,
        Err(err) => {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error("parse_error", format!("failed to parse budgets.toml: {err}"), None, None)),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        }
    };

    let enabled = parsed.defaults.and_then(|defaults| defaults.enabled).unwrap_or(true);
    let mut endpoints = BTreeMap::new();
    for (raw_name, raw_budget) in parsed.endpoints.unwrap_or_default() {
        let Some(endpoint) = parse_budget_endpoint_name(&raw_name) else {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error("unknown_endpoint", format!("unknown budget endpoint: {raw_name}"), Some(raw_name), None)),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        };
        let Some(limit) = raw_budget.limit else {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error("missing_limit", format!("budget endpoint {endpoint} is missing limit"), Some(endpoint), Some("limit"))),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        };
        if limit <= 0 {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error(
                    "invalid_limit",
                    format!("budget endpoint {endpoint} limit must be a positive integer"),
                    Some(endpoint),
                    Some("limit"),
                )),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        }

        let Some(window_seconds) = raw_budget.window_seconds else {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error(
                    "missing_window_seconds",
                    format!("budget endpoint {endpoint} is missing window_seconds"),
                    Some(endpoint),
                    Some("window_seconds"),
                )),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        };
        if window_seconds <= 0 {
            return BudgetConfigSnapshot {
                config_loaded: true,
                enabled: false,
                source: budget_source_label(),
                error: Some(budget_error(
                    "invalid_window_seconds",
                    format!("budget endpoint {endpoint} window_seconds must be a positive integer"),
                    Some(endpoint),
                    Some("window_seconds"),
                )),
                endpoints: BTreeMap::new(),
                recent_denials: 0,
            };
        }

        endpoints.insert(endpoint, BudgetEndpointSnapshot { limit: limit as u64, window_seconds: window_seconds as u64 });
    }

    BudgetConfigSnapshot { config_loaded: true, enabled, source: budget_source_label(), error: None, endpoints, recent_denials: 0 }
}

pub fn read_budget_config_snapshot(path: &Path) -> Result<BudgetConfigSnapshot, String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(budget_snapshot_from_contents(path, &contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(empty_budget_snapshot(path)),
        Err(err) => Ok(BudgetConfigSnapshot {
            config_loaded: true,
            enabled: false,
            source: budget_source_label(),
            error: Some(budget_error("io_error", format!("failed to read budgets.toml: {err}"), None, None)),
            endpoints: BTreeMap::new(),
            recent_denials: 0,
        }),
    }
}

pub fn validate_budget_draft(draft: BudgetConfigDraft) -> Result<BudgetTomlFile, String> {
    let mut endpoints = BTreeMap::new();
    for raw in draft.endpoints {
        if !raw.enabled {
            continue;
        }
        let endpoint = parse_budget_endpoint_name(&raw.endpoint).ok_or_else(|| format!("Unknown budget endpoint: {}", raw.endpoint))?;
        if endpoints.contains_key(&endpoint) {
            return Err(format!("Duplicate budget endpoint: {endpoint}"));
        }

        let limit = raw.limit.ok_or_else(|| format!("Budget endpoint {endpoint} is missing limit"))?;
        if limit == 0 || limit > MAX_BUDGET_INTEGER {
            return Err(format!("Budget endpoint {endpoint} limit must be between 1 and {MAX_BUDGET_INTEGER}"));
        }

        let window_seconds = raw.window_seconds.ok_or_else(|| format!("Budget endpoint {endpoint} is missing window_seconds"))?;
        if window_seconds == 0 || window_seconds > MAX_BUDGET_INTEGER {
            return Err(format!("Budget endpoint {endpoint} window_seconds must be between 1 and {MAX_BUDGET_INTEGER}"));
        }

        endpoints.insert(endpoint, BudgetTomlEndpoint { limit, window_seconds });
    }

    Ok(BudgetTomlFile { defaults: BudgetTomlDefaults { enabled: draft.enabled }, endpoints })
}

pub fn write_budget_config_file(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| format!("Invalid budget config path: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    let temp_path = parent.join(format!(".{}.{}.tmp", BUDGETS_FILE_NAME, std::process::id()));
    {
        let mut file = File::create(&temp_path).map_err(|err| format!("Failed to create {}: {err}", temp_path.display()))?;
        file.write_all(contents.as_bytes()).map_err(|err| format!("Failed to write {}: {err}", temp_path.display()))?;
        file.sync_all().map_err(|err| format!("Failed to flush {}: {err}", temp_path.display()))?;
    }

    if path.exists() {
        fs::remove_file(path).map_err(|err| format!("Failed to replace {}: {err}", path.display()))?;
    }
    fs::rename(&temp_path, path).map_err(|err| {
        let _ = fs::remove_file(&temp_path);
        format!("Failed to save {}: {err}", path.display())
    })?;
    Ok(())
}

pub fn save_budget_from_draft(draft: BudgetConfigDraft) -> Result<BudgetConfigSnapshot, String> {
    let path = budget_config_path()?;
    let config = validate_budget_draft(draft)?;
    let contents = toml::to_string_pretty(&config).map_err(|err| format!("Failed to serialize budget config: {err}"))?;
    write_budget_config_file(&path, &contents)?;
    read_budget_config_snapshot(&path)
}

#[cfg(test)]
#[path = "../../../../../tests/control-center/rust/budget.rs"]
mod tests;
