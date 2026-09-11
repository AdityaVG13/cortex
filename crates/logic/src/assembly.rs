//! Reversible assemblies and attributed learning math.
//!
//! Membership is exact. Factoring is typed JSON, not a summary. Ranking is
//! scoped utility from authorized events, never a truth score.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipRole {
    Observation,
    Support,
    Exception,
    Contradiction,
    Qualification,
}

impl MembershipRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Support => "support",
            Self::Exception => "exception",
            Self::Contradiction => "contradiction",
            Self::Qualification => "qualification",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "observation" => Ok(Self::Observation),
            "support" => Ok(Self::Support),
            "exception" => Ok(Self::Exception),
            "contradiction" => Ok(Self::Contradiction),
            "qualification" => Ok(Self::Qualification),
            _ => Err("invalid_membership_role".into()),
        }
    }

    pub fn is_required_exception(self) -> bool {
        matches!(self, Self::Exception | Self::Contradiction)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorCodec {
    Raw,
    TemplateResidual,
}

impl FactorCodec {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::TemplateResidual => "template_residual",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "raw" => Ok(Self::Raw),
            "template_residual" => Ok(Self::TemplateResidual),
            _ => Err("unknown_factor_codec".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningKind {
    Explicit,
    VerifiedUse,
    Exposure,
    Uncertain,
    Structural,
}

impl LearningKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::VerifiedUse => "verified_use",
            Self::Exposure => "exposure",
            Self::Uncertain => "uncertain",
            Self::Structural => "structural",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "explicit" => Ok(Self::Explicit),
            "verified_use" => Ok(Self::VerifiedUse),
            "exposure" => Ok(Self::Exposure),
            "uncertain" => Ok(Self::Uncertain),
            "structural" => Ok(Self::Structural),
            _ => Err("invalid_feedback_kind".into()),
        }
    }

    pub fn counts_as_reward(self) -> bool {
        matches!(self, Self::Explicit | Self::VerifiedUse)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearningEvent {
    pub origin: String,
    pub origin_event_id: String,
    pub principal: String,
    pub scope: String,
    pub training_unit: String,
    pub target: String,
    pub kind: LearningKind,
    pub reward: i8,
    pub cues: Vec<String>,
    pub sources: Vec<String>,
    pub observed_at: i64,
    pub receipt_ref: String,
}

impl LearningEvent {
    pub fn validate(&self) -> Result<(), String> {
        for value in [
            &self.origin,
            &self.origin_event_id,
            &self.principal,
            &self.scope,
            &self.training_unit,
            &self.target,
            &self.receipt_ref,
        ] {
            if value.is_empty() || value.len() > 1024 {
                return Err("invalid_feedback_identity".into());
            }
        }
        if !matches!(self.reward, -1 | 0 | 1) {
            return Err("invalid_feedback_reward".into());
        }
        if self.observed_at < 0 {
            return Err("invalid_feedback_time".into());
        }
        if self.cues.is_empty() || self.cues.len() > 64 {
            return Err("invalid_feedback_cues".into());
        }
        let unique_cues: BTreeSet<_> = self.cues.iter().collect();
        if unique_cues.len() != self.cues.len()
            || self
                .cues
                .iter()
                .any(|cue| cue.is_empty() || cue.len() > 1024)
        {
            return Err("invalid_feedback_cues".into());
        }
        let unique_sources: BTreeSet<_> = self.sources.iter().collect();
        if unique_sources.len() != self.sources.len()
            || self.sources.iter().any(|source| source.len() > 1024)
        {
            return Err("duplicate_feedback_field".into());
        }
        Ok(())
    }

    pub fn key(&self) -> (&str, &str) {
        (&self.origin, &self.origin_event_id)
    }
}

/// Canonical JSON used only to compare typed fields. Object keys are sorted;
/// `true`, `1`, `1.0`, and `"1"` stay distinct.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();
            let inner = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(&key).unwrap_or_else(|_| "\"\"".into()),
                        canonical_json(&map[&key])
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{inner}}}")
        }
        Value::Array(items) => {
            let inner = items.iter().map(canonical_json).collect::<Vec<_>>().join(",");
            format!("[{inner}]")
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}

