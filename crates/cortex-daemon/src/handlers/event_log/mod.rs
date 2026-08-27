use super::truncate_chars;
use rusqlite;
use serde_json::{json, Value};
const MAX_EVENT_JSON_BYTES: usize = 1_200;
const MAX_EVENT_VALUE_CHARS: usize = 240;
const MERGE_EVENT_PREVIEW_CHARS: usize = 240;
const MAX_SOURCE_LABEL_LEN: usize = 160;
const HIGH_VOLUME_EVENT_PRUNE_INTERVAL: i64 = 64;
const HIGH_VOLUME_EVENT_CAPS: &[(&str, i64)] = &[
    ("agent_boot", 4_000),
    ("boot_savings", 6_000),
    ("store_savings", 10_000),
    ("tool_call_savings", 10_000),
    ("decision_stored", 18_000),
    ("decision_supersede", 10_000),
    ("decision_refine_pending", 10_000),
    ("decision_agreement_merge", 8_000),
    ("decision_truncated", 8_000),
    ("recall_query", 14_000),
    ("merge", 6_000),
    ("decision_conflict", 6_000),
    ("decision_rejected_duplicate", 6_000),
    ("decision_resolve", 6_000),
    ("forget", 3_000),
    ("diary_write", 3_000),
];
const NON_PERSISTENT_BENCHMARK_EVENT_KINDS: &[&str] = &[
    "agent_boot",
    "boot_savings",
    "recall_query",
    "store_savings",
    "tool_call_savings",
    "decision_stored",
    "decision_conflict",
    "decision_rejected_duplicate",
    "decision_supersede",
    "decision_refine_pending",
    "decision_agreement_merge",
    "decision_truncated",
    "decision_resolve",
    "merge",
];
fn compact_event_payload(kind: &str, data: Value) -> Value {
    let projected = match kind {
        "recall_query" => compact_recall_query_payload(data),
        "merge" => compact_merge_event_payload(data),
        "store_savings" | "tool_call_savings" | "boot_savings" => compact_savings_event_payload(data),
        _ => truncate_event_value(data, 0),
    };
    enforce_event_payload_budget(kind, projected)
}
fn compact_recall_query_payload(data: Value) -> Value {
    let Some(obj) = data.as_object() else {
        return truncate_event_value(data, 0);
    };
    let semantic_route = compact_semantic_route(obj.get("semantic_route"));
    let shadow_semantic = compact_shadow_semantic(obj.get("shadow_semantic"));
    json!({"agent":obj.get("agent").cloned(
).unwrap_or(Value::Null),"query":obj.get("query").and_then(Value::as_str).map(|q|truncate_chars(q,120)).unwrap_or_default(),
"budget":extract_i64(obj.get("budget")),"spent":extract_i64(obj.get("spent")),"saved":extract_i64(obj.get("saved")),"hits":
extract_i64(obj.get("hits")),"mode":obj.get("mode").cloned().unwrap_or(Value::Null),"cached":obj.get("cached").cloned().unwrap_or(
Value::Null),"tier":obj.get("tier").cloned().unwrap_or(Value::Null),"latency_ms":extract_i64(obj.get("latency_ms")),
"method_breakdown":truncate_event_value(obj.get("method_breakdown").cloned().unwrap_or(Value::Null),0),"semantic_route":
semantic_route,"shadow_semantic":shadow_semantic,})
}
fn compact_semantic_route(value: Option<&Value>) -> Value {
    let Some(route) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    json!({"mode":route.get("mode").cloned().unwrap_or(Value::Null),"reason":route
    .get("reason").cloned().unwrap_or(Value::Null),"sampled":route.get("sampled").cloned().unwrap_or(Value::Null),"trialPercent":route
    .get("trialPercent").cloned().unwrap_or(Value::Null),"candidateCount":route.get("candidateCount").cloned().unwrap_or(Value::Null),
    })
}
fn compact_shadow_semantic(value: Option<&Value>) -> Value {
    let Some(shadow) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    json!({"status":shadow.get("status").cloned().unwrap_or(Value::Null),"reason":shadow.get("reason").cloned().unwrap_or(Value
::Null),"baselineCount":shadow.get("baselineCount").cloned().unwrap_or(Value::Null),"shadowCount":shadow.get("shadowCount").cloned
().unwrap_or(Value::Null),"overlapCount":shadow.get("overlapCount").cloned().unwrap_or(Value::Null),"baselineTopSimilarity":shadow
.get("baselineTopSimilarity").cloned().unwrap_or(Value::Null),"shadowTopSimilarity":shadow.get("shadowTopSimilarity").cloned().
unwrap_or(Value::Null),"baselineTopSources":Value::Null,"shadowTopSources":Value::Null,})
}
fn compact_merge_event_payload(data: Value) -> Value {
    let Some(obj) = data.as_object() else {
        return truncate_event_value(data, 0);
    };
    let incoming = obj.get("incoming_text").and_then(Value::as_str).unwrap_or_default();
    let incoming_chars = incoming.chars().count() as i64;
    json!({"source_id":obj.get(
"source_id").cloned().unwrap_or(Value::Null),"target_id":obj.get("target_id").cloned().unwrap_or(Value::Null),"target_type":obj.
get("target_type").cloned().unwrap_or(Value::Null),"similarity":obj.get("similarity").cloned().unwrap_or(Value::Null),"jaccard":
obj.get("jaccard").cloned().unwrap_or(Value::Null),"source_agent":obj.get("source_agent").cloned().unwrap_or(Value::Null),
"incoming_chars":incoming_chars,"incoming_preview":truncate_chars(incoming,MERGE_EVENT_PREVIEW_CHARS),})
}
fn compact_savings_event_payload(data: Value) -> Value {
    let Some(obj) = data.as_object() else {
        return truncate_event_value(data, 0);
    };
    json!({
"agent":obj.get("agent").cloned().unwrap_or(Value::Null),"query":obj.get("query").and_then(Value::as_str).map(|q|truncate_chars(q,
120)).unwrap_or_default(),"saved":extract_i64(obj.get("saved")),"served":extract_i64(obj.get("served")),"baseline":extract_i64(obj
.get("baseline")),"spent":extract_i64(obj.get("spent")),"budget":extract_i64(obj.get("budget")),"hits":extract_i64(obj.get("hits")
),"boots":extract_i64(obj.get("boots")),"percent":extract_i64(obj.get("percent")),"admitted":extract_i64(obj.get("admitted")),
"rejected":extract_i64(obj.get("rejected")),"mode":obj.get("mode").cloned().unwrap_or(Value::Null),"cached":obj.get("cached").
cloned().unwrap_or(Value::Null),"tier":obj.get("tier").cloned().unwrap_or(Value::Null),"latency_ms":extract_i64(obj.get(
"latency_ms")),})
}
fn extract_i64(value: Option<&Value>) -> i64 {
    value
        .and_then(|v| v.as_i64().or_else(|| v.as_u64().and_then(|x| i64::try_from(x).ok())).or_else(|| v.as_f64().map(|x| x.round() as i64)))
        .unwrap_or(0)
}
fn truncate_event_value(value: Value, depth: usize) -> Value {
    if depth >= 4 {
        return Value::Null;
    }
    match value {
        Value::String(s) => Value::String(truncate_chars(&s, MAX_EVENT_VALUE_CHARS)),
        Value::Array(items) => Value::Array(items.into_iter().take(16).map(|item| truncate_event_value(item, depth + 1)).collect()),
        Value::Object(map) => {
            let compacted = map.into_iter().take(24).map(|(key, val)| (key, truncate_event_value(val, depth + 1))).collect();
            Value::Object(compacted)
        }
        other => other,
    }
}
fn enforce_event_payload_budget(kind: &str, payload: Value) -> Value {
    let encoded = payload.to_string();
    if encoded.len() <= MAX_EVENT_JSON_BYTES {
        return payload;
    }
    let mut fallback = json!({"truncated":true,"type":kind,"bytes":encoded.len()});
    if let Some(obj) = payload.as_object() {
        for key in [
            "agent",
            "source_agent",
            "saved",
            "served",
            "baseline",
            "spent",
            "budget",
            "hits",
            "misses",
            "events",
            "boots",
            "percent",
            "admitted",
            "rejected",
            "mode",
            "cached",
            "tier",
            "latency_ms",
            "source_id",
            "target_id",
            "target_type",
            "similarity",
            "jaccard",
            "incoming_chars",
        ] {
            if let Some(value) = obj.get(key).and_then(|value| compact_budget_scalar(value, MAX_EVENT_VALUE_CHARS)) {
                fallback[key] = value;
            }
        }
        if let Some(query) = obj.get("query").and_then(Value::as_str) {
            fallback["query"] = Value::String(truncate_chars(query, 120));
        }
        let semantic_route = compact_semantic_route(obj.get("semantic_route"));
        if !semantic_route.is_null() {
            fallback["semantic_route"] = semantic_route;
        }
        let shadow_semantic = compact_shadow_semantic(obj.get("shadow_semantic"));
        if !shadow_semantic.is_null() {
            fallback["shadow_semantic"] = shadow_semantic;
        }
    }
    if fallback.to_string().len() <= MAX_EVENT_JSON_BYTES {
        return fallback;
    }
    if let Some(fallback_obj) = fallback.as_object_mut() {
        for key in ["query", "semantic_route", "shadow_semantic", "target_type", "tier", "mode"] {
            fallback_obj.remove(key);
        }
    }
    if fallback.to_string().len() <= MAX_EVENT_JSON_BYTES {
        return fallback;
    }
    let mut minimal = json!({"truncated":true,"type":kind,"bytes":encoded.len()});
    if let Some(obj) = payload.as_object() {
        for key in ["agent", "source_agent"] {
            if let Some(value) = obj.get(key).and_then(|value| compact_budget_scalar(value, MAX_SOURCE_LABEL_LEN)) {
                minimal[key] = value;
            }
        }
    }
    minimal
}
fn compact_budget_scalar(value: &Value, max_chars: usize) -> Option<Value> {
    match value {
        Value::String(text) => Some(Value::String(truncate_chars(text, max_chars))),
        Value::Number(_) | Value::Bool(_) | Value::Null => Some(value.clone()),
        _ => None,
    }
}
fn payload_field_has_benchmark_prefix(payload: &Value, key: &str, lowercase_prefix: &str) -> bool {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().starts_with(lowercase_prefix))
        .unwrap_or(false)
}
fn is_benchmark_event_source(source_agent: &str, payload: &Value) -> bool {
    let benchmark_prefix = crate::compaction::BENCHMARK_SOURCE_AGENT_PREFIX.to_ascii_lowercase();
    source_agent.trim().to_ascii_lowercase().starts_with(&benchmark_prefix)
        || payload_field_has_benchmark_prefix(payload, "source_agent", &benchmark_prefix)
        || payload_field_has_benchmark_prefix(payload, "agent", &benchmark_prefix)
}
fn should_skip_benchmark_event_persistence(kind: &str, payload: &Value, source_agent: &str) -> bool {
    NON_PERSISTENT_BENCHMARK_EVENT_KINDS.contains(&kind) && is_benchmark_event_source(source_agent, payload)
}
pub fn log_event(conn: &rusqlite::Connection, kind: &str, data: Value, source_agent: &str) -> rusqlite::Result<()> {
    let compacted = compact_event_payload(kind, data);
    if should_skip_benchmark_event_persistence(kind, &compacted, source_agent) {
        return Ok(());
    }
    conn.execute("INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)", rusqlite::params![kind, compacted.to_string(), source_agent])?;
    maybe_prune_high_volume_event(conn, kind)?;
    Ok(())
}
fn maybe_prune_high_volume_event(conn: &rusqlite::Connection, kind: &str) -> rusqlite::Result<()> {
    let Some(keep_rows) = HIGH_VOLUME_EVENT_CAPS.iter().find_map(|(event_type, keep)| (*event_type == kind).then_some(*keep)) else {
        return Ok(());
    };
    let inserted_id = conn.last_insert_rowid();
    if inserted_id <= 0 || inserted_id % HIGH_VOLUME_EVENT_PRUNE_INTERVAL != 0 {
        return Ok(());
    }
    prune_event_type_keep_latest(conn, kind, keep_rows)
}
fn prune_event_type_keep_latest(conn: &rusqlite::Connection, event_type: &str, keep_rows: i64) -> rusqlite::Result<()> {
    if keep_rows < 1 {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM events
         WHERE id IN (
           SELECT id
           FROM events
           WHERE type = ?1
           ORDER BY id DESC
           LIMIT -1 OFFSET ?2
         )",
        rusqlite::params![event_type, keep_rows],
    )?;
    Ok(())
}

