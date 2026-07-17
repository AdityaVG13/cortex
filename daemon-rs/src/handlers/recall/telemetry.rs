// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;
use std::time::Instant;

use crate::handlers::{ensure_auth_with_caller_rated_for_class, ensure_endpoint_budget};
use crate::handlers::{
    estimate_tokens, json_response, now_iso, parse_timestamp_ms, resolve_source_identity,
    truncate_chars,
};

use super::*;
use crate::budgets::BudgetEndpoint;
use crate::co_occurrence;
use crate::db::checkpoint_wal_best_effort;
use crate::rate_limit::RequestClass;
use crate::rerank::{RerankCandidate, RerankedScore};
use crate::state::{
    PreCacheEntry, RecallHistoryEntry, RuntimeState, SqliteVecCanaryConfig, SqliteVecRouteMode,
};

// ─── Unified recall pipeline ─────────────────────────────────────────────────

pub(crate) fn is_benchmark_recall_scope(agent: &str, source_prefix: Option<&str>) -> bool {
    if agent
        .trim()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_AGENT_PREFIX)
    {
        return true;
    }
    source_prefix
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .starts_with(BENCHMARK_SOURCE_SCOPE_PREFIX)
}

pub(crate) async fn emit_recall_query_event(
    state: &RuntimeState,
    agent: &str,
    source_prefix: Option<&str>,
    payload: Value,
) {
    if is_benchmark_recall_scope(agent, source_prefix) {
        return;
    }
    let conn = state.db.lock().await;
    if crate::handlers::log_event(&conn, "recall_query", payload, agent).is_ok() {
        checkpoint_wal_best_effort(&conn);
    }
}

pub(crate) fn build_method_breakdown(results: &[RecallItem]) -> Value {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for item in results {
        *counts.entry(item.method.clone()).or_insert(0) += 1;
    }
    json!(counts)
}

pub(crate) fn method_count(methods: &Value, method: &str) -> i64 {
    methods.get(method).and_then(|v| v.as_i64()).unwrap_or(0)
}

pub(crate) fn classify_recall_tier(cached: bool, mode: &str, methods: &Value) -> &'static str {
    if cached {
        return "cache_hit";
    }
    if mode == "headlines" {
        return "headlines";
    }
    if mode == "semantic" {
        return "semantic_only";
    }

    let keyword = method_count(methods, "keyword");
    let semantic = method_count(methods, "semantic");
    let hybrid = method_count(methods, "hybrid");
    let crystal = method_count(methods, "crystal");
    let associative = method_count(methods, "associative");

    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        if crystal > 0 {
            return "hybrid_crystal";
        }
        return "hybrid_fusion";
    }
    if associative > 0 && (keyword > 0 || semantic > 0 || crystal > 0) {
        return "associative_blend";
    }
    if keyword > 0 {
        if crystal > 0 {
            return "keyword_crystal";
        }
        return "keyword_only";
    }
    if semantic > 0 {
        if crystal > 0 {
            return "semantic_crystal";
        }
        return "semantic_only";
    }
    if crystal > 0 {
        return "crystal_only";
    }
    if associative > 0 {
        return "associative_only";
    }
    "unknown"
}

pub(crate) fn shadow_semantic_telemetry_summary(shadow_semantic: &Value) -> Value {
    let status = shadow_semantic
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");

    let mut summary = json!({
        "status": status,
    });
    if let Some(reason) = shadow_semantic.get("reason").and_then(Value::as_str) {
        summary["reason"] = json!(reason);
    }
    for key in [
        "topK",
        "vectorDimension",
        "baselineCandidateCount",
        "shadowCandidateCount",
        "overlapCount",
        "overlapRatio",
        "jaccard",
        "matchedRankPairs",
        "meanAbsRankDelta",
        "top1Match",
    ] {
        if let Some(value) = shadow_semantic.get(key) {
            summary[key] = value.clone();
        }
    }
    if status == "error" && summary.get("reason").is_none() {
        summary["reason"] = json!("shadow_payload_invalid");
    }
    summary
}

