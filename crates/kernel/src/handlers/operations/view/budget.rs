use super::{Card, View, is_constraint_kind, need_covered};
use crate::lens::Need;
use crate::protocol::ResponseStatus;
use serde_json::json;

fn card_label_prefix(card: &Card) -> &'static str {
    if card.epistemic == "contested" {
        "x"
    } else if is_constraint_kind(&card.kind) {
        "c"
    } else if matches!(card.kind.as_str(), "attempt" | "failure" | "outcome") {
        "f"
    } else {
        "k"
    }
}

fn assign_aliases(delivered: &mut [Card]) {
    let mut counters = std::collections::BTreeMap::<&str, usize>::new();
    for (index, card) in delivered.iter_mut().enumerate() {
        card.alias = format!("m{}", index + 1);
        let prefix = card_label_prefix(card);
        let n = counters.entry(prefix).or_insert(0);
        *n += 1;
        card.label = format!("{prefix}{n}");
    }
}

fn coverage_status(view: &View) -> ResponseStatus {
    if view.cards.is_empty() {
        if view.leads.is_empty() && view.engine_leads == 0 {
            ResponseStatus::NoMatch
        } else {
            ResponseStatus::Ambiguous
        }
    } else if view.coverage.unmet.is_empty() {
        ResponseStatus::Ok
    } else {
        ResponseStatus::Partial
    }
}

impl View {
    /// Deterministic budgeted selection: required bundles first (all or
    /// `needs_more_budget` with the concrete plan size), then optional
    /// bundles by marginal coverage per byte with stable ties, never cutting
    /// inside a Card. Aliases are reassigned in delivery order; labels map to
    /// them in the sidecar.
    pub fn select_within_budget(&mut self) {
        let cost = |c: &Card| c.bytes + c.alias.len() + 4;
        let required_total: usize = self.cards.iter().filter(|c| c.required).map(cost).sum();
        if required_total > self.budget_bytes {
            self.status = ResponseStatus::NeedsMoreBudget;
            self.required_plan_bytes = Some(required_total);
            self.coverage.omissions = self
                .cards
                .iter()
                .map(
                    |c| json!({"reference": c.reference, "bytes": cost(c), "required": c.required}),
                )
                .collect();
            self.cards.clear();
            self.used_bytes = 0;
            self.refresh_need_coverage();
            return;
        }
        let mut remaining = self.budget_bytes.saturating_sub(required_total);
        let mut delivered: Vec<Card> = Vec::new();
        let mut omissions = Vec::new();
        // Optional bundles ranked by (new need families covered / cost), then
        // original rank, then canonical reference for a stable tie-break.
        let optional: Vec<(usize, Card)> = self
            .cards
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.required)
            .map(|(i, c)| (i, c.clone()))
            .collect();
        for card in self.cards.iter().filter(|c| c.required).cloned() {
            delivered.push(card);
        }
        let mut seen_kinds: std::collections::BTreeSet<String> =
            delivered.iter().map(|c| c.kind.clone()).collect();
        let mut ranked: Vec<(usize, Card)> = optional;
        ranked.sort_by(|(ia, a), (ib, b)| {
            let gain = |c: &Card| {
                if seen_kinds.contains(&c.kind) {
                    1u64
                } else {
                    2u64
                }
            };
            let ratio = |c: &Card| gain(c) * 1_000_000 / (cost(c) as u64).max(1);
            ratio(b)
                .cmp(&ratio(a))
                .then(ia.cmp(ib))
                .then(a.reference.cmp(&b.reference))
        });
        for (_, card) in ranked {
            let c = cost(&card);
            if c <= remaining {
                remaining -= c;
                seen_kinds.insert(card.kind.clone());
                delivered.push(card);
            } else {
                omissions.push(json!({"reference": card.reference, "bytes": c, "required": false, "reason": "budget"}));
            }
        }
        assign_aliases(&mut delivered);
        self.used_bytes = delivered.iter().map(|c| c.bytes).sum();
        // Nothing delivered although evidence exists: the budget cannot hold
        // even the smallest safe bundle. Say so with that bundle's size.
        if delivered.is_empty() && !omissions.is_empty() {
            let smallest = omissions
                .iter()
                .filter_map(|o| o["bytes"].as_u64())
                .min()
                .unwrap_or(0) as usize;
            self.status = ResponseStatus::NeedsMoreBudget;
            self.required_plan_bytes = Some(smallest);
            self.cards.clear();
            self.coverage.omissions = omissions;
            self.refresh_need_coverage();
            return;
        }
        self.cards = delivered;
        self.coverage.omissions = omissions;
        // Coverage is decided on what is actually delivered, with kinds known.
        self.refresh_need_coverage();
        self.status = coverage_status(self);
        self.required_plan_bytes = None;
    }

    fn refresh_need_coverage(&mut self) {
        self.coverage.covered = self
            .needs
            .iter()
            .filter(|n| need_covered(n, &self.cards))
            .map(Need::label)
            .collect();
        self.coverage.unmet = self
            .needs
            .iter()
            .filter(|n| !need_covered(n, &self.cards))
            .map(Need::label)
            .collect();
    }
}
