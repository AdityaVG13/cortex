// SPDX-License-Identifier: MIT
use serde_json::Value;
#[derive(Clone, Debug)]
pub struct RecallHistoryEntry {
    pub query: String,
    pub timestamp: i64,
}
#[derive(Clone, Debug)]
pub struct PreCacheEntry {
    pub query: String,
    pub results: Value,
    pub expires_at: i64,
}
#[derive(Clone, Debug)]
pub struct DaemonEvent {
    pub event_type: String,
    #[allow(dead_code)]
    pub data: Value,
}
#[derive(Clone, Debug)]
pub enum BrainKind {
    ConsolidationStarted,
    MemberAdded,
    ClusterFinalized,
    Recall,
}
impl BrainKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BrainKind::ConsolidationStarted => "consolidation_started",
            BrainKind::MemberAdded => "member_added",
            BrainKind::ClusterFinalized => "cluster_finalized",
            BrainKind::Recall => "recall",
        }
    }
}
#[derive(Clone, Debug)]
pub struct BrainFiringEvent {
    pub kind: BrainKind,
    pub payload: Value,
    pub owner_id: Option<i64>,
}
#[derive(Clone, Debug)]
pub enum SqliteVecRouteMode {
    Baseline,
    Trial,
    Primary,
}
impl SqliteVecRouteMode {
    pub(crate) fn from_env() -> Self {
        match std::env::var("CORTEX_SQLITE_VEC_ROUTE") {
            Ok(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "baseline" | "off" | "disabled" => Self::Baseline,
                "trial" | "canary" | "sampled" => Self::Trial,
                "primary" | "vec0" | "production" | "on" => Self::Primary,
                unknown => {
                    eprintln!("[cortex] WARNING: invalid CORTEX_SQLITE_VEC_ROUTE={unknown:?}; using primary");
                    Self::Primary
                }
            },
            Err(_) => Self::Primary,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Trial => "trial",
            Self::Primary => "primary",
        }
    }
}
#[derive(Clone, Debug)]
pub struct SqliteVecCanaryConfig {
    pub trial_percent: u8,
    pub force_off: bool,
    pub route_mode: SqliteVecRouteMode,
}
impl SqliteVecCanaryConfig {
    pub(crate) fn from_env() -> Self {
        let route_mode = SqliteVecRouteMode::from_env();
        let trial_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT")
            .ok()
            .and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    return None;
                }
                match trimmed.parse::<u8>() {
                    Ok(percent) => Some(percent.min(100)),
                    Err(_) => {
                        eprintln!("[cortex] WARNING: invalid CORTEX_SQLITE_VEC_TRIAL_PERCENT={trimmed:?}; using 0");
                        Some(0)
                    }
                }
            })
            .unwrap_or(0);
        let force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF")
            .ok()
            .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"));
        Self { trial_percent, force_off, route_mode }
    }
    pub fn effective_route_mode(&self) -> SqliteVecRouteMode {
        if self.force_off {
            SqliteVecRouteMode::Baseline
        } else {
            self.route_mode.clone()
        }
    }
}
