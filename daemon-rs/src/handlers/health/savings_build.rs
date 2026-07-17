// SPDX-License-Identifier: MIT
use serde_json::{json, Value};
use std::collections::BTreeMap;
pub(crate) fn value_i64_any(payload: &Value, keys: &[&str]) -> i64 {
    keys.iter().find_map(|key| payload.get(*key).and_then(|v| v.as_i64())).unwrap_or(0)
}
pub(crate) fn method_count(payload: &Value, method: &str) -> i64 {
    payload.get("method_breakdown").and_then(|value| value.get(method)).and_then(|value| value.as_i64()).unwrap_or(0)
}
pub(crate) fn classify_recall_tier_from_payload(payload: &Value) -> String {
    if let Some(tier) = payload.get("tier").and_then(|value| value.as_str()) {
        if !tier.trim().is_empty() {
            return tier.to_string();
        }
    }
    if payload.get("cached").and_then(|value| value.as_bool()).unwrap_or(false) {
        return "cache_hit".to_string();
    }
    let mode = payload.get("mode").and_then(|value| value.as_str()).unwrap_or_default();
    if mode == "headlines" {
        return "headlines".to_string();
    }
    if mode == "semantic" {
        return "semantic_only".to_string();
    }
    let keyword = method_count(payload, "keyword");
    let semantic = method_count(payload, "semantic");
    let hybrid = method_count(payload, "hybrid");
    let crystal = method_count(payload, "crystal");
    if hybrid > 0 || (keyword > 0 && semantic > 0) {
        if crystal > 0 {
            return "hybrid_crystal".to_string();
        }
        return "hybrid_fusion".to_string();
    }
    if keyword > 0 {
        if crystal > 0 {
            return "keyword_crystal".to_string();
        }
        return "keyword_only".to_string();
    }
    if semantic > 0 {
        if crystal > 0 {
            return "semantic_crystal".to_string();
        }
        return "semantic_only".to_string();
    }
    if crystal > 0 {
        return "crystal_only".to_string();
    }
    "unknown".to_string()
}
pub(crate) fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}
pub(crate) fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}
pub(crate) fn normalize_shadow_status(status: &str) -> &'static str {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "ok" => "ok",
        "unavailable" => "unavailable",
        "error" => "error",
        "skipped" => "skipped",
        _ => "unknown",
    }
}
pub(crate) const SHADOW_GATE_MIN_PROBED_EVENTS: i64 = 25;
pub(crate) const SHADOW_GATE_MIN_OK_SAMPLES: i64 = 15;
pub(crate) const SHADOW_GATE_MAX_UNAVAILABLE_RATE: f64 = 0.35;
pub(crate) const SHADOW_GATE_MAX_ERROR_RATE: f64 = 0.05;
pub(crate) const SHADOW_GATE_MIN_OK_OVERLAP_RATIO: f64 = 0.60;
pub(crate) const SHADOW_GATE_MIN_OK_JACCARD: f64 = 0.45;
pub(crate) const SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA: f64 = 1.25;
pub(crate) const SHADOW_GATE_MIN_TOP1_MATCH_RATE: f64 = 0.60;
pub(crate) struct ShadowOkMetricSamples {
    overlap_ratio: i64,
    jaccard: i64,
    mean_abs_rank_delta: i64,
    top1_match: i64,
}
pub(crate) struct ShadowOkMetricAverages {
    overlap_ratio: Option<f64>,
    jaccard: Option<f64>,
    mean_abs_rank_delta: Option<f64>,
    top1_match_rate: Option<f64>,
}
pub(crate) fn build_shadow_semantic_gate(
    shadow_status_counts: &BTreeMap<String, i64>,
    ok_samples: i64,
    ok_metric_samples: &ShadowOkMetricSamples,
    ok_metric_averages: &ShadowOkMetricAverages,
) -> Value {
    let ok_count = *shadow_status_counts.get("ok").unwrap_or(&0);
    let unavailable_count = *shadow_status_counts.get("unavailable").unwrap_or(&0);
    let error_count = *shadow_status_counts.get("error").unwrap_or(&0);
    let unknown_count = *shadow_status_counts.get("unknown").unwrap_or(&0);
    let skipped_count = *shadow_status_counts.get("skipped").unwrap_or(&0);
    let probed_events = ok_count + unavailable_count + error_count + unknown_count;
    let unavailable_rate = if probed_events > 0 { round4(unavailable_count as f64 / probed_events as f64) } else { 0.0 };
    let error_rate = if probed_events > 0 { round4(error_count as f64 / probed_events as f64) } else { 0.0 };
    let mut blockers: Vec<String> = Vec::new();
    if probed_events < SHADOW_GATE_MIN_PROBED_EVENTS {
        blockers.push("insufficient_shadow_samples".to_string());
    }
    if ok_samples < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_ok_samples".to_string());
    }
    if unavailable_rate > SHADOW_GATE_MAX_UNAVAILABLE_RATE {
        blockers.push("unavailable_rate_above_gate".to_string());
    }
    if error_rate > SHADOW_GATE_MAX_ERROR_RATE {
        blockers.push("error_rate_above_gate".to_string());
    }
    if ok_metric_samples.overlap_ratio > 0 && ok_metric_samples.overlap_ratio < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_overlap_ratio_samples".to_string());
    }
    match ok_metric_averages.overlap_ratio {
        Some(value) if value < SHADOW_GATE_MIN_OK_OVERLAP_RATIO => {
            blockers.push("overlap_ratio_below_gate".to_string());
        }
        None => blockers.push("missing_overlap_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.jaccard > 0 && ok_metric_samples.jaccard < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_jaccard_samples".to_string());
    }
    match ok_metric_averages.jaccard {
        Some(value) if value < SHADOW_GATE_MIN_OK_JACCARD => {
            blockers.push("jaccard_below_gate".to_string());
        }
        None => blockers.push("missing_jaccard_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.mean_abs_rank_delta > 0 && ok_metric_samples.mean_abs_rank_delta < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_rank_delta_samples".to_string());
    }
    match ok_metric_averages.mean_abs_rank_delta {
        Some(value) if value > SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA => {
            blockers.push("mean_abs_rank_delta_above_gate".to_string());
        }
        None => blockers.push("missing_rank_delta_signal".to_string()),
        _ => {}
    }
    if ok_metric_samples.top1_match > 0 && ok_metric_samples.top1_match < SHADOW_GATE_MIN_OK_SAMPLES {
        blockers.push("insufficient_top1_match_samples".to_string());
    }
    match ok_metric_averages.top1_match_rate {
        Some(value) if value < SHADOW_GATE_MIN_TOP1_MATCH_RATE => {
            blockers.push("top1_match_rate_below_gate".to_string());
        }
        None => blockers.push("missing_top1_match_signal".to_string()),
        _ => {}
    }
    let ready = blockers.is_empty();
    json!({
        "ready": ready,
        "decision": if ready { "ready_for_vec0_trial" } else { "hold" },
        "target": "sqlite_vec_production_routing",
        "blockers": blockers,
        "metrics": {
            "probed_events": probed_events,
            "ok_count": ok_count,
            "unavailable_count": unavailable_count,
            "error_count": error_count,
            "unknown_count": unknown_count,
            "skipped_count": skipped_count,
            "ok_samples": ok_samples,
            "ok_overlap_samples": ok_metric_samples.overlap_ratio,
            "ok_jaccard_samples": ok_metric_samples.jaccard,
            "ok_rank_delta_samples": ok_metric_samples.mean_abs_rank_delta,
            "ok_top1_match_samples": ok_metric_samples.top1_match,
            "ok_overlap_ratio_avg": ok_metric_averages.overlap_ratio,
            "ok_jaccard_avg": ok_metric_averages.jaccard,
            "ok_mean_abs_rank_delta_avg": ok_metric_averages.mean_abs_rank_delta,
            "ok_top1_match_rate": ok_metric_averages.top1_match_rate,
            "unavailable_rate": unavailable_rate,
            "error_rate": error_rate
        },
        "thresholds": {
            "min_probed_events": SHADOW_GATE_MIN_PROBED_EVENTS,
            "min_ok_samples": SHADOW_GATE_MIN_OK_SAMPLES,
            "max_unavailable_rate": SHADOW_GATE_MAX_UNAVAILABLE_RATE,
            "max_error_rate": SHADOW_GATE_MAX_ERROR_RATE,
            "min_ok_overlap_ratio": SHADOW_GATE_MIN_OK_OVERLAP_RATIO,
            "min_ok_jaccard": SHADOW_GATE_MIN_OK_JACCARD,
            "max_mean_abs_rank_delta": SHADOW_GATE_MAX_MEAN_ABS_RANK_DELTA,
            "min_top1_match_rate": SHADOW_GATE_MIN_TOP1_MATCH_RATE
        }
    })
}
pub(crate) fn build_recall_stats_payload_from_rows(rows: &[(String, String)]) -> Value {
    let mut tier_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_sum: BTreeMap<String, i64> = BTreeMap::new();
    let mut tier_latency_samples: BTreeMap<String, i64> = BTreeMap::new();
    let mut mode_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut shadow_status_counts: BTreeMap<String, i64> = BTreeMap::new();
    let mut total_budget = 0_i64;
    let mut total_spent = 0_i64;
    let mut total_saved = 0_i64;
    let mut total_hits = 0_i64;
    let mut shadow_ok_overlap_ratio_sum = 0.0_f64;
    let mut shadow_ok_overlap_ratio_samples = 0_i64;
    let mut shadow_ok_jaccard_sum = 0.0_f64;
    let mut shadow_ok_jaccard_samples = 0_i64;
    let mut shadow_ok_rank_delta_sum = 0.0_f64;
    let mut shadow_ok_rank_delta_samples = 0_i64;
    let mut shadow_ok_top1_match_sum = 0.0_f64;
    let mut shadow_ok_top1_match_samples = 0_i64;
    let mut latency_total = 0_i64;
    let mut latency_samples = 0_i64;
    let mut recent: Vec<Value> = Vec::new();
    for (data_str, created_at) in rows {
        let payload: Value = serde_json::from_str(data_str).unwrap_or_else(|_| json!({}));
        let mode = payload.get("mode").and_then(|value| value.as_str()).unwrap_or("unknown").to_string();
        *mode_counts.entry(mode.clone()).or_insert(0) += 1;
        let tier = classify_recall_tier_from_payload(&payload);
        *tier_counts.entry(tier.clone()).or_insert(0) += 1;
        let budget = value_i64_any(&payload, &["budget", "baseline"]);
        let spent = value_i64_any(&payload, &["spent", "served"]);
        let saved = value_i64_any(&payload, &["saved"]);
        let hits = value_i64_any(&payload, &["hits", "results"]);
        total_budget += budget.max(0);
        total_spent += spent.max(0);
        total_saved += saved;
        total_hits += hits.max(0);
        if let Some(latency_ms) = payload.get("latency_ms").and_then(|value| value.as_i64()) {
            if latency_ms >= 0 {
                latency_total += latency_ms;
                latency_samples += 1;
                *tier_latency_sum.entry(tier.clone()).or_insert(0) += latency_ms;
                *tier_latency_samples.entry(tier.clone()).or_insert(0) += 1;
            }
        }
        if let Some(shadow_semantic) = payload.get("shadow_semantic").and_then(|value| value.as_object()) {
            let status = shadow_semantic
                .get("status")
                .and_then(|value| value.as_str())
                .map(normalize_shadow_status)
                .unwrap_or("unknown")
                .to_string();
            *shadow_status_counts.entry(status.clone()).or_insert(0) += 1;
            if status == "ok" {
                if let Some(overlap_ratio) = shadow_semantic.get("overlapRatio").and_then(|value| value.as_f64()) {
                    shadow_ok_overlap_ratio_sum += overlap_ratio;
                    shadow_ok_overlap_ratio_samples += 1;
                }
                if let Some(jaccard) = shadow_semantic.get("jaccard").and_then(|value| value.as_f64()) {
                    shadow_ok_jaccard_sum += jaccard;
                    shadow_ok_jaccard_samples += 1;
                }
                if let Some(mean_abs_rank_delta) = shadow_semantic.get("meanAbsRankDelta").and_then(|value| value.as_f64()) {
                    shadow_ok_rank_delta_sum += mean_abs_rank_delta;
                    shadow_ok_rank_delta_samples += 1;
                }
                if let Some(top1_match) = shadow_semantic.get("top1Match").and_then(|value| value.as_bool()) {
                    shadow_ok_top1_match_sum += if top1_match { 1.0 } else { 0.0 };
                    shadow_ok_top1_match_samples += 1;
                }
            }
        }
        recent.push(json!({
            "timestamp": created_at,
            "mode": mode,
            "tier": tier,
            "budget": budget,
            "spent": spent,
            "saved": saved,
            "hits": hits,
            "cached": payload.get("cached").and_then(|value| value.as_bool()).unwrap_or(false),
            "latencyMs": payload.get("latency_ms").and_then(|value| value.as_i64()),
        }));
    }
    let total_recalls = rows.len() as i64;
    let avg_latency_ms = if latency_samples > 0 { round1(latency_total as f64 / latency_samples as f64) } else { 0.0 };
    let savings_pct_vs_budget = if total_budget > 0 { round1((total_saved as f64 / total_budget as f64) * 100.0) } else { 0.0 };
    let shadow_overlap_ratio_avg = if shadow_ok_overlap_ratio_samples > 0 {
        Some(round4(shadow_ok_overlap_ratio_sum / shadow_ok_overlap_ratio_samples as f64))
    } else {
        None
    };
    let shadow_jaccard_avg = if shadow_ok_jaccard_samples > 0 {
        Some(round4(shadow_ok_jaccard_sum / shadow_ok_jaccard_samples as f64))
    } else {
        None
    };
    let shadow_mean_abs_rank_delta_avg = if shadow_ok_rank_delta_samples > 0 {
        Some(round4(shadow_ok_rank_delta_sum / shadow_ok_rank_delta_samples as f64))
    } else {
        None
    };
    let shadow_top1_match_rate = if shadow_ok_top1_match_samples > 0 {
        Some(round4(shadow_ok_top1_match_sum / shadow_ok_top1_match_samples as f64))
    } else {
        None
    };
    let ok_metric_samples = ShadowOkMetricSamples {
        overlap_ratio: shadow_ok_overlap_ratio_samples,
        jaccard: shadow_ok_jaccard_samples,
        mean_abs_rank_delta: shadow_ok_rank_delta_samples,
        top1_match: shadow_ok_top1_match_samples,
    };
    let ok_metric_averages = ShadowOkMetricAverages {
        overlap_ratio: shadow_overlap_ratio_avg,
        jaccard: shadow_jaccard_avg,
        mean_abs_rank_delta: shadow_mean_abs_rank_delta_avg,
        top1_match_rate: shadow_top1_match_rate,
    };
    let shadow_ok_samples = [
        ok_metric_samples.overlap_ratio,
        ok_metric_samples.jaccard,
        ok_metric_samples.mean_abs_rank_delta,
        ok_metric_samples.top1_match,
    ]
    .into_iter()
    .min()
    .unwrap_or(0);
    let shadow_gate = build_shadow_semantic_gate(&shadow_status_counts, shadow_ok_samples, &ok_metric_samples, &ok_metric_averages);
    let tier_distribution: Vec<Value> = tier_counts
        .iter()
        .map(|(tier, count)| {
            let percent = if total_recalls > 0 { round1((*count as f64 / total_recalls as f64) * 100.0) } else { 0.0 };
            let avg_tier_latency = match (tier_latency_sum.get(tier).copied(), tier_latency_samples.get(tier).copied()) {
                (Some(sum), Some(samples)) if samples > 0 => round1(sum as f64 / samples as f64),
                _ => 0.0,
            };
            json!({
                "tier": tier,
                "count": count,
                "percent": percent,
                "avgLatencyMs": avg_tier_latency
            })
        })
        .collect();
    let tier_distribution_map: Value = json!(tier_counts.iter().map(|(tier, count)| (tier.clone(), json!(count))).collect::<serde_json::Map<String, Value>>());
    let avg_latency_map: Value = {
        let mut map = serde_json::Map::new();
        map.insert("overall".to_string(), json!(avg_latency_ms));
        for entry in &tier_distribution {
            if let (Some(tier), Some(avg)) = (entry.get("tier").and_then(|value| value.as_str()), entry.get("avgLatencyMs")) {
                map.insert(tier.to_string(), avg.clone());
            }
        }
        Value::Object(map)
    };
    recent.sort_by(|a, b| {
        let a_ts = a.get("timestamp").and_then(|value| value.as_str()).unwrap_or("");
        let b_ts = b.get("timestamp").and_then(|value| value.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });
    recent.truncate(30);
    json!({
        "summary": {
            "totalRecalls": total_recalls,
            "totalHits": total_hits,
            "totalBudget": total_budget,
            "totalSpent": total_spent,
            "totalSaved": total_saved,
            "savingsPctVsBudget": savings_pct_vs_budget,
            "avgLatencyMs": avg_latency_ms
        },
        "tierDistribution": tier_distribution,
        "tier_distribution": tier_distribution_map,
        "avg_latency_ms": avg_latency_map,
        "estimated_savings": {
            "vs_always_full_pipeline_pct": savings_pct_vs_budget
        },
        "modeCounts": mode_counts,
        "shadow_semantic": {
            "status_counts": shadow_status_counts,
            "ok_samples": shadow_ok_samples,
            "ok_overlap_samples": ok_metric_samples.overlap_ratio,
            "ok_jaccard_samples": ok_metric_samples.jaccard,
            "ok_rank_delta_samples": ok_metric_samples.mean_abs_rank_delta,
            "ok_top1_match_samples": ok_metric_samples.top1_match,
            "ok_overlap_ratio_avg": ok_metric_averages.overlap_ratio,
            "ok_jaccard_avg": ok_metric_averages.jaccard,
            "ok_mean_abs_rank_delta_avg": ok_metric_averages.mean_abs_rank_delta,
            "ok_top1_match_rate": ok_metric_averages.top1_match_rate
        },
        "shadowSemantic": {
            "statusCounts": shadow_status_counts,
            "okSamples": shadow_ok_samples,
            "okOverlapSamples": ok_metric_samples.overlap_ratio,
            "okJaccardSamples": ok_metric_samples.jaccard,
            "okRankDeltaSamples": ok_metric_samples.mean_abs_rank_delta,
            "okTop1MatchSamples": ok_metric_samples.top1_match,
            "okOverlapRatioAvg": ok_metric_averages.overlap_ratio,
            "okJaccardAvg": ok_metric_averages.jaccard,
            "okMeanAbsRankDeltaAvg": ok_metric_averages.mean_abs_rank_delta,
            "okTop1MatchRate": ok_metric_averages.top1_match_rate
        },
        "shadow_semantic_gate": shadow_gate,
        "shadowSemanticGate": {
            "ready": shadow_gate["ready"],
            "decision": shadow_gate["decision"],
            "target": shadow_gate["target"],
            "blockers": shadow_gate["blockers"],
            "metrics": {
                "probedEvents": shadow_gate["metrics"]["probed_events"],
                "okCount": shadow_gate["metrics"]["ok_count"],
                "unavailableCount": shadow_gate["metrics"]["unavailable_count"],
                "errorCount": shadow_gate["metrics"]["error_count"],
                "unknownCount": shadow_gate["metrics"]["unknown_count"],
                "skippedCount": shadow_gate["metrics"]["skipped_count"],
                "okSamples": shadow_gate["metrics"]["ok_samples"],
                "okOverlapSamples": shadow_gate["metrics"]["ok_overlap_samples"],
                "okJaccardSamples": shadow_gate["metrics"]["ok_jaccard_samples"],
                "okRankDeltaSamples": shadow_gate["metrics"]["ok_rank_delta_samples"],
                "okTop1MatchSamples": shadow_gate["metrics"]["ok_top1_match_samples"],
                "okOverlapRatioAvg": shadow_gate["metrics"]["ok_overlap_ratio_avg"],
                "okJaccardAvg": shadow_gate["metrics"]["ok_jaccard_avg"],
                "okMeanAbsRankDeltaAvg": shadow_gate["metrics"]["ok_mean_abs_rank_delta_avg"],
                "okTop1MatchRate": shadow_gate["metrics"]["ok_top1_match_rate"],
                "unavailableRate": shadow_gate["metrics"]["unavailable_rate"],
                "errorRate": shadow_gate["metrics"]["error_rate"]
            },
            "thresholds": {
                "minProbedEvents": shadow_gate["thresholds"]["min_probed_events"],
                "minOkSamples": shadow_gate["thresholds"]["min_ok_samples"],
                "maxUnavailableRate": shadow_gate["thresholds"]["max_unavailable_rate"],
                "maxErrorRate": shadow_gate["thresholds"]["max_error_rate"],
                "minOkOverlapRatio": shadow_gate["thresholds"]["min_ok_overlap_ratio"],
                "minOkJaccard": shadow_gate["thresholds"]["min_ok_jaccard"],
                "maxMeanAbsRankDelta": shadow_gate["thresholds"]["max_mean_abs_rank_delta"],
                "minTop1MatchRate": shadow_gate["thresholds"]["min_top1_match_rate"]
            }
        },
        "recent": recent
    })
}