pub fn factor_records(rows: &[Value]) -> Value {
    if rows.is_empty() {
        return serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": []});
    }
    let first = match &rows[0] {
        Value::Object(map) => map,
        _ => {
            return serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": rows});
        }
    };
    let mut template = Map::new();
    for (key, value) in first {
        if rows.iter().all(|row| {
            row.as_object()
                .and_then(|map| map.get(key))
                .is_some_and(|other| canonical_json(other) == canonical_json(value))
        }) {
            template.insert(key.clone(), value.clone());
        }
    }
    let residuals: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut residual = Map::new();
            if let Value::Object(map) = row {
                for (key, value) in map {
                    if !template.contains_key(key) {
                        residual.insert(key.clone(), value.clone());
                    }
                }
            }
            Value::Object(residual)
        })
        .collect();
    let factored = serde_json::json!({
        "codec": FactorCodec::TemplateResidual.as_str(),
        "template": Value::Object(template),
        "residuals": residuals
    });
    let plain = serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": rows});
    if canonical_json(&factored).len() < canonical_json(&plain).len() {
        factored
    } else {
        plain
    }
}

pub fn expand_records(pack: &Value) -> Result<Vec<Value>, String> {
    let codec = pack
        .get("codec")
        .and_then(Value::as_str)
        .ok_or("unknown_factor_codec")?;
    match FactorCodec::parse(codec)? {
        FactorCodec::Raw => pack
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "unknown_factor_codec".into()),
        FactorCodec::TemplateResidual => {
            let template = pack.get("template").cloned().unwrap_or(Value::Object(Map::new()));
            let residuals = pack
                .get("residuals")
                .and_then(Value::as_array)
                .ok_or("unknown_factor_codec")?;
            Ok(residuals
                .iter()
                .map(|row| {
                    let mut out = match &template {
                        Value::Object(map) => map.clone(),
                        _ => Map::new(),
                    };
                    if let Value::Object(extra) = row {
                        for (key, value) in extra {
                            out.insert(key.clone(), value.clone());
                        }
                    }
                    Value::Object(out)
                })
                .collect())
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteScore {
    pub target: String,
    pub utility: f64,
    pub mass: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteEdge {
    pub cue: String,
    pub target: String,
    pub positive: f64,
    pub negative: f64,
}

/// Rebuild cue→assembly edges. One training unit contributes at most once per
/// cue/target. Conflicting labels inside a unit are discarded, not voted.
pub fn route_edges(events: &[LearningEvent], now: i64) -> Result<Vec<RouteEdge>, String> {
    if now < 0 {
        return Err("invalid_read_time".into());
    }
    let mut units: BTreeMap<(String, String, String), Vec<&LearningEvent>> = BTreeMap::new();
    for event in events {
        event.validate()?;
        if event.observed_at > now || !event.kind.counts_as_reward() || event.reward == 0 {
            continue;
        }
        for cue in &event.cues {
            units
                .entry((event.training_unit.clone(), cue.clone(), event.target.clone()))
                .or_default()
                .push(event);
        }
    }
    let mut edges: BTreeMap<(String, String), (f64, f64)> = BTreeMap::new();
    for ((_, cue, target), group) in units {
        let rewards: BTreeSet<i8> = group.iter().map(|event| event.reward).collect();
        if rewards.len() != 1 {
            continue;
        }
        let chosen = group
            .into_iter()
            .min_by_key(|event| (event.observed_at, event.origin.as_str(), event.origin_event_id.as_str()))
            .expect("non-empty group");
        let slot = edges.entry((cue, target)).or_insert((0.0, 0.0));
        if chosen.reward > 0 {
            slot.0 += 1.0;
        } else {
            slot.1 += 1.0;
        }
    }
    Ok(edges
        .into_iter()
        .map(|((cue, target), (positive, negative))| RouteEdge {
            cue,
            target,
            positive,
            negative,
        })
        .collect())
}

pub fn rank_routes(edges: &[RouteEdge], cues: &[String], allowed: &[String], min_mass: f64) -> Vec<RouteScore> {
    let query: BTreeSet<_> = cues.iter().cloned().collect();
    let allowed: BTreeSet<_> = allowed.iter().cloned().collect();
    let mut scores: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for edge in edges {
        if !query.contains(&edge.cue) || !allowed.contains(&edge.target) {
            continue;
        }
        let slot = scores.entry(edge.target.clone()).or_insert((0.0, 0.0));
        slot.0 += edge.positive;
        slot.1 += edge.negative;
    }
    let mut ranked: Vec<_> = scores
        .into_iter()
        .filter_map(|(target, (positive, negative))| {
            let mass = positive + negative;
            if mass < min_mass {
                return None;
            }
            Some(RouteScore {
                target,
                utility: (positive - negative) / (2.0 + mass),
                mass,
            })
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .utility
            .total_cmp(&left.utility)
            .then(left.target.cmp(&right.target))
    });
    ranked
}
