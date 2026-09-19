//! Hand-authored recipe templates: the first production path for recurring
//! memory questions. Each template is a fixed, inspectable plan over the
//! authoritative facts; parameters bind only to typed predicates. An
//! unrecognised recipe name falls back to ordinary recall, never to an
//! invented plan. Agent-proposed recipes are validated structurally and
//! replayed through the same interpreter.

use crate::recipe::{Predicate, Step};
use serde_json::{Value, json};

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
    Template {
        name: "failed_attempts",
        purpose: "attempt records that ended in failure",
        parameters: &[],
    },
    Template {
        name: "open_work",
        purpose: "obligations not yet complete or verified",
        parameters: &[],
    },
    Template {
        name: "conflicts",
        purpose: "records whose heads carry different values for one key",
        parameters: &["key", "value_field"],
    },
    Template {
        name: "what_changed",
        purpose: "records known after a commit sequence",
        parameters: &["since_seq"],
    },
    Template {
        name: "handoff",
        purpose: "latest checkpoint plus open work and failed attempts",
        parameters: &[],
    },
];

/// Open work matches `agent_open_work`: every obligation that is not
/// `verified_complete` or `cancelled`. `observed_complete` is still open —
/// the checker has not verified it, and `verify_obligation` treats that
/// state as unfinished work.
fn open_obligation_states() -> Vec<Value> {
    vec![
        json!("proposed"),
        json!("ready"),
        json!("in_progress"),
        json!("blocked"),
        json!("observed_complete"),
        json!("reopened"),
    ]
}

fn sel(id: &str, relation: &str) -> Step {
    Step::Select {
        id: id.into(),
        relation: relation.into(),
    }
}
fn proj(id: &str, input: &str, fields: &[&str]) -> Step {
    Step::Project {
        id: id.into(),
        input: input.into(),
        fields: fields.iter().map(|s| (*s).to_string()).collect(),
    }
}
fn filter_eq(id: &str, input: &str, field: &str, value: Value) -> Step {
    Step::Filter {
        id: id.into(),
        input: input.into(),
        field: field.into(),
        predicate: Predicate::Eq { value },
    }
}
fn filter_in(id: &str, input: &str, field: &str, values: Vec<Value>) -> Step {
    Step::Filter {
        id: id.into(),
        input: input.into(),
        field: field.into(),
        predicate: Predicate::In { values },
    }
}
fn count_step(id: &str, input: &str) -> Step {
    Step::Count {
        id: id.into(),
        input: input.into(),
    }
}

pub fn template(name: &str, params: &Value) -> Option<(Vec<Step>, Vec<String>)> {
    let p = |k: &str| params.get(k).cloned();
    Some(match name {
        "current_constraints" => {
            let mut steps = vec![sel(
                "rules",
                // Same family as agent_constraints / boot Constraints /
                // required-role recall. Deposit stores the entry type as
                // records.kind; omitting policy/rule here dropped those
                // rows and skipped their guard epochs.
                "constraint|policy|rule|convention|contract|preference",
            )];
            let mut input = "rules".to_string();
            let mut exception_input = "exceptions".to_string();
            if let Some(subject) = p("subject") {
                steps.push(filter_eq("on_subject", &input, "subject", subject.clone()));
                input = "on_subject".into();
                steps.push(sel("exceptions", "exception"));
                steps.push(filter_eq(
                    "exceptions_on_subject",
                    "exceptions",
                    "subject",
                    subject,
                ));
                exception_input = "exceptions_on_subject".into();
            } else {
                steps.push(sel("exceptions", "exception"));
            }
            steps.push(Step::Exists {
                id: "has_exceptions".into(),
                input: exception_input,
            });
            steps.push(proj("out", &input, &["text", "record", "subject"]));
            (steps, vec!["out".into(), "has_exceptions".into()])
        }
        "last_verified_outcome" => (
            vec![
                sel("o", "obligation"),
                filter_eq("v", "o", "epistemic", json!("checker_verified")),
                proj("out", "v", &["record", "verification", "state"]),
            ],
            vec!["out".into()],
        ),
        "failed_attempts" => (
            vec![
                sel("f", "failure"),
                proj(
                    "out",
                    "f",
                    &["record", "failure", "environment", "procedure", "text"],
                ),
            ],
            vec!["out".into()],
        ),
        "open_work" => (
            vec![
                sel("o", "obligation"),
                filter_in("open", "o", "state", open_obligation_states()),
                proj("out", "open", &["record", "state", "title"]),
                count_step("count", "open"),
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
                    sel("d", "decision"),
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
            let since = p("since_seq")
                .as_ref()
                .and_then(crate::protocol::json_i64)
                .unwrap_or(0);
            (
                vec![
                    sel("d", "decision"),
                    Step::TemporalSlice {
                        id: "before".into(),
                        input: "d".into(),
                        valid_at: i64::MAX / 2,
                        known_seq: since,
                    },
                    Step::Filter {
                        id: "after".into(),
                        input: "d".into(),
                        field: "known_seq".into(),
                        predicate: Predicate::Gt {
                            value: since as f64,
                        },
                    },
                    count_step("known_before", "before"),
                    count_step("known_now", "d"),
                    proj("out", "after", &["record", "text"]),
                ],
                vec!["known_before".into(), "known_now".into(), "out".into()],
            )
        }
        "handoff" => (
            vec![
                sel("c", "checkpoint"),
                proj("checkpoints", "c", &["record", "goal", "state"]),
                sel("o", "obligation"),
                filter_in("open", "o", "state", open_obligation_states()),
                proj("open_work", "open", &["record", "state"]),
                sel("f", "failure"),
                proj("failed", "f", &["record", "failure"]),
            ],
            vec!["checkpoints".into(), "open_work".into(), "failed".into()],
        ),
        _ => return None,
    })
}

mod run;
pub(in crate::handlers::operations) use run::run_recipe;
pub use run::validate_proposed;
