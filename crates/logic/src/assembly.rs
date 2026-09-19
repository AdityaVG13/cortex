//! Reversible assemblies and attributed learning math.
//!
//! Membership is exact. Factoring is typed JSON, not a summary. Ranking is
//! scoped utility from authorized events, never a truth score.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

macro_rules! str_enum_impl {
    ($err:expr; $($variant:ident => $lit:literal),+ $(,)?) => {
        pub fn as_str(self) -> &'static str {
            match self {
                $(Self::$variant => $lit,)+
            }
        }
        pub fn parse(raw: &str) -> Result<Self, String> {
            match raw {
                $($lit => Ok(Self::$variant),)+
                _ => Err($err.into()),
            }
        }
    };
}

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
    str_enum_impl!(
        "invalid_membership_role";
        Observation => "observation",
        Support => "support",
        Exception => "exception",
        Contradiction => "contradiction",
        Qualification => "qualification",
    );

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
    str_enum_impl!(
        "unknown_factor_codec";
        Raw => "raw",
        TemplateResidual => "template_residual",
    );
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
    str_enum_impl!(
        "invalid_feedback_kind";
        Explicit => "explicit",
        VerifiedUse => "verified_use",
        Exposure => "exposure",
        Uncertain => "uncertain",
        Structural => "structural",
    );

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
        unique_bounded(&self.cues, false, "invalid_feedback_cues")?;
        unique_bounded(&self.sources, true, "duplicate_feedback_field")?;
        Ok(())
    }

    pub fn key(&self) -> (&str, &str) {
        (&self.origin, &self.origin_event_id)
    }
}

fn unique_bounded(items: &[String], allow_empty: bool, err: &str) -> Result<(), String> {
    let unique: BTreeSet<_> = items.iter().collect();
    if unique.len() != items.len()
        || items
            .iter()
            .any(|item| (!allow_empty && item.is_empty()) || item.len() > 1024)
    {
        return Err(err.into());
    }
    Ok(())
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
            let inner = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{inner}]")
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}

mod factor;
pub use factor::{expand_records, factor_records};
mod routes;
pub use routes::{RouteEdge, RouteScore, rank_routes, route_edges};
