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
            Predicate::Ne { value } => v != Some(value),
            Predicate::Lt { value } => v
                .and_then(Value::as_f64)
                .map(|x| x < *value)
                .unwrap_or(false),
            Predicate::Gt { value } => v
                .and_then(Value::as_f64)
                .map(|x| x > *value)
                .unwrap_or(false),
            Predicate::Interval { from, until } => v
                .and_then(Value::as_f64)
                .map(|x| x >= *from && x < *until)
                .unwrap_or(false),
            Predicate::In { values } => v.map(|x| values.contains(x)).unwrap_or(false),
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

pub fn evaluate(
    snapshot: &Snapshot,
    scope: &str,
    steps: &[Step],
    outputs: &[String],
    limits: Limits,
) -> Result<RecipeResult, RecipeError> {
    if limits.max_steps < 1
        || limits.max_rows < 1
        || limits.max_work < 1
        || limits.max_bytes < 1
        || limits.max_depth < 1
    {
        return Err(RecipeError::InvalidLimits);
    }
    if steps.len() > limits.max_steps {
        return Err(RecipeError::StepLimit);
    }
    // Static checks: unique ids, inputs defined earlier (no cycles), depth.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    for step in steps {
        if !seen.insert(step.id()) {
            return Err(RecipeError::DuplicateStepId);
        }
        let mut d = 0;
        for input in step.inputs() {
            if input == step.id() {
                return Err(RecipeError::Cycle);
            }
            let Some(input_depth) = depth.get(input) else {
                return Err(if seen.contains(input) {
                    RecipeError::Cycle
                } else {
                    RecipeError::UnknownInput(input.to_string())
                });
            };
            d = d.max(input_depth + 1);
        }
        if d > limits.max_depth {
            return Err(RecipeError::DepthLimit);
        }
        depth.insert(step.id(), d);
    }
    let mut values: BTreeMap<String, Value> = BTreeMap::new();
    let mut rows_by_id: BTreeMap<String, Vec<Fact>> = BTreeMap::new();
    let mut guards = BTreeMap::new();
    let mut positive = BTreeSet::new();
    let mut work = 0usize;
    let mut spend = |n: usize, work: &mut usize| -> Result<(), RecipeError> {
        *work += n;
        if *work > limits.max_work {
            Err(RecipeError::WorkLimit)
        } else {
            Ok(())
        }
    };
    for step in steps {
        spend(1, &mut work)?;
        match step {
            Step::Select { id, relation } => {
                let key = (scope.to_string(), relation.clone());
                guards.insert(key.clone(), snapshot.epochs.get(&key).copied().unwrap_or(0));
                let mut rows = Vec::new();
                for fact in snapshot.facts.values() {
                    spend(1, &mut work)?;
                    if fact.scope == scope && &fact.relation == relation {
                        rows.push(fact.clone());
                    }
                }
                if rows.len() > limits.max_rows {
                    return Err(RecipeError::RowLimit);
                }
                rows_by_id.insert(id.clone(), rows);
            }
            Step::Filter {
                id,
                input,
                field,
                predicate,
            } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                rows_by_id.insert(
                    id.clone(),
                    rows.into_iter()
                        .filter(|f| predicate.matches(f.fields.get(field)))
                        .collect(),
                );
            }
            Step::Join {
                id,
                left,
                right,
                on,
            } => {
                let l = rows_by_id.get(left).cloned().unwrap_or_default();
                let r = rows_by_id.get(right).cloned().unwrap_or_default();
                spend(l.len() * r.len().max(1), &mut work)?;
                let mut out = Vec::new();
                for a in &l {
                    for b in &r {
                        if a.fields.get(on).is_some() && a.fields.get(on) == b.fields.get(on) {
                            let mut fields = a.fields.clone();
                            for (k, v) in &b.fields {
                                fields.entry(format!("right.{k}")).or_insert(v.clone());
                            }
                            out.push(Fact {
                                id: format!("{}+{}", a.id, b.id),
                                scope: a.scope.clone(),
                                relation: format!("{}⋈{}", a.relation, b.relation),
                                fields,
                                valid_from: a.valid_from,
                                valid_until: a.valid_until,
                                known_seq: a.known_seq.max(b.known_seq),
                            });
                            if out.len() > limits.max_rows {
                                return Err(RecipeError::RowLimit);
                            }
                        }
                    }
                }
                rows_by_id.insert(id.clone(), out);
            }
            Step::Project { id, input, fields } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                let projected: Vec<Value> = rows
                    .iter()
                    .map(|f| {
                        positive.insert(f.id.clone());
                        serde_json::json!({"source": f.id, "fields": fields.iter().filter_map(|k| f.fields.get(k).map(|v| (k.clone(), v.clone()))).collect::<BTreeMap<_, _>>()})
                    })
                    .collect();
                values.insert(id.clone(), Value::Array(projected));
                rows_by_id.insert(id.clone(), rows);
            }
            Step::TemporalSlice {
                id,
                input,
                valid_at,
                known_seq,
            } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                rows_by_id.insert(
                    id.clone(),
                    rows.into_iter()
                        .filter(|f| {
                            f.known_seq <= *known_seq
                                && f.valid_from.map(|v| v <= *valid_at).unwrap_or(true)
                                && f.valid_until.map(|u| *valid_at < u).unwrap_or(true)
                        })
                        .collect(),
                );
            }
            Step::Compare {
                id,
                left,
                right,
                fields,
            } => {
                let l = rows_by_id.get(left).cloned().unwrap_or_default();
                let r = rows_by_id.get(right).cloned().unwrap_or_default();
                spend(l.len() + r.len(), &mut work)?;
                let mut diffs = Vec::new();
                for (a, b) in l.iter().zip(r.iter()) {
                    positive.insert(a.id.clone());
                    positive.insert(b.id.clone());
                    for field in fields {
                        if a.fields.get(field) != b.fields.get(field) {
                            diffs.push(serde_json::json!({"field": field, "left": a.fields.get(field), "right": b.fields.get(field), "left_source": a.id, "right_source": b.id}));
                        }
                    }
                }
                values.insert(id.clone(), Value::Array(diffs));
            }
            Step::Count { id, input } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                values.insert(id.clone(), Value::from(rows.len()));
            }
            Step::Exists { id, input } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                values.insert(id.clone(), Value::Bool(!rows.is_empty()));
            }
            Step::Conflict {
                id,
                input,
                key,
                value_field,
            } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                let mut by_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                let mut sources: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for f in &rows {
                    let k = f.fields.get(key).map(|v| v.to_string()).unwrap_or_default();
                    let v = f
                        .fields
                        .get(value_field)
                        .map(|v| v.to_string())
                        .unwrap_or_default();
                    by_key.entry(k.clone()).or_default().insert(v);
                    sources.entry(k).or_default().push(f.id.clone());
                }
                let conflicts: Vec<Value> = by_key.into_iter().filter(|(_, vs)| vs.len() > 1).map(|(k, vs)| serde_json::json!({"key": k, "values": vs, "sources": sources.get(&k)})).collect();
                for c in &conflicts {
                    for s in c["sources"].as_array().into_iter().flatten() {
                        if let Some(s) = s.as_str() {
                            positive.insert(s.to_string());
                        }
                    }
                }
                values.insert(id.clone(), Value::Array(conflicts));
            }
            Step::Guard {
                id,
                brain_epoch,
                policy_epoch,
                environment,
            } => {
                if let Some(b) = brain_epoch {
                    if b != &snapshot.brain_epoch {
                        return Err(RecipeError::GuardFailed(format!(
                            "brain epoch {b} != {}",
                            snapshot.brain_epoch
                        )));
                    }
                }
                if let Some(p) = policy_epoch {
                    if p != &snapshot.policy_epoch {
                        return Err(RecipeError::GuardFailed(format!(
                            "policy epoch {p} != {}",
                            snapshot.policy_epoch
                        )));
                    }
                }
                if let Some(e) = environment {
                    if e != &snapshot.environment {
                        return Err(RecipeError::GuardFailed(format!(
                            "environment {e} != {}",
                            snapshot.environment
                        )));
                    }
                }
                values.insert(id.clone(), Value::Bool(true));
            }
            Step::Render {
                id,
                input,
                template,
            } => {
                let rows = rows_by_id.get(input).cloned().unwrap_or_default();
                spend(rows.len(), &mut work)?;
                if template.contains("{{")
                    && !template.contains("{{text}}")
                    && !template.contains("{{id}}")
                {
                    return Err(RecipeError::UnsupportedFeature(
                        "render template may reference only {{id}} and {{text}}".into(),
                    ));
                }
                let mut out = String::new();
                for f in &rows {
                    positive.insert(f.id.clone());
                    let text = f.fields.get("text").and_then(Value::as_str).unwrap_or("");
                    out.push_str(&template.replace("{{id}}", &f.id).replace("{{text}}", text));
                    out.push('\n');
                    if out.len() > limits.max_bytes {
                        return Err(RecipeError::ByteLimit);
                    }
                }
                values.insert(id.clone(), Value::String(out));
            }
        }
    }
    for output in outputs {
        if !values.contains_key(output) {
            return Err(RecipeError::UnknownOutput(output.clone()));
        }
    }
    let values: BTreeMap<String, Value> = outputs
        .iter()
        .filter_map(|o| values.get(o).map(|v| (o.clone(), v.clone())))
        .collect();
    Ok(RecipeResult {
        values,
        guards,
        positive,
        brain_epoch: snapshot.brain_epoch.clone(),
        policy_epoch: snapshot.policy_epoch.clone(),
        environment: snapshot.environment.clone(),
        scope: scope.to_string(),
        work,
        operator_versions: OPERATOR_VERSIONS,
    })
}
