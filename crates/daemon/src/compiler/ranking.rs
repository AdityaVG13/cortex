use super::*;
use crate::handlers::estimate_tokens;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RankComponents {
    pub(crate) class_score: f64,
    pub(crate) recency_score: f64,
    pub(crate) relevance_score: f64,
    pub(crate) activity_score: f64,
    pub(crate) total_score: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RankAudit {
    pub(crate) source_kind: &'static str,
    pub(crate) source_id: i64,
    pub(crate) retention_class: String,
    pub(crate) trust_sigil: &'static str,
    pub(crate) valid_from: Option<String>,
    pub(crate) valid_until: Option<String>,
    pub(crate) components: RankComponents,
}
#[derive(Clone, Debug)]
pub(crate) struct RankedCandidate {
    pub(crate) source_kind: &'static str,
    pub(crate) source_id: i64,
    pub(crate) retention_class: String,
    pub(crate) body: String,
    pub(crate) updated_at: Option<String>,
    pub(crate) created_at: Option<String>,
    pub(crate) last_accessed: Option<String>,
    pub(crate) retrievals: i64,
    pub(crate) relevance: f64,
    pub(crate) status: String,
    pub(crate) confirmed_by: Option<String>,
    pub(crate) valid_from: Option<String>,
    pub(crate) valid_until: Option<String>,
    pub(crate) components: RankComponents,
}
pub(crate) fn clamp01(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
pub(crate) fn retention_class_score(retention_class: &str) -> f64 {
    match retention_class {
        "durable" => 1.0,
        "operational" => 0.8,
        "audit" => 0.4,
        "ephemeral" => 0.2,
        _ => 0.6,
    }
}
pub(crate) fn parse_timestamp(value: Option<&str>) -> Option<DateTime<Utc>> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok().map(|dt| Utc.from_utc_datetime(&dt)))
}
pub(crate) fn recency_score(timestamp: Option<&str>, now: DateTime<Utc>) -> f64 {
    let Some(timestamp) = parse_timestamp(timestamp) else {
        return 0.05;
    };
    let age_hours = now.signed_duration_since(timestamp).num_hours().max(0);
    match age_hours {
        0..=1 => 1.0,
        2..=24 => 0.85,
        25..=168 => 0.60,
        169..=720 => 0.35,
        721..=2160 => 0.15,
        _ => 0.05,
    }
}
pub(crate) fn activity_score(retrievals: i64, last_accessed: Option<&str>, now: DateTime<Utc>) -> f64 {
    let retrieval_score = (retrievals.max(0) as f64 / 10.0).min(1.0);
    let access_score = recency_score(last_accessed, now);
    (retrieval_score * 0.55) + (access_score * 0.45)
}
pub(crate) fn rank_components_for(candidate: &RankedCandidate, now: DateTime<Utc>) -> RankComponents {
    let timestamp = candidate.updated_at.as_deref().or(candidate.created_at.as_deref());
    let class_score = retention_class_score(&candidate.retention_class);
    let recency_score = recency_score(timestamp, now);
    let relevance_score = clamp01(candidate.relevance);
    let activity_score = activity_score(candidate.retrievals, candidate.last_accessed.as_deref(), now);
    let total_score = (class_score * RANK_WEIGHT_CLASS)
        + (recency_score * RANK_WEIGHT_RECENCY)
        + (relevance_score * RANK_WEIGHT_RELEVANCE)
        + (activity_score * RANK_WEIGHT_ACTIVITY);
    RankComponents { class_score, recency_score, relevance_score, activity_score, total_score }
}
pub(crate) fn rank_candidates(mut candidates: Vec<RankedCandidate>, top_n: usize, now: DateTime<Utc>) -> Vec<RankedCandidate> {
    for candidate in &mut candidates {
        candidate.components = rank_components_for(candidate, now);
    }
    candidates.sort_by(|left, right| {
        right
            .components
            .total_score
            .partial_cmp(&left.components.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.source_kind.cmp(right.source_kind))
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    candidates.truncate(top_n);
    candidates
}
pub(crate) fn rank_audit_json(audit: &RankAudit) -> Value {
    json!({
        "sourceKind": audit.source_kind,
        "sourceId": audit.source_id,
        "retentionClass": audit.retention_class,
        "trustSigil": audit.trust_sigil,
        "validFrom": audit.valid_from,
        "validUntil": audit.valid_until,
        "rankComponents": {
            "class": (audit.components.class_score * 10000.0).round() / 10000.0,
            "recency": (audit.components.recency_score * 10000.0).round() / 10000.0,
            "relevance": (audit.components.relevance_score * 10000.0).round() / 10000.0,
            "activity": (audit.components.activity_score * 10000.0).round() / 10000.0,
            "total": (audit.components.total_score * 10000.0).round() / 10000.0
        }
    })
}
pub(crate) fn fact_trust_sigil(status: &str, confirmed_by: Option<&str>) -> &'static str {
    if status == "disputed" {
        "FACT~"
    } else if confirmed_by.is_some_and(|value| !value.trim().is_empty()) {
        "FACT!"
    } else {
        "FACT?"
    }
}
fn fact_window_boundary(value: Option<&str>, fallback: &str) -> String {
    parse_timestamp(value)
        .map(|timestamp| timestamp.format("%Y-%m-%d").to_string())
        .or_else(|| value.map(|raw| raw.chars().take(10).collect()))
        .filter(|value: &String| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}
pub(crate) fn format_fact_line(
    source_kind: &str, source_id: i64, body: &str, status: &str, confirmed_by: Option<&str>, valid_from: Option<&str>, valid_until: Option<&str>,
) -> String {
    let sigil = fact_trust_sigil(status, confirmed_by);
    let id_prefix = if source_kind == "decision" { 'd' } else { 'm' };
    let body = body.chars().take(420).collect::<String>();
    let valid_from = fact_window_boundary(valid_from, "unknown");
    let valid_until = fact_window_boundary(valid_until, "now");
    format!("{sigil} {body}  (valid {valid_from} → {valid_until})  [{id_prefix}{source_id}]")
}
pub(crate) struct ContextItem {
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) tokens: usize,
    pub(crate) priority: f64,
    pub(crate) utility: f64,
    pub(crate) rank_audit: Option<RankAudit>,
}
impl ContextItem {
    pub(crate) fn new(name: &str, text: String, priority: f64) -> Self {
        let tokens = estimate_tokens(&text);
        let utility = if tokens > 0 { priority / (tokens as f64) } else { 0.0 };
        Self { name: name.to_string(), text, tokens, priority, utility, rank_audit: None }
    }
    pub(crate) fn from_ranked_candidate(candidate: RankedCandidate) -> Self {
        let trust_sigil = fact_trust_sigil(&candidate.status, candidate.confirmed_by.as_deref());
        let text = format_fact_line(
            candidate.source_kind,
            candidate.source_id,
            &candidate.body,
            &candidate.status,
            candidate.confirmed_by.as_deref(),
            candidate.valid_from.as_deref(),
            candidate.valid_until.as_deref(),
        );
        let mut item = Self::new(&format!("ranked:{}:{}", candidate.source_kind, candidate.source_id), text, candidate.components.total_score.max(0.01));
        item.rank_audit = Some(RankAudit {
            source_kind: candidate.source_kind,
            source_id: candidate.source_id,
            retention_class: candidate.retention_class,
            trust_sigil,
            valid_from: candidate.valid_from,
            valid_until: candidate.valid_until,
            components: candidate.components,
        });
        item
    }
}
pub(crate) fn attach_rank_audit(mut entry: Value, item: &ContextItem) -> Value {
    if let (Some(object), Some(audit)) = (entry.as_object_mut(), item.rank_audit.as_ref()) {
        if let Value::Object(rank_object) = rank_audit_json(audit) {
            for (key, value) in rank_object {
                object.insert(key, value);
            }
        }
    }
    entry
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceTokenBounds {
    pub(crate) min: usize,
    pub(crate) max: usize,
}
impl SourceTokenBounds {
    pub(crate) fn new(min: usize, max: usize) -> Self {
        let min = min.max(1);
        Self { min, max: max.max(min) }
    }
}
pub(crate) struct PackedContext {
    pub(crate) assembled_parts: Vec<String>,
    pub(crate) admitted: Vec<Value>,
    pub(crate) rejected: Vec<Value>,
}
