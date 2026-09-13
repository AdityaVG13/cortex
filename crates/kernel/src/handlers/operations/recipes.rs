//! Hand-authored recipe templates: the first production path for recurring
//! memory questions. Each template is a fixed, inspectable plan over the
//! authoritative facts; parameters bind only to typed predicates. An
//! unrecognised recipe name falls back to ordinary recall, never to an
//! invented plan. Agent-proposed recipes are validated structurally and
//! replayed through the same interpreter.

use crate::recipe::{Limits, Predicate, Step};
use serde_json::{json, Value};

pub struct Template {
    pub name: &'static str,
    pub purpose: &'static str,
    pub parameters: &'static [&'static str],
}

pub const TEMPLATES: [Template; 7] = [
    Template {
        name: "current_constraints",
        purpose: "constraint-kind records in scope, optionally on one subject, with whether exceptions exist",
        parameters: &["subject"],
    },
    Template {
        name: "last_verified_outcome",
        purpose: "obligations whose head is checker_verified",
        parameters: &[],
    },
    Template { name: "failed_attempts", purpose: "attempt records that ended in failure", parameters: &[] },
    Template { name: "open_work", purpose: "obligations not yet complete or verified", parameters: &[] },
    Template {
        name: "conflicts",
        purpose: "records whose heads carry different values for one key",
        parameters: &["key", "value_field"],
    },
    Template { name: "what_changed", purpose: "records known after a commit sequence", parameters: &["since_seq"] },
    Template { name: "handoff", purpose: "latest checkpoint plus open work and failed attempts", parameters: &[] },
];

