mod engine;

pub use engine::bump_retrievals_sources;
pub use engine::clock_health_payload;
pub use engine::*;
pub use engine::{
    execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall, unfold_source,
};
/// Route-family quotas as JSON (operator view).
pub fn route_quotas() -> serde_json::Value {
    let map: std::collections::BTreeMap<String, usize> = engine::ROUTE_QUOTAS
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    serde_json::json!(map)
}
pub use engine::run_budget_recall_trace_with_query_vector;
pub use engine::{
    parse_recall_policy_mode, resolve_recall_budget_k, RecallContext, RecallPolicyMode,
};
pub(crate) use engine::target_scope_compatible;
