use super::{Card, Coverage, View, coverage_labels, json_count};
use crate::lens::NeedFrame;
use crate::protocol::ResponseStatus;
use serde_json::{Value, json};

fn epistemic_for(item: &Value) -> &'static str {
    match item["status"].as_str() {
        Some("archived") | Some("superseded") => "retracted",
        Some("disputed") | Some("open") => "contested",
        _ => "asserted",
    }
}

fn applicability_for(item: &Value) -> &'static str {
    if item["why"]["filters"]["validAt"]
        .as_str()
        .map(|v| v != "current")
        .unwrap_or(false)
    {
        return "outside_validity";
    }
    match item["status"].as_str() {
        Some("archived") | Some("superseded") => "out_of_scope",
        _ => "applicable",
    }
}

fn card_from_recall_item(index: usize, item: &Value) -> Option<Card> {
    let statement = item["excerpt"].as_str().unwrap_or("").trim().to_string();
    if statement.is_empty() {
        return None;
    }
    let arms = item["why"]["clockVotes"]["admittedArms"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let reference = item["source"].as_str().unwrap_or("").to_string();
    // Namespace default only. `close_evidence` replaces this with the
    // row's `type` (constraint, failure, …); leaving memories as
    // "decision" made CurrentConstraints / FailedAttempts look uncovered
    // even when the recalled row was that kind.
    let kind = if reference.starts_with("memory::") {
        "memory"
    } else {
        "decision"
    };
    Some(Card {
        alias: format!("m{}", index + 1),
        label: String::new(),
        kind: kind.into(),
        retention: "operational".into(),
        required: false,
        expandable: false,
        exact_text: None,
        // Free-form source/context without `::` cannot be expanded;
        // it is still an admitted claim, not a hidden lead.
        reference,
        bytes: statement.len(),
        statement,
        epistemic: epistemic_for(item),
        applicability: applicability_for(item),
        exceptions: Vec::new(),
        freshness: item["validFrom"]
            .as_str()
            .or(item["created_at"].as_str())
            .map(str::to_string),
        admitted_by: item["why"]["admittedBy"].as_str().map(str::to_string),
        arms,
    })
}

impl View {
    /// Build a View from the unified recall payload. Required bundles are
    /// the admitted Cards in rank order; when they exceed the budget the View
    /// is `needs_more_budget` with the concrete safe plan size instead of a
    /// Card cut mid-condition.
    pub fn from_recall(frame: &NeedFrame, payload: &Value, budget_bytes: usize) -> Self {
        let empty = Vec::new();
        let results = payload["results"].as_array().unwrap_or(&empty);
        let cards: Vec<Card> = results
            .iter()
            .enumerate()
            .filter_map(|(index, item)| card_from_recall_item(index, item))
            .collect();
        // Budget selection happens after closure (`select_within_budget`),
        // when required exceptions are known; here every admitted Card is kept.
        // Engine leads (candidates not admitted as results) must not wait for
        // close_evidence: empty Cards with leftover material is `ambiguous`.
        let engine_leads = json_count(&payload["routes"]["leads"]);
        let (status, required_plan_bytes) = match (cards.is_empty(), engine_leads == 0) {
            (true, true) => (ResponseStatus::NoMatch, None),
            (true, false) => (ResponseStatus::Ambiguous, None),
            (false, _) => (ResponseStatus::Ok, None),
        };
        let delivered = cards;
        let (covered, unmet) = coverage_labels(&frame.needs, &delivered);
        let status = if status == ResponseStatus::Ok && !unmet.is_empty() {
            ResponseStatus::Partial
        } else {
            status
        };
        let used_bytes = delivered.iter().map(|c| c.bytes).sum();
        Self {
            status,
            include_cold: matches!(
                frame.profile,
                crate::lens::LensProfile::History | crate::lens::LensProfile::Audit
            ),
            needs: frame.needs.clone(),
            presence: None,
            present: Vec::new(),
            change_cursor_in: None,
            change_cursor_out: None,
            changes: Vec::new(),
            cursor_status: None,
            profile: frame.profile.as_str().to_string(),
            cards: delivered,
            leads: Vec::new(),
            engine_leads,
            coverage: Coverage {
                covered,
                unmet,
                partitions: json!({"searched": ["decisions", "memories"], "exhausted": payload["routes"]["exhausted"].as_array().map(|e| e.is_empty()).unwrap_or(true), "routes": payload["routes"].clone(), "cold": "not_searched"}),
                omissions: Vec::new(),
            },
            budget_bytes,
            used_bytes,
            receipt_id: String::new(),
            frontier: None,
            unresolved: frame.unresolved.clone(),
            required_plan_bytes,
        }
    }
}
