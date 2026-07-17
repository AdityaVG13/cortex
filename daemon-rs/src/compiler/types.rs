use serde_json::Value;
use std::env;
pub(crate) const DEFAULT_BOOT_MIN_SOURCE_TOKENS: usize = 40;
pub(crate) const DEFAULT_BOOT_MAX_SOURCE_TOKENS: usize = 600;
pub(crate) const DEFAULT_BOOT_RANK_TOP_N: usize = 5;
pub(crate) const SCORE_VARIANCE_FLAT_THRESHOLD: f64 = 0.0001;
pub(crate) const RANK_WEIGHT_CLASS: f64 = 0.30;
pub(crate) const RANK_WEIGHT_RECENCY: f64 = 0.30;
pub(crate) const RANK_WEIGHT_RELEVANCE: f64 = 0.25;
pub(crate) const RANK_WEIGHT_ACTIVITY: f64 = 0.15;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BootPackingMode {
    Auto,
    LegacyGreedy,
    ScoreAdaptive,
}
pub(crate) fn boot_packing_mode() -> BootPackingMode {
    match env::var("CORTEX_BOOT_PACKING_MODE").unwrap_or_default().trim().to_ascii_lowercase().as_str() {
        "legacy" | "greedy" | "v0.5" | "v0.5.0" => BootPackingMode::LegacyGreedy,
        "adaptive" | "score-adaptive" | "score_adaptive" => BootPackingMode::ScoreAdaptive,
        _ => BootPackingMode::Auto,
    }
}
pub(crate) fn detect_identity() -> String {
    let user = env::var("USERNAME").or_else(|_| env::var("USER")).unwrap_or_else(|_| "cortex-user".to_string());
    let platform = match env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    };
    let shell = env::var("SHELL")
        .or_else(|_| env::var("COMSPEC"))
        .map(|s| s.rsplit(['/', '\\']).next().unwrap_or(&s).to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    format!("User: {user}. Platform: {platform}. Shell: {shell}.")
}
pub struct BootResult {
    pub boot_prompt: String,
    pub token_estimate: usize,
    pub savings: Value,
    pub capsules: Vec<Value>,
}
