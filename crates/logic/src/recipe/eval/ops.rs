use super::{EvalCtx, guard_epoch};
use crate::protocol::nonempty_str;
use crate::recipe::{Fact, RecipeError, Step};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn select_relation_names(relation: &str) -> Vec<&str> {
    let names: Vec<&str> = relation.split('|').filter_map(nonempty_str).collect();
    if names.is_empty() {
        vec![relation]
    } else {
        names
    }
}

impl EvalCtx<'_> {
    pub(super) fn apply(&mut self, step: &Step) -> Result<(), RecipeError> {
        match step {
            Step::Select { id, relation } => self.select(id, relation),
            Step::Filter {
                id,
                input,
                field,
                predicate,
            } => self.retain(id, input, |f| predicate.matches(f.fields.get(field))),
            Step::Join {
                id,
                left,
                right,
                on,
            } => self.join(id, left, right, on),
            Step::Project { id, input, fields } => self.project(id, input, fields),
            Step::TemporalSlice {
                id,
                input,
                valid_at,
                known_seq,
            } => self.retain(id, input, |f| {
                f.known_seq <= *known_seq
                    && f.valid_from.map(|v| v <= *valid_at).unwrap_or(true)
                    && f.valid_until.map(|u| *valid_at < u).unwrap_or(true)
            }),
            Step::Compare {
                id,
                left,
                right,
                fields,
            } => self.compare(id, left, right, fields),
            Step::Count { id, input } => self.count_or_exists(id, input, true),
            Step::Exists { id, input } => self.count_or_exists(id, input, false),
            Step::Conflict {
                id,
                input,
                key,
                value_field,
            } => self.conflict(id, input, key, value_field),
            Step::Guard {
                id,
                brain_epoch,
                policy_epoch,
                environment,
            } => self.guard(id, brain_epoch, policy_epoch, environment),
            Step::Render {
                id,
                input,
                template,
            } => self.render(id, input, template),
        }
    }

    fn select(&mut self, id: &str, relation: &str) -> Result<(), RecipeError> {
        let relations = select_relation_names(relation);
        for rel in &relations {
            let key = (self.scope.to_string(), (*rel).to_string());
            self.guards.insert(
                key.clone(),
                self.snapshot.epochs.get(&key).copied().unwrap_or(0),
            );
        }
        let mut rows = Vec::new();
        for fact in self.snapshot.facts.values() {
            self.spend(1)?;
            if fact.scope == self.scope && relations.iter().any(|rel| fact.relation == *rel) {
                rows.push(fact.clone());
            }
        }
        if rows.len() > self.limits.max_rows {
            return Err(RecipeError::RowLimit);
        }
        self.rows_by_id.insert(id.to_string(), rows);
        Ok(())
    }

    fn join(&mut self, id: &str, left: &str, right: &str, on: &str) -> Result<(), RecipeError> {
        let l = self.rows(left)?;
        let r = self.rows(right)?;
        self.spend(l.len() * r.len().max(1))?;
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
                    if out.len() > self.limits.max_rows {
                        return Err(RecipeError::RowLimit);
                    }
                }
            }
        }
        self.rows_by_id.insert(id.to_string(), out);
        Ok(())
    }

    fn project(&mut self, id: &str, input: &str, fields: &[String]) -> Result<(), RecipeError> {
        let rows = self.rows(input)?;
        self.spend(rows.len())?;
        let projected: Vec<Value> = rows.iter().map(|f| { self.positive.insert(f.id.clone()); serde_json::json!({"source": f.id, "fields": fields.iter().filter_map(|k| f.fields.get(k).map(|v| (k.clone(), v.clone()))).collect::<BTreeMap<_, _>>()}) }).collect();
        self.values.insert(id.to_string(), Value::Array(projected));
        self.rows_by_id.insert(id.to_string(), rows);
        Ok(())
    }

    fn compare(
        &mut self,
        id: &str,
        left: &str,
        right: &str,
        fields: &[String],
    ) -> Result<(), RecipeError> {
        let l = self.rows(left)?;
        let r = self.rows(right)?;
        self.spend(l.len() + r.len())?;
        let mut diffs = Vec::new();
        for (a, b) in l.iter().zip(r.iter()) {
            self.positive.insert(a.id.clone());
            self.positive.insert(b.id.clone());
            for field in fields {
                if a.fields.get(field) != b.fields.get(field) {
                    diffs.push(serde_json::json!({"field": field, "left": a.fields.get(field), "right": b.fields.get(field), "left_source": a.id, "right_source": b.id}));
                }
            }
        }
        self.values.insert(id.to_string(), Value::Array(diffs));
        Ok(())
    }

    fn count_or_exists(&mut self, id: &str, input: &str, count: bool) -> Result<(), RecipeError> {
        let rows = self.rows(input)?;
        self.spend(rows.len())?;
        self.values.insert(
            id.to_string(),
            if count {
                Value::from(rows.len())
            } else {
                Value::Bool(!rows.is_empty())
            },
        );
        Ok(())
    }

    fn conflict(
        &mut self,
        id: &str,
        input: &str,
        key: &str,
        value_field: &str,
    ) -> Result<(), RecipeError> {
        let rows = self.rows(input)?;
        self.spend(rows.len())?;
        let mut by_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut sources: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in &rows {
            let Some(k) = f.fields.get(key).map(|v| v.to_string()) else {
                continue;
            };
            let v = f
                .fields
                .get(value_field)
                .map(|v| v.to_string())
                .unwrap_or_default();
            by_key.entry(k.clone()).or_default().insert(v);
            sources.entry(k).or_default().push(f.id.clone());
        }
        let conflicts: Vec<Value> = by_key
            .into_iter()
            .filter(|(_, vs)| vs.len() > 1)
            .map(|(k, vs)| serde_json::json!({"key": k, "values": vs, "sources": sources.get(&k)}))
            .collect();
        for c in &conflicts {
            for s in c["sources"].as_array().into_iter().flatten() {
                if let Some(s) = s.as_str() {
                    self.positive.insert(s.to_string());
                }
            }
        }
        self.values.insert(id.to_string(), Value::Array(conflicts));
        Ok(())
    }

    fn guard(
        &mut self,
        id: &str,
        brain_epoch: &Option<String>,
        policy_epoch: &Option<String>,
        environment: &Option<String>,
    ) -> Result<(), RecipeError> {
        guard_epoch(
            brain_epoch.as_ref(),
            &self.snapshot.brain_epoch,
            "brain epoch",
        )?;
        guard_epoch(
            policy_epoch.as_ref(),
            &self.snapshot.policy_epoch,
            "policy epoch",
        )?;
        guard_epoch(
            environment.as_ref(),
            &self.snapshot.environment,
            "environment",
        )?;
        self.values.insert(id.to_string(), Value::Bool(true));
        Ok(())
    }

    fn render(&mut self, id: &str, input: &str, template: &str) -> Result<(), RecipeError> {
        let rows = self.rows(input)?;
        self.spend(rows.len())?;
        if template.contains("{{") && !template.contains("{{text}}") && !template.contains("{{id}}")
        {
            return Err(RecipeError::UnsupportedFeature(
                "render template may reference only {{id}} and {{text}}".into(),
            ));
        }
        let mut out = String::new();
        for f in &rows {
            self.positive.insert(f.id.clone());
            let text = f.fields.get("text").and_then(Value::as_str).unwrap_or("");
            out.push_str(&template.replace("{{id}}", &f.id).replace("{{text}}", text));
            out.push('\n');
            if out.len() > self.limits.max_bytes {
                return Err(RecipeError::ByteLimit);
            }
        }
        self.values.insert(id.to_string(), Value::String(out));
        Ok(())
    }
}
