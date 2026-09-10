use serde_json::Value;
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
    pub fn disabled() -> Self {
        Self {
            trial_percent: 0,
            force_off: true,
            route_mode: SqliteVecRouteMode::Baseline,
        }
    }
    pub fn effective_route_mode(&self) -> SqliteVecRouteMode {
        if self.force_off {
            SqliteVecRouteMode::Baseline
        } else {
            self.route_mode.clone()
        }
    }
}
