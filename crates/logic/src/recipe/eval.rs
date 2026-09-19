use super::{Fact, Limits, OPERATOR_VERSIONS, RecipeError, RecipeResult, Snapshot, Step};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

mod ops;

struct EvalCtx<'a> {
    snapshot: &'a Snapshot,
    scope: &'a str,
    limits: Limits,
    values: BTreeMap<String, Value>,
    rows_by_id: BTreeMap<String, Vec<Fact>>,
    guards: BTreeMap<(String, String), i64>,
    positive: BTreeSet<String>,
    work: usize,
}

impl EvalCtx<'_> {
    fn spend(&mut self, n: usize) -> Result<(), RecipeError> {
        self.work += n;
        if self.work > self.limits.max_work {
            Err(RecipeError::WorkLimit)
        } else {
            Ok(())
        }
    }

    fn rows(&self, input: &str) -> Result<Vec<Fact>, RecipeError> {
        self.rows_by_id
            .get(input)
            .cloned()
            .ok_or_else(|| RecipeError::UnknownInput(input.to_string()))
    }

    fn retain(
        &mut self,
        id: &str,
        input: &str,
        pred: impl Fn(&Fact) -> bool,
    ) -> Result<(), RecipeError> {
        let rows = self.rows(input)?;
        self.spend(rows.len())?;
        self.rows_by_id
            .insert(id.to_string(), rows.into_iter().filter(pred).collect());
        Ok(())
    }

    fn finish(self, outputs: &[String], scope: &str) -> Result<RecipeResult, RecipeError> {
        for output in outputs {
            if !self.values.contains_key(output) {
                return Err(RecipeError::UnknownOutput(output.clone()));
            }
        }
        let values: BTreeMap<String, Value> = outputs
            .iter()
            .filter_map(|o| self.values.get(o).map(|v| (o.clone(), v.clone())))
            .collect();
        Ok(RecipeResult {
            values,
            guards: self.guards,
            positive: self.positive,
            brain_epoch: self.snapshot.brain_epoch.clone(),
            policy_epoch: self.snapshot.policy_epoch.clone(),
            environment: self.snapshot.environment.clone(),
            scope: scope.to_string(),
            work: self.work,
            operator_versions: OPERATOR_VERSIONS,
        })
    }
}

fn guard_epoch(expected: Option<&String>, actual: &str, label: &str) -> Result<(), RecipeError> {
    if let Some(value) = expected {
        if value != actual {
            return Err(RecipeError::GuardFailed(format!(
                "{label} {value} != {actual}"
            )));
        }
    }
    Ok(())
}

fn validate_limits(limits: &Limits) -> Result<(), RecipeError> {
    if limits.max_steps < 1
        || limits.max_rows < 1
        || limits.max_work < 1
        || limits.max_bytes < 1
        || limits.max_depth < 1
    {
        Err(RecipeError::InvalidLimits)
    } else {
        Ok(())
    }
}

fn validate_plan(steps: &[Step], limits: &Limits) -> Result<(), RecipeError> {
    if steps.len() > limits.max_steps {
        return Err(RecipeError::StepLimit);
    }
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
    Ok(())
}

pub fn evaluate(
    snapshot: &Snapshot,
    scope: &str,
    steps: &[Step],
    outputs: &[String],
    limits: Limits,
) -> Result<RecipeResult, RecipeError> {
    validate_limits(&limits)?;
    validate_plan(steps, &limits)?;
    let mut ctx = EvalCtx {
        snapshot,
        scope,
        limits,
        values: BTreeMap::new(),
        rows_by_id: BTreeMap::new(),
        guards: BTreeMap::new(),
        positive: BTreeSet::new(),
        work: 0,
    };
    for step in steps {
        ctx.spend(1)?;
        ctx.apply(step)?;
    }
    ctx.finish(outputs, scope)
}