pub fn template(name: &str, params: &Value) -> Option<(Vec<Step>, Vec<String>)> {
    let p = |k: &str| params.get(k).cloned();
    Some(match name {
        "current_constraints" => {
            let mut steps = vec![Step::Select {
                id: "rules".into(),
                relation: "constraint".into(),
            }];
            let mut input = "rules".to_string();
            let mut exception_input = "exceptions".to_string();
            if let Some(subject) = p("subject") {
                steps.push(Step::Filter {
                    id: "on_subject".into(),
                    input: input.clone(),
                    field: "subject".into(),
                    predicate: Predicate::Eq {
                        value: subject.clone(),
                    },
                });
                input = "on_subject".into();
                steps.push(Step::Select {
                    id: "exceptions".into(),
                    relation: "exception".into(),
                });
                steps.push(Step::Filter {
                    id: "exceptions_on_subject".into(),
                    input: "exceptions".into(),
                    field: "subject".into(),
                    predicate: Predicate::Eq { value: subject },
                });
                exception_input = "exceptions_on_subject".into();
            } else {
                steps.push(Step::Select {
                    id: "exceptions".into(),
                    relation: "exception".into(),
                });
            }
            steps.push(Step::Exists {
                id: "has_exceptions".into(),
                input: exception_input,
            });
            steps.push(Step::Project {
                id: "out".into(),
                input,
                fields: vec!["text".into(), "record".into(), "subject".into()],
            });
            (steps, vec!["out".into(), "has_exceptions".into()])
        }
        "last_verified_outcome" => (
            vec![
                Step::Select {
                    id: "o".into(),
                    relation: "obligation".into(),
                },
                Step::Filter {
                    id: "v".into(),
                    input: "o".into(),
                    field: "epistemic".into(),
                    predicate: Predicate::Eq {
                        value: json!("checker_verified"),
                    },
                },
                Step::Project {
                    id: "out".into(),
                    input: "v".into(),
                    fields: vec!["record".into(), "verification".into(), "state".into()],
                },
            ],
            vec!["out".into()],
        ),
        "failed_attempts" => (
            vec![
                Step::Select {
                    id: "f".into(),
                    relation: "failure".into(),
                },
                Step::Project {
                    id: "out".into(),
                    input: "f".into(),
                    fields: vec![
                        "record".into(),
                        "failure".into(),
                        "environment".into(),
                        "procedure".into(),
                        "text".into(),
                    ],
                },
            ],
            vec!["out".into()],
        ),
        "open_work" => (
            vec![
                Step::Select {
                    id: "o".into(),
                    relation: "obligation".into(),
                },
                Step::Filter {
                    id: "open".into(),
                    input: "o".into(),
                    field: "state".into(),
                    predicate: Predicate::In {
                        values: vec![
                            json!("proposed"),
                            json!("ready"),
                            json!("in_progress"),
                            json!("blocked"),
                            json!("reopened"),
                        ],
                    },
                },
                Step::Project {
                    id: "out".into(),
                    input: "open".into(),
                    fields: vec!["record".into(), "state".into(), "title".into()],
                },
                Step::Count {
                    id: "count".into(),
                    input: "open".into(),
                },
            ],
            vec!["out".into(), "count".into()],
        ),
        "conflicts" => {
            let key = p("key")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "subject".into());
            let value_field = p("value_field")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "text".into());
            (
                vec![
                    Step::Select {
                        id: "d".into(),
                        relation: "decision".into(),
                    },
                    Step::Conflict {
                        id: "out".into(),
                        input: "d".into(),
                        key,
                        value_field,
                    },
                ],
                vec!["out".into()],
            )
        }
        "what_changed" => {
            let since = p("since_seq").and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
                    .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
            }).unwrap_or(0);
            (
                vec![
                    Step::Select {
                        id: "d".into(),
                        relation: "decision".into(),
                    },
                    Step::TemporalSlice {
                        id: "before".into(),
                        input: "d".into(),
                        valid_at: i64::MAX / 2,
                        known_seq: since,
                    },
                    Step::Count {
                        id: "known_before".into(),
                        input: "before".into(),
                    },
                    Step::Count {
                        id: "known_now".into(),
                        input: "d".into(),
                    },
                    Step::Project {
                        id: "out".into(),
                        input: "d".into(),
                        fields: vec!["record".into(), "text".into()],
                    },
                ],
                vec!["known_before".into(), "known_now".into(), "out".into()],
            )
        }
        "handoff" => (
            vec![
                Step::Select {
                    id: "c".into(),
                    relation: "checkpoint".into(),
                },
                Step::Project {
                    id: "checkpoints".into(),
                    input: "c".into(),
                    fields: vec!["record".into(), "goal".into(), "state".into()],
                },
                Step::Select {
                    id: "o".into(),
                    relation: "obligation".into(),
                },
                Step::Filter {
                    id: "open".into(),
                    input: "o".into(),
                    field: "state".into(),
                    predicate: Predicate::In {
                        values: vec![
                            json!("proposed"),
                            json!("ready"),
                            json!("in_progress"),
                            json!("blocked"),
                            json!("reopened"),
                        ],
                    },
                },
                Step::Project {
                    id: "open_work".into(),
                    input: "open".into(),
                    fields: vec!["record".into(), "state".into()],
                },
                Step::Select {
                    id: "f".into(),
                    relation: "failure".into(),
                },
                Step::Project {
                    id: "failed".into(),
                    input: "f".into(),
                    fields: vec!["record".into(), "failure".into()],
                },
            ],
            vec!["checkpoints".into(), "open_work".into(), "failed".into()],
        ),
        _ => return None,
    })
}

/// Validate an agent-proposed plan structurally by running it against an
/// empty snapshot with tight limits: unknown operators fail at parse time,
/// cycles/limits at evaluation. No natural-language interpretation.
pub fn validate_proposed(plan: &Value) -> Result<(Vec<Step>, Vec<String>), String> {
    let steps: Vec<Step> = serde_json::from_value(plan.get("steps").cloned().unwrap_or(json!([])))
        .map_err(|e| format!("invalid step: {e}"))?;
    let outputs: Vec<String> =
        serde_json::from_value(plan.get("outputs").cloned().unwrap_or(json!([])))
            .map_err(|e| format!("invalid outputs: {e}"))?;
    if steps.is_empty() || outputs.is_empty() {
        return Err("a recipe needs steps and outputs".into());
    }
    let empty = crate::recipe::Snapshot::default();
    crate::recipe::evaluate(&empty, "default", &steps, &outputs, Limits::default())
        .map_err(|e| format!("structural validation failed: {e}"))?;
    Ok((steps, outputs))
}
