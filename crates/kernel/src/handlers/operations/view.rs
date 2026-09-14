//! View and Card: the answer shape every read operation returns.
//!
//! A Card carries a statement, epistemic state, applicability, freshness and
//! an expansion handle (alias bound to this View's receipt). Coverage maps
//! declared needs to delivered evidence; unmet needs are named, never
//! silently dropped. Leads (unsupported candidates) are separate from Cards.

use crate::lens::{Need, NeedFrame};
use crate::presence::{
    decide, ChangeCursor, CurrentEpochs, CursorError, PresenceDecision, CHANGE_RULE_VERSION,
};
use crate::protocol::{ContextPresence, LogicalId, ResponseStatus};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

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

impl View {
    /// Build a View from the unified recall payload. Required bundles are
    /// the admitted Cards in rank order; when they exceed the budget the View
    /// is `needs_more_budget` with the concrete safe plan size instead of a
    /// Card cut mid-condition.
    pub fn from_recall(frame: &NeedFrame, payload: &Value, budget_bytes: usize) -> Self {
        let empty = Vec::new();
        let results = payload["results"].as_array().unwrap_or(&empty);
        let mut cards = Vec::new();
        for (index, item) in results.iter().enumerate() {
            let statement = item["excerpt"].as_str().unwrap_or("").trim().to_string();
            if statement.is_empty() {
                continue;
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
            let kind = match reference.split_once("::") {
                Some(("memory", _)) => "memory",
                Some(("decision", _)) => "decision",
                _ => "decision",
            };
            cards.push(Card {
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
            });
        }
        // Budget selection happens after closure (`select_within_budget`),
        // when required exceptions are known; here every admitted Card is kept.
        // Engine leads (candidates not admitted as results) must not wait for
        // close_evidence: empty Cards with leftover material is `ambiguous`.
        let engine_leads = json_count(&payload["routes"]["leads"]);
        let (status, required_plan_bytes) = if cards.is_empty() {
            if engine_leads == 0 {
                (ResponseStatus::NoMatch, None)
            } else {
                (ResponseStatus::Ambiguous, None)
            }
        } else {
            (ResponseStatus::Ok, None)
        };
        let delivered = cards;
        let covered: Vec<String> = frame
            .needs
            .iter()
            .filter(|n| need_covered(n, &delivered))
            .map(Need::label)
            .collect();
        let unmet: Vec<String> = frame
            .needs
            .iter()
            .filter(|n| !need_covered(n, &delivered))
            .map(Need::label)
            .collect();
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

    /// Attach the evidence closure to each Card: required qualifiers and
    /// counterevidence become `exceptions`; an open CONTRADICTS conflict turns
    /// the Card into a contested contrast Card carrying the other side.
    pub fn close_evidence(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        use super::closure::{close_revision, DependencyRole};
        for card in &mut self.cards {
            let Some(record_id) = legacy_record(conn, &card.reference)? else {
                if card.reference.contains("::") {
                    mark_contested(
                        card,
                        "qualification_unavailable: no record mapping",
                    );
                }
                continue;
            };
            let heads = crate::db::records::heads(conn, &record_id)?;
            let Some(revision) = heads.first() else {
                mark_contested(card, "qualification_unavailable: no record head");
                continue;
            };
            let parsed = card.reference.split_once("::").and_then(|(k, id)| {
                id.parse::<i64>().ok().map(|n| (k.to_string(), n))
            });
            let legacy_id = parsed
                .as_ref()
                .filter(|(k, _)| k == "decision")
                .map(|(_, id)| *id);
            if let Some((ns, id)) = parsed.as_ref() {
                let sql = match ns.as_str() {
                    "decision" => {
                        "SELECT COALESCE(type,'decision'), COALESCE(retention_class,'operational') FROM decisions WHERE id = ?1"
                    }
                    "memory" => {
                        "SELECT COALESCE(type,'memory'), COALESCE(retention_class,'operational') FROM memories WHERE id = ?1"
                    }
                    _ => "",
                };
                if !sql.is_empty() {
                    match conn.query_row(sql, params![id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    }) {
                        Ok((kind, retention)) => {
                            card.kind = kind;
                            card.retention = retention;
                        }
                        Err(rusqlite::Error::QueryReturnedNoRows) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            let Ok((items, contrary)) = close_revision(conn, revision, legacy_id) else {
                // Fail closed: a claim whose exceptions could not be loaded
                // must not travel as an asserted Card.
                mark_contested(card, "qualification_unavailable: evidence closure failed");
                continue;
            };
            for item in items {
                match item.role {
                    DependencyRole::RequiredQualifier
                    | DependencyRole::ProcedurePrecondition
                    | DependencyRole::Policy => {
                        card.exceptions
                            .push(format!("{}: {}", item.role.as_str(), item.text))
                    }
                    DependencyRole::Counterevidence => card
                        .exceptions
                        .push(format!("counterevidence: {}", item.text)),
                    _ => {}
                }
            }
            if !contrary.is_empty() {
                card.epistemic = "contested";
                for c in &contrary {
                    card.exceptions.push(format!(
                        "contrary head {}: {}",
                        c["other"].as_str().unwrap_or("?"),
                        c["text"].as_str().unwrap_or("")
                    ));
                }
            }
            if heads.len() > 1 {
                card.epistemic = "contested";
                card.exceptions
                    .push(format!("{} concurrent heads unresolved", heads.len()));
            }
            // Cases and procedures travel with their nearest counterexample:
            // same preconditions (or same subject) that failed.
            if matches!(card.kind.as_str(), "case" | "procedure") {
                if let Ok(Some(body)) = crate::db::records::revision_body(conn, revision) {
                    let pre = body["preconditions"].clone();
                    let subject = body["subject"].clone();
                    if let Ok(mut stmt) = conn.prepare("SELECT r.record_id, v.body_json FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.kind = 'counterexample' ORDER BY r.record_id") {
                        let rows: Vec<(String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.flatten().collect()).unwrap_or_default();
                        for (id, raw) in rows {
                            let Ok(b) = serde_json::from_str::<Value>(&raw) else { continue };
                            let same_pre = !pre.is_null() && b["preconditions"] == pre;
                            let same_subject = !subject.is_null() && b["subject"] == subject;
                            if same_pre || same_subject {
                                card.exceptions.push(format!("counterexample {id}: {}", b["text"].as_str().unwrap_or("")));
                            }
                        }
                    }
                }
            }
            // Retention governs deletion, not delivery: a protected bundle is
            // a constraint-like kind or an unresolved contradiction.
            card.required = card.epistemic == "contested" || is_constraint_kind(&card.kind);
            card.bytes = card.statement.len()
                + card.exceptions.iter().map(|e| e.len() + 2).sum::<usize>()
                + card.exact_text.as_ref().map(|t| t.len()).unwrap_or(0);
        }
        self.select_within_budget();
        self.watermark(conn);
        Ok(())
    }

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
            let gain_a = if seen_kinds.contains(&a.kind) {
                1u64
            } else {
                2u64
            };
            let gain_b = if seen_kinds.contains(&b.kind) {
                1u64
            } else {
                2u64
            };
            let ratio_a = gain_a * 1_000_000 / (cost(a) as u64).max(1);
            let ratio_b = gain_b * 1_000_000 / (cost(b) as u64).max(1);
            ratio_b
                .cmp(&ratio_a)
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
        let mut counters = std::collections::BTreeMap::<&str, usize>::new();
        for (index, card) in delivered.iter_mut().enumerate() {
            card.alias = format!("m{}", index + 1);
            let prefix = if card.epistemic == "contested" {
                "x"
            } else if is_constraint_kind(&card.kind) {
                "c"
            } else if matches!(card.kind.as_str(), "attempt" | "failure" | "outcome") {
                "f"
            } else {
                "k"
            };
            let n = counters.entry(prefix).or_insert(0);
            *n += 1;
            card.label = format!("{prefix}{n}");
        }
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
        self.status = if self.cards.is_empty() {
            if self.leads.is_empty() && self.engine_leads == 0 {
                ResponseStatus::NoMatch
            } else {
                ResponseStatus::Ambiguous
            }
        } else if self.coverage.unmet.is_empty() {
            ResponseStatus::Ok
        } else {
            ResponseStatus::Partial
        };
        self.required_plan_bytes = None;
    }

    /// Coverage watermark per partition at this connection's frontier.
    pub fn watermark(&mut self, conn: &Connection) {
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let max_decision: i64 = conn
            .query_row("SELECT COALESCE(MAX(id),0) FROM decisions", [], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let max_memory: i64 = conn
            .query_row("SELECT COALESCE(MAX(id),0) FROM memories", [], |r| r.get(0))
            .unwrap_or(0);
        let archived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE status IN ('archived','superseded')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let cold_segments = crate::db::cold::cold_count(conn);
        let searched_cold = self.include_cold;
        let exhausted = self.coverage.partitions["exhausted"]
            .as_bool()
            .unwrap_or(true);
        let routes = self.coverage.partitions["routes"].clone();
        self.coverage.partitions = json!({
            "routes": routes,
            "decisions": {"source_frontier": seq, "index_frontier": seq, "searched_range": {"ids_through": max_decision}, "continuation": null, "limits_hit": !exhausted, "archive_policy": "active_only", "exhausted": exhausted},
            "memories": {"source_frontier": seq, "index_frontier": seq, "searched_range": {"ids_through": max_memory}, "continuation": null, "limits_hit": !exhausted, "archive_policy": "active_only", "exhausted": exhausted},
            "cold": {"searched": searched_cold, "rows_not_searched": if searched_cold { 0 } else { archived }, "cold_segments": cold_segments, "reason": if searched_cold { "cold partition included" } else { "archived/superseded rows are outside the active partition; ask with profile=history or time" }}
        });
    }

    /// Bind aliases to a receipt row scoped to the principal and brain epoch.
    pub fn persist_receipt(&mut self, conn: &Connection, principal: &str) -> rusqlite::Result<()> {
        crate::db::records::ensure_authoritative_schema(conn)?;
        let (_, restore_epoch, policy_epoch) = crate::db::records::brain_epochs(conn);
        self.apply_change_cursor(conn, principal, &restore_epoch);
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let random: String =
            conn.query_row("SELECT lower(hex(randomblob(4)))", [], |r| r.get(0))?;
        let receipt_id = format!("view-{seq}-{random}");
        self.frontier = Some(json!(frontier));
        let sp = crate::db::SqliteSavepoint::enter(conn, "view_receipt")?;
        conn.execute(
            "INSERT INTO view_receipts (receipt_id, principal_id, brain_epoch, through_sequence, receipt_json) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![receipt_id, principal, restore_epoch, seq, json!({"profile": self.profile, "cards": self.cards.len()}).to_string()],
        )?;
        for card in &mut self.cards {
            card.expandable = false;
            let Some(record_id) = legacy_record(conn, &card.reference)? else {
                continue;
            };
            let heads = crate::db::records::heads(conn, &record_id)?;
            let Some(revision) = heads.first() else {
                continue;
            };
            conn.execute(
                "INSERT OR REPLACE INTO view_aliases (receipt_id, alias, record_id, revision_id, representation_version) VALUES (?1, ?2, ?3, ?4, 'brief/1')",
                params![receipt_id, card.alias, record_id, revision],
            )?;
            card.expandable = true;
        }
        // Presence is a transport saving. Bind aliases first so a suppressed
        // Card in `present` can still be expanded by alias+receipt.
        self.apply_presence(conn, &restore_epoch, &policy_epoch)?;
        sp.release()?;
        self.receipt_id = receipt_id;
        Ok(())
    }

    /// Per-Card presence decision. A suppressed Card keeps its alias and
    /// revision in `present` so the agent can still refer to it; its payload
    /// is omitted from the transport only.
    fn apply_presence(
        &mut self,
        conn: &Connection,
        restore_epoch: &str,
        policy_epoch: &str,
    ) -> rusqlite::Result<()> {
        let Some(inputs) = self.presence.clone() else {
            return Ok(());
        };
        let current = CurrentEpochs {
            brain_epoch: restore_epoch.to_string(),
            policy_epoch: policy_epoch.to_string(),
        };
        let context_epoch = inputs.context_epoch.clone().unwrap_or_default();
        let mut i = 0;
        while i < self.cards.len() {
            let revision = legacy_record(conn, &self.cards[i].reference)?
                .and_then(|record| crate::db::records::heads(conn, &record).ok())
                .and_then(|h| h.first().cloned());
            let decision = match revision.as_ref() {
                Some(rev) => decide(
                    inputs.presence.as_ref(),
                    inputs.attested_brain.as_deref(),
                    inputs.attested_policy.as_deref(),
                    &current,
                    &context_epoch,
                    &LogicalId::new("revision", rev),
                    "brief/1",
                ),
                None => PresenceDecision::DeliverRepresentationDiffers,
            };
            if decision.suppresses() {
                let card = self.cards.remove(i);
                self.present
                    .push(json!({"alias": card.alias, "label": card.label, "revision": revision, "representation": "brief/1", "decision": decision}));
            } else {
                i += 1;
            }
        }
        self.used_bytes = self.cards.iter().map(|c| c.bytes).sum();
        Ok(())
    }

    /// Change cursor: changes since the cursor under the same restore epoch,
    /// scope/filter identity and rule version; otherwise `resnapshot_required`
    /// and the View stays self-contained.
    fn apply_change_cursor(&mut self, conn: &Connection, principal: &str, restore_epoch: &str) {
        let scope_filter = format!("{principal}:{}", self.profile);
        let frontier = crate::store_spi::sqlite::current_frontier(conn);
        let seq = crate::store_spi::sqlite::frontier_sequence(&frontier);
        let out = ChangeCursor {
            restore_epoch: restore_epoch.to_string(),
            scope_filter: scope_filter.clone(),
            sequence: seq,
            rule_version: CHANGE_RULE_VERSION.into(),
        };
        self.change_cursor_out = Some(out.encode());
        let Some(raw) = self.change_cursor_in.clone() else {
            return;
        };
        match ChangeCursor::decode(&raw).and_then(|c| c.validate(restore_epoch, &scope_filter)) {
            Ok(since) => {
                // A failed versions read is not "nothing changed": that would
                // hide durable work behind an Ok empty delta.
                match conn.prepare("SELECT v.id, v.target_type, v.target_id, v.op FROM versions v WHERE v.id > ?1 ORDER BY v.id LIMIT 200") {
                    Ok(mut stmt) => match stmt.query_map(params![since], |r| Ok(json!({"sequence": r.get::<_, i64>(0)?, "target": format!("{}::{}", r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0)), "action": r.get::<_, String>(3)?}))) {
                        Ok(rows) => match rows.collect::<Result<Vec<_>, _>>() {
                            Ok(changes) => {
                                self.changes = changes;
                                self.cursor_status = Some(ResponseStatus::Ok);
                            }
                            Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                        },
                        Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                    },
                    Err(_) => self.cursor_status = Some(ResponseStatus::Unavailable),
                }
            }
            Err(CursorError::Malformed) => {
                self.cursor_status = Some(ResponseStatus::InvalidRequest)
            }
            Err(CursorError::ResnapshotRequired { .. }) => {
                self.cursor_status = Some(ResponseStatus::ResnapshotRequired)
            }
        }
    }

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

fn json_count(value: &Value) -> usize {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .unwrap_or(0) as usize
}

fn mark_contested(card: &mut Card, reason: &str) {
    card.epistemic = "contested";
    card.exceptions.push(reason.into());
    card.required = true;
    card.bytes = card.statement.len()
        + card.exceptions.iter().map(|e| e.len() + 2).sum::<usize>()
        + card.exact_text.as_ref().map(|t| t.len()).unwrap_or(0);
}

fn is_constraint_kind(kind: &str) -> bool {
    matches!(
        kind,
        "constraint" | "policy" | "rule" | "convention" | "contract" | "preference"
    )
}

fn need_covered(need: &Need, cards: &[Card]) -> bool {
    match need {
        Need::Conflicts => cards.iter().any(|c| c.epistemic == "contested"),
        Need::Unverified => cards.iter().any(|c| c.epistemic != "asserted"),
        Need::Answer | Need::Map => !cards.is_empty(),
        Need::CurrentConstraints => cards.iter().any(|c| is_constraint_kind(&c.kind)),
        Need::FailedAttempts => cards.iter().any(|c| {
            matches!(c.kind.as_str(), "attempt" | "failure" | "outcome")
        }),
        Need::Procedures => {
            cards.iter().any(|c| matches!(c.kind.as_str(), "procedure" | "case"))
        },
        Need::AsKnown | Need::Changes => !cards.is_empty(),
        // Obligations / verified outcomes / compare / audit / recipes are
        // not recall Cards; they stay unmet until a recipe or continuation
        // supplies them (see operations contract: open_obligations named).
        _ => false,
    }
}

fn legacy_record(conn: &Connection, reference: &str) -> rusqlite::Result<Option<String>> {
    let Some((kind, id)) = reference.split_once("::") else {
        return Ok(None);
    };
    let Ok(id) = id.parse::<i64>() else {
        return Ok(None);
    };
    crate::db::records::record_for_legacy(conn, kind, id)
}
