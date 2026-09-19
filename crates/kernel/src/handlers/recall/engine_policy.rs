use super::{
    DEFAULT_RECALL_BUDGET_BALANCED, DEFAULT_RECALL_BUDGET_DEEP, DEFAULT_RECALL_BUDGET_FAST,
    DEFAULT_RECALL_LATENCY_BALANCED_MS, DEFAULT_RECALL_LATENCY_DEEP_MS,
    DEFAULT_RECALL_LATENCY_FAST_MS, query_shape_profile,
};
use crate::protocol::nonempty_opt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallPolicyMode {
    Headlines,
    Fast,
    Balanced,
    Deep,
}

impl RecallPolicyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Headlines => "headlines",
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Deep => "deep",
        }
    }
}

pub fn parse_env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

pub fn recall_default_budget_for_mode(mode: RecallPolicyMode) -> usize {
    match mode {
        RecallPolicyMode::Headlines => 0,
        RecallPolicyMode::Fast => parse_env_usize(
            "CORTEX_RECALL_FAST_BUDGET",
            DEFAULT_RECALL_BUDGET_FAST,
            1,
            2000,
        ),
        RecallPolicyMode::Balanced => parse_env_usize(
            "CORTEX_RECALL_BALANCED_BUDGET",
            DEFAULT_RECALL_BUDGET_BALANCED,
            1,
            4000,
        ),
        RecallPolicyMode::Deep => parse_env_usize(
            "CORTEX_RECALL_DEEP_BUDGET",
            DEFAULT_RECALL_BUDGET_DEEP,
            1,
            8000,
        ),
    }
}

pub fn recall_default_k_for_mode(mode: RecallPolicyMode) -> usize {
    match mode {
        RecallPolicyMode::Headlines => 10,
        RecallPolicyMode::Fast => 16,
        RecallPolicyMode::Balanced => 12,
        RecallPolicyMode::Deep => 10,
    }
}

pub fn recall_latency_budget_ms_for_mode(mode: RecallPolicyMode) -> u128 {
    match mode {
        RecallPolicyMode::Headlines => parse_env_usize(
            "CORTEX_RECALL_HEADLINES_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_FAST_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Fast => parse_env_usize(
            "CORTEX_RECALL_FAST_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_FAST_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Balanced => parse_env_usize(
            "CORTEX_RECALL_BALANCED_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_BALANCED_MS as usize,
            0,
            60_000,
        ) as u128,
        RecallPolicyMode::Deep => parse_env_usize(
            "CORTEX_RECALL_DEEP_MAX_LATENCY_MS",
            DEFAULT_RECALL_LATENCY_DEEP_MS as usize,
            0,
            120_000,
        ) as u128,
    }
}

pub fn recall_mode_for_budget(budget: usize) -> RecallPolicyMode {
    match budget {
        0 => RecallPolicyMode::Headlines,
        1..=220 => RecallPolicyMode::Fast,
        221..=500 => RecallPolicyMode::Balanced,
        _ => RecallPolicyMode::Deep,
    }
}

pub fn parse_recall_policy_mode(raw: Option<&str>) -> Result<Option<RecallPolicyMode>, String> {
    let Some(raw) = nonempty_opt(raw) else {
        return Ok(None);
    };
    let mode = match raw.to_ascii_lowercase().as_str() {
        "headlines" => RecallPolicyMode::Headlines,
        "fast" => RecallPolicyMode::Fast,
        "balanced" => RecallPolicyMode::Balanced,
        "deep" => RecallPolicyMode::Deep,
        _ => {
            return Err(
                "Invalid policy mode. Expected one of: headlines, fast, balanced, deep".into(),
            );
        }
    };
    Ok(Some(mode))
}

pub fn resolve_recall_budget_k(
    requested_mode: Option<RecallPolicyMode>,
    budget: Option<usize>,
    k: Option<usize>,
) -> (usize, usize, RecallPolicyMode) {
    let resolved_budget = match (requested_mode, budget) {
        (_, Some(explicit_budget)) => explicit_budget,
        (Some(mode), None) => recall_default_budget_for_mode(mode),
        (None, None) => recall_default_budget_for_mode(RecallPolicyMode::Balanced),
    };
    let resolved_mode = recall_mode_for_budget(resolved_budget);
    let resolved_k = k.unwrap_or_else(|| recall_default_k_for_mode(resolved_mode));
    (resolved_budget, resolved_k.max(1), resolved_mode)
}

pub fn adaptive_default_budget_for_query(
    query_text: &str,
    resolved_k: usize,
    default_budget: usize,
) -> usize {
    if default_budget == 0 {
        return 0;
    }
    let profile = query_shape_profile(query_text, None);
    let token_count = query_text.split_whitespace().count();
    let base: usize = match (profile.exactish, profile.naturalish) {
        (true, false) => 180,
        (false, true) if token_count >= 14 => 300,
        (false, true) => 270,
        _ => 240,
    };
    let scaled = match resolved_k {
        0..=3 => base.saturating_sub(40),
        4..=6 => base,
        7..=10 => base.saturating_add(30),
        _ => base.saturating_add(60),
    };
    scaled.clamp(140, default_budget.max(140))
}

pub fn maybe_apply_adaptive_default_budget(
    query_text: &str,
    requested_mode: Option<RecallPolicyMode>,
    requested_budget: Option<usize>,
    resolved_budget: usize,
    resolved_k: usize,
) -> usize {
    if requested_mode.is_some() || requested_budget.is_some() {
        return resolved_budget;
    }
    adaptive_default_budget_for_query(query_text, resolved_k, resolved_budget)
}
