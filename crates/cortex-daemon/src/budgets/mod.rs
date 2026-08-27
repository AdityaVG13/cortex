use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
const BUDGETS_FILE_NAME: &str = "budgets.toml";
pub const BUDGET_SOURCE: &str = "budgets.toml";
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum BudgetEndpoint {
    Store,
    Recall,
    Boot,
    Mcp,
}
impl BudgetEndpoint {
    pub const fn all() -> &'static [BudgetEndpoint] {
        &[BudgetEndpoint::Store, BudgetEndpoint::Recall, BudgetEndpoint::Boot, BudgetEndpoint::Mcp]
    }
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetEndpoint::Store => "store",
            BudgetEndpoint::Recall => "recall",
            BudgetEndpoint::Boot => "boot",
            BudgetEndpoint::Mcp => "mcp",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "store" => Some(Self::Store),
            "recall" => Some(Self::Recall),
            "boot" => Some(Self::Boot),
            "mcp" => Some(Self::Mcp),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointBudget {
    pub limit: usize,
    pub window_seconds: u64,
}
impl EndpointBudget {
    fn to_health_json(self) -> Value {
        json!({"limit":self.
limit,"windowSeconds":self.window_seconds,"window_seconds":self.window_seconds})
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetConfig {
    pub enabled: bool,
    endpoints: BTreeMap<BudgetEndpoint, EndpointBudget>,
}
impl BudgetConfig {
    pub fn parse_toml_str(contents: &str) -> Result<Self, BudgetConfigError> {
        let raw: RawBudgetFile =
            toml::from_str(contents).map_err(|error| BudgetConfigError::new("parse_error", format!("failed to parse budgets.toml: {error}"), None, None))?;
        let enabled = raw.defaults.and_then(|defaults| defaults.enabled).unwrap_or(true);
        let mut endpoints = BTreeMap::new();
        for (name, raw_budget) in raw.endpoints.unwrap_or_default() {
            let endpoint = BudgetEndpoint::parse(&name)
                .ok_or_else(|| BudgetConfigError::new("unknown_endpoint", format!("unknown budget endpoint: {name}"), Some(name.clone()), None))?;
            let limit = raw_budget.limit.ok_or_else(|| {
                BudgetConfigError::new("missing_limit", format!("budget endpoint {name} is missing limit"), Some(name.clone()), Some("limit"))
            })?;
            if limit <= 0 {
                return Err(BudgetConfigError::new(
                    "invalid_limit",
                    format!("budget endpoint {name} limit must be a positive integer"),
                    Some(name.clone()),
                    Some("limit"),
                ));
            }
            let window_seconds = raw_budget.window_seconds.ok_or_else(|| {
                BudgetConfigError::new(
                    "missing_window_seconds",
                    format!("budget endpoint {name} is missing window_seconds"),
                    Some(name.clone()),
                    Some("window_seconds"),
                )
            })?;
            if window_seconds <= 0 {
                return Err(BudgetConfigError::new(
                    "invalid_window_seconds",
                    format!("budget endpoint {name} window_seconds must be a positive integer"),
                    Some(name.clone()),
                    Some("window_seconds"),
                ));
            }
            endpoints.insert(endpoint, EndpointBudget { limit: limit as usize, window_seconds: window_seconds as u64 });
        }
        Ok(Self { enabled, endpoints })
    }
    pub fn budget_for(&self, endpoint: BudgetEndpoint) -> Option<EndpointBudget> {
        self.endpoints.get(&endpoint).copied()
    }
    fn endpoints_json(&self) -> Value {
        let mut map = serde_json::Map::new();
        for endpoint in BudgetEndpoint::all() {
            if let Some(budget) = self.endpoints.get(endpoint) {
                map.insert(endpoint.as_str().to_string(), budget.to_health_json());
            }
        }
        Value::Object(map)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetConfigError {
    pub code: String,
    pub message: String,
    pub endpoint: Option<String>,
    pub field: Option<String>,
}
impl BudgetConfigError {
    fn new(code: impl Into<String>, message: impl Into<String>, endpoint: Option<String>, field: Option<&str>) -> Self {
        Self { code: code.into(), message: message.into(), endpoint, field: field.map(str::to_string) }
    }
    fn to_json(&self) -> Value {
        json!({"code":self.code,"message":self.message,"endpoint":self.endpoint,
"field":self.field})
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetConfigStatus {
    pub config_loaded: bool,
    pub source: PathBuf,
    pub config: Option<BudgetConfig>,
    pub error: Option<BudgetConfigError>,
}
impl BudgetConfigStatus {
    pub fn load_from_home(home: &Path) -> Self {
        Self::load_from_path(home.join(BUDGETS_FILE_NAME))
    }
    pub fn load_from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        match std::fs::read_to_string(&path) {
            Ok(contents) => Self::from_contents(path, &contents),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self { config_loaded: false, source: path, config: None, error: None },
            Err(error) => Self {
                config_loaded: true,
                source: path,
                config: None,
                error: Some(BudgetConfigError::new("io_error", format!("failed to read budgets.toml: {error}"), None, None)),
            },
        }
    }
    pub fn missing_for_tests() -> Self {
        Self { config_loaded: false, source: PathBuf::from(BUDGETS_FILE_NAME), config: None, error: None }
    }
    fn from_contents(source: PathBuf, contents: &str) -> Self {
        match BudgetConfig::parse_toml_str(contents) {
            Ok(config) => Self { config_loaded: true, source, config: Some(config), error: None },
            Err(error) => Self { config_loaded: true, source, config: None, error: Some(error) },
        }
    }
    pub fn enabled(&self) -> bool {
        self.error.is_none() && self.config.as_ref().map(|config| config.enabled).unwrap_or(false)
    }
    pub fn budget_for(&self, endpoint: BudgetEndpoint) -> Option<EndpointBudget> {
        if !self.enabled() {
            return None;
        }
        self.config.as_ref().and_then(|config| config.budget_for(endpoint))
    }
    pub fn to_health_json(&self, recent_denials: usize) -> Value {
        json!({"configLoaded":self.config_loaded,
"config_loaded":self.config_loaded,"enabled":self.enabled(),"source":BUDGET_SOURCE,"error":self.error.as_ref().map(
BudgetConfigError::to_json),"endpoints":self.config.as_ref().map(BudgetConfig::endpoints_json).unwrap_or_else(||json!({})),
"recentDenials":recent_denials,"recent_denials":recent_denials})
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetDecision {
    pub allowed: bool,
    pub endpoint: BudgetEndpoint,
    pub limit: usize,
    pub window_seconds: u64,
    pub retry_after_seconds: u64,
    pub remaining: Option<usize>,
}
impl BudgetDecision {
    pub fn allowed(endpoint: BudgetEndpoint, budget: EndpointBudget, remaining: usize) -> Self {
        Self {
            allowed: true,
            endpoint,
            limit: budget.limit,
            window_seconds: budget.window_seconds,
            retry_after_seconds: 0,
            remaining: Some(remaining),
        }
    }
    pub fn denied(endpoint: BudgetEndpoint, budget: EndpointBudget, retry_after: u64) -> Self {
        Self {
            allowed: false,
            endpoint,
            limit: budget.limit,
            window_seconds: budget.window_seconds,
            retry_after_seconds: retry_after,
            remaining: Some(0),
        }
    }
    pub fn http_body_json(&self) -> Value {
        json!({"error":
"budget_exceeded","endpoint":self.endpoint.as_str(),"limit":self.limit,"window_seconds":self.window_seconds,"retry_after_seconds":
self.retry_after_seconds,"source":BUDGET_SOURCE})
    }
    pub fn event_json(&self, request_source: &str, source_ip: &str) -> Value {
        json!({
"endpoint":self.endpoint.as_str(),"limit":self.limit,"window_seconds":self.window_seconds,"retry_after_seconds":self.
retry_after_seconds,"source":BUDGET_SOURCE,"request_source":request_source,"source_ip":source_ip})
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudgetFile {
    defaults: Option<RawDefaults>,
    endpoints: Option<HashMap<String, RawEndpointBudget>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    enabled: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndpointBudget {
    limit: Option<i64>,
    window_seconds: Option<i64>,
}

