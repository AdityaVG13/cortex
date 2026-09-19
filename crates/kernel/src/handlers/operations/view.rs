//! View and Card: the answer shape every read operation returns.
//!
//! A Card carries a statement, epistemic state, applicability, freshness and
//! an expansion handle (alias bound to this View's receipt). Coverage maps
//! declared needs to delivered evidence; unmet needs are named, never
//! silently dropped. Leads (unsupported candidates) are separate from Cards.

use crate::lens::Need;
use crate::protocol::{ContextPresence, ResponseStatus};
use rusqlite::Connection;
use serde_json::{Value, json};
mod budget;
mod evidence;
mod from;
mod receipt;

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub alias: String,
    /// Brief label: Constraint c1 / Known k1 / Conflict x1 / Attempt f1.
    pub label: String,
    pub kind: String,
    pub retention: String,
    /// Required bundles (protected constraints, contested claims) can never
    /// be dropped for budget; optional ones can, and are listed as omissions.
    pub required: bool,
    /// Alias is bound to this View's receipt and can be expanded. Persist
    /// skips cards without a live revision (FK on view_aliases); those must
    /// not advertise an expand handle.
    pub expandable: bool,
    pub exact_text: Option<String>,
    pub reference: String,
    pub statement: String,
    pub epistemic: &'static str,
    pub applicability: &'static str,
    pub exceptions: Vec<String>,
    pub freshness: Option<String>,
    pub admitted_by: Option<String>,
    pub arms: Vec<String>,
    pub bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    pub covered: Vec<String>,
    pub unmet: Vec<String>,
    pub partitions: Value,
    /// Optional Cards left out for budget, by reference, with their cost.
    pub omissions: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PresenceInputs {
    pub presence: Option<ContextPresence>,
    pub attested_brain: Option<String>,
    pub attested_policy: Option<String>,
    pub context_epoch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub status: ResponseStatus,
    pub include_cold: bool,
    pub needs: Vec<Need>,
    pub presence: Option<PresenceInputs>,
    /// Cards attested present in the current invocation: alias + revision
    /// only, payload omitted (a transport saving, never a token claim).
    pub present: Vec<Value>,
    pub change_cursor_in: Option<String>,
    pub change_cursor_out: Option<String>,
    pub changes: Vec<Value>,
    pub cursor_status: Option<ResponseStatus>,
    pub profile: String,
    pub cards: Vec<Card>,
    pub leads: Vec<Value>,
    /// Candidates the engine collected but did not admit. When nothing is
    /// admitted yet material exists, the status is `ambiguous`, never
    /// `no_match`: a hidden cutoff must not masquerade as absence.
    pub engine_leads: usize,
    pub coverage: Coverage,
    pub budget_bytes: usize,
    pub used_bytes: usize,
    pub receipt_id: String,
    pub frontier: Option<Value>,
    pub unresolved: Vec<String>,
    pub required_plan_bytes: Option<usize>,
}

fn coverage_labels(needs: &[Need], cards: &[Card]) -> (Vec<String>, Vec<String>) {
    let mut covered = Vec::new();
    let mut unmet = Vec::new();
    for need in needs {
        if need_covered(need, cards) {
            covered.push(Need::label(need));
        } else {
            unmet.push(Need::label(need));
        }
    }
    (covered, unmet)
}

impl View {
    pub fn to_json(&self) -> Value {
        json!({
            "status": self.status.as_str(),
            "present": self.present,
            "change_cursor": self.change_cursor_out,
            "changes": self.changes,
            "cursor_status": self.cursor_status.map(|s| s.as_str()),
            "profile": self.profile,
            "receipt": self.receipt_id,
            "frontier": self.frontier,
            "cards": self.cards.iter().map(|c| json!({
                "alias": c.alias, "label": c.label, "statement": c.statement, "epistemic": c.epistemic, "applicability": c.applicability,
                "exceptions": c.exceptions, "freshness": c.freshness, "required": c.required,
                "expand": if c.expandable { json!({"alias": c.alias, "receipt": self.receipt_id}) } else { Value::Null },
                // Memory is data: a card is a recalled claim with provenance,
                // never an instruction to the reader and never a policy input.
                "trust": {"kind": "recalled_claim", "instruction": false, "privilege": "none", "provenance": c.reference},
                "exact": c.exact_text,
                "sidecar": {"reference": c.reference, "kind": c.kind, "retention": c.retention, "admitted_by": c.admitted_by, "arms": c.arms, "bytes": c.bytes}
            })).collect::<Vec<_>>(),
            "leads": self.leads,
            "coverage": {"covered": self.coverage.covered, "unmet": self.coverage.unmet, "partitions": self.coverage.partitions, "omissions": self.coverage.omissions},
            "labels": self.cards.iter().map(|c| json!({"label": c.label, "alias": c.alias})).collect::<Vec<_>>(),
            "budget": {"bytes": self.budget_bytes, "used_bytes": self.used_bytes, "unit": "utf8_bytes", "tokenizer": null},
            "required_plan_bytes": self.required_plan_bytes,
            "unresolved": self.unresolved,
            "brief": self.brief(),
        })
    }

