mod engine;

pub use engine::*;
/// Route-family quotas as JSON (operator view).
pub fn route_quotas() -> serde_json::Value {
    let map: std::collections::BTreeMap<String, usize> = engine::ROUTE_QUOTAS
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
    serde_json::json!(map)
}
pub(crate) use engine::{
    explicit_paths_by_target, jaccard_path_sets, normalize_query_paths, read_path_sets,
};
