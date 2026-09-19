//! Bounded, read-only recipe interpreter.
//!
//! Operators: Select, Filter, Join, Project, TemporalSlice, Compare,
//! Count/Exists, Conflict, Guard, Render. Every step has row, byte, depth and
//! work limits; cycles, unknown operators, unbounded fan-out and dynamic
//! evaluation are rejected. There is no network, process, filesystem or
//! arbitrary eval. Results record positive dependencies (facts read) and
//! negative dependencies (the (scope, relation) epochs of every domain
//! searched, including empty ones), so a later insert into a searched range
//! invalidates the result even though no old fact changed.
//!
//! Determinism class: pure over the snapshot; iteration order is canonical.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const OPERATOR_VERSIONS: &str = "select/1 filter/1 join/1 project/1 temporal_slice/1 compare/1 count/1 exists/1 conflict/1 guard/1 render/1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub scope: String,
    pub relation: String,
    pub fields: BTreeMap<String, Value>,
    /// Half-open validity in epoch seconds; `None` = unbounded.
    #[serde(default)]
    pub valid_from: Option<i64>,
    #[serde(default)]
    pub valid_until: Option<i64>,
    /// Commit sequence the fact became known at.
    #[serde(default)]
    pub known_seq: i64,
}

/// A coherent read snapshot: facts plus the guard epochs of every
/// (scope, relation) domain, and the epochs the result binds to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub facts: BTreeMap<String, Fact>,
    pub epochs: BTreeMap<(String, String), i64>,
    pub brain_epoch: String,
    pub policy_epoch: String,
    pub environment: String,
}

impl Snapshot {
    pub fn touch(&mut self, scope: &str, relation: &str) {
        *self
            .epochs
            .entry((scope.to_string(), relation.to_string()))
            .or_insert(0) += 1;
    }
    pub fn put(&mut self, fact: Fact) {
        if let Some(old) = self.facts.get(&fact.id).cloned() {
            self.touch(&old.scope, &old.relation);
        }
        self.touch(&fact.scope, &fact.relation);
        self.facts.insert(fact.id.clone(), fact);
    }
    pub fn remove(&mut self, id: &str) {
        if let Some(old) = self.facts.remove(id) {
            self.touch(&old.scope, &old.relation);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_steps: usize,
    pub max_rows: usize,
    pub max_work: usize,
    pub max_bytes: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 32,
            max_rows: 256,
            max_work: 4096,
            max_bytes: 64 * 1024,
            max_depth: 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op")]
pub enum Step {
    Select {
        id: String,
        relation: String,
    },
    Filter {
        id: String,
        input: String,
        field: String,
        predicate: Predicate,
    },
    Join {
        id: String,
        left: String,
        right: String,
        on: String,
    },
    Project {
        id: String,
        input: String,
        fields: Vec<String>,
    },
    TemporalSlice {
        id: String,
        input: String,
        valid_at: i64,
        known_seq: i64,
    },
    Compare {
        id: String,
        left: String,
        right: String,
        fields: Vec<String>,
    },
    Count {
        id: String,
        input: String,
    },
    Exists {
        id: String,
        input: String,
    },
    Conflict {
        id: String,
        input: String,
        key: String,
        value_field: String,
    },
    Guard {
        id: String,
        brain_epoch: Option<String>,
        policy_epoch: Option<String>,
        environment: Option<String>,
    },
    Render {
        id: String,
        input: String,
        template: String,
    },
}

/// Select names one relation, or several joined by `|`.
/// Constraint-like kinds (`policy`/`rule`/…) are distinct record relations
/// (deposit copies `decisions.type`); a Select of only `constraint` both
/// misses those rows and fails to watch their guard epochs.

impl Step {
    pub fn id(&self) -> &str {
        match self {
            Step::Select { id, .. }
            | Step::Filter { id, .. }
            | Step::Join { id, .. }
            | Step::Project { id, .. }
            | Step::TemporalSlice { id, .. }
            | Step::Compare { id, .. }
            | Step::Count { id, .. }
            | Step::Exists { id, .. }
            | Step::Conflict { id, .. }
            | Step::Guard { id, .. }
            | Step::Render { id, .. } => id,
        }
    }
    fn inputs(&self) -> Vec<&str> {
        match self {
            Step::Select { .. } | Step::Guard { .. } => vec![],
            Step::Filter { input, .. }
            | Step::Project { input, .. }
            | Step::TemporalSlice { input, .. }
            | Step::Count { input, .. }
            | Step::Exists { input, .. }
            | Step::Conflict { input, .. }
            | Step::Render { input, .. } => vec![input.as_str()],
            Step::Join { left, right, .. } | Step::Compare { left, right, .. } => {
                vec![left.as_str(), right.as_str()]
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Predicate {
    Eq { value: Value },
    Ne { value: Value },
    Lt { value: f64 },
    Gt { value: f64 },
    Interval { from: f64, until: f64 },
    In { values: Vec<Value> },
}

impl Predicate {
    fn matches(&self, v: Option<&Value>) -> bool {
        match self {
            Predicate::Eq { value } => v == Some(value),
            // Missing fields are not "not equal": they failed to present a
            // comparable value, same as Lt/Gt/In on an absent field.
            Predicate::Ne { value } => v.is_some_and(|got| got != value),
            Predicate::Lt { value } => v.and_then(Value::as_f64).is_some_and(|x| x < *value),
            Predicate::Gt { value } => v.and_then(Value::as_f64).is_some_and(|x| x > *value),
            Predicate::Interval { from, until } => v
                .and_then(Value::as_f64)
                .is_some_and(|x| x >= *from && x < *until),
            Predicate::In { values } => v.is_some_and(|x| values.contains(x)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeError {
    InvalidLimits,
    StepLimit,
    RowLimit,
    WorkLimit,
    ByteLimit,
    DepthLimit,
    Cycle,
    DuplicateStepId,
    UnknownInput(String),
    UnknownOutput(String),
    GuardFailed(String),
    UnsupportedFeature(String),
}

impl std::fmt::Display for RecipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string(self).unwrap_or_else(|_| "recipe_error".into())
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecipeResult {
    pub values: BTreeMap<String, Value>,
    /// Negative/range dependencies: epoch of every domain searched.
    pub guards: BTreeMap<(String, String), i64>,
    /// Positive dependencies: exact facts that flowed into the outputs.
    pub positive: BTreeSet<String>,
    pub brain_epoch: String,
    pub policy_epoch: String,
    pub environment: String,
    pub scope: String,
    pub work: usize,
    pub operator_versions: &'static str,
}

impl RecipeResult {
    /// Reusable only under the same scope, epochs, environment and every
    /// searched domain's epoch — an insert into a searched range changes an
    /// epoch even when no fact the result read was modified.
    pub fn reusable(&self, current: &Snapshot, scope: &str) -> Result<(), String> {
        if scope != self.scope {
            return Err(format!("scope differs: {scope} vs {}", self.scope));
        }
        if current.brain_epoch != self.brain_epoch || current.policy_epoch != self.policy_epoch {
            return Err("brain or policy epoch changed".into());
        }
        if current.environment != self.environment {
            return Err("environment fingerprint changed".into());
        }
        for (key, expected) in &self.guards {
            let now = current.epochs.get(key).copied().unwrap_or(0);
            if now != *expected {
                return Err(format!(
                    "guard {}/{} moved {expected} -> {now}",
                    key.0, key.1
                ));
            }
        }
        for id in &self.positive {
            if !current.facts.contains_key(id) {
                return Err(format!("positive dependency {id} no longer present"));
            }
        }
        Ok(())
    }
}

mod eval;
pub use eval::evaluate;