    /// Readable situation brief; labels map to aliases in `labels`.
    /// Machine metadata is not rendered again as prose.
    pub fn brief(&self) -> String {
        let mut lines = Vec::new();
        for card in &self.cards {
            let tag = match card.label.chars().next() {
                Some('x') => "Conflict",
                Some('c') => "Constraint",
                Some('f') => "Attempt",
                _ => "Known",
            };
            let tag = if card.epistemic == "retracted" {
                "Retracted"
            } else {
                tag
            };
            let limit = if card.applicability == "applicable" {
                String::new()
            } else {
                format!(" [{}]", card.applicability)
            };
            lines.push(format!("{tag} {}: {}{limit}", card.label, card.statement));
            for (i, exception) in card.exceptions.iter().enumerate() {
                lines.push(format!("  Boundary b{}: {exception}", i + 1));
            }
        }
        if !self.cards.is_empty() {
            let expandable: Vec<&str> = self
                .cards
                .iter()
                .filter(|c| c.expandable)
                .map(|c| c.alias.as_str())
                .collect();
            if !expandable.is_empty() {
                lines.push(format!(
                    "Evidence: expand {} for exact sources (receipt {})",
                    expandable.join(", "),
                    self.receipt_id
                ));
            }
        }
        if !self.coverage.unmet.is_empty() {
            lines.push(format!("Unmet: {}", self.coverage.unmet.join(", ")));
        }
        if let Some(bytes) = self.required_plan_bytes {
            lines.push(format!(
                "Needs more budget: a safe plan is {bytes} bytes (budget {})",
                self.budget_bytes
            ));
        }
        lines.join("\n")
    }
}

fn json_count(value: &Value) -> usize {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .unwrap_or(0) as usize
}

pub(super) fn mark_contested(card: &mut Card, reason: &str) {
    card.epistemic = "contested";
    card.exceptions.push(reason.into());
    card.required = true;
    card.bytes = card.statement.len()
        + card.exceptions.iter().map(|e| e.len() + 2).sum::<usize>()
        + card.exact_text.as_ref().map(|t| t.len()).unwrap_or(0);
}

pub(super) fn is_constraint_kind(kind: &str) -> bool {
    matches!(
        kind,
        "constraint" | "policy" | "rule" | "convention" | "contract" | "preference"
    )
}

pub(super) fn need_covered(need: &Need, cards: &[Card]) -> bool {
    match need {
        Need::Conflicts => cards.iter().any(|c| c.epistemic == "contested"),
        Need::Unverified => cards.iter().any(|c| c.epistemic != "asserted"),
        Need::Answer | Need::Map | Need::AsKnown | Need::Changes => !cards.is_empty(),
        Need::CurrentConstraints => cards.iter().any(|c| is_constraint_kind(&c.kind)),
        Need::FailedAttempts => cards
            .iter()
            .any(|c| matches!(c.kind.as_str(), "attempt" | "failure" | "outcome")),
        Need::Procedures => cards
            .iter()
            .any(|c| matches!(c.kind.as_str(), "procedure" | "case")),
        // Obligations / verified outcomes / compare / audit / recipes are
        // not recall Cards; they stay unmet until a recipe or continuation
        // supplies them (see operations contract: open_obligations named).
        _ => false,
    }
}

pub(super) fn legacy_record(
    conn: &Connection,
    reference: &str,
) -> rusqlite::Result<Option<String>> {
    let Some((kind, id)) = reference.split_once("::") else {
        return Ok(None);
    };
    let Ok(id) = id.parse::<i64>() else {
        return Ok(None);
    };
    crate::db::records::record_for_legacy(conn, kind, id)
}
