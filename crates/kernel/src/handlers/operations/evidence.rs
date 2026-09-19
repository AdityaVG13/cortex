//! Cited observations on a Deposit. Promotion is explicit: capture never
//! becomes a fact by itself, and an unknown/unauthorized cite fails closed.

use crate::protocol::ResponseStatus;
use crate::runtime::observation::{self, ObservationRole};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

pub(crate) fn observation_evidence_ids(args: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["evidence", "from_observations", "observation_evidence"] {
        if let Some(Value::Array(items)) = args.get(key) {
            for item in items {
                let raw = item.as_str().unwrap_or_default().trim();
                if raw.is_empty() {
                    continue;
                }
                let id = raw
                    .strip_prefix("obs:")
                    .or_else(|| raw.strip_prefix("observation::"))
                    .unwrap_or(raw);
                if !id.is_empty() {
                    ids.push(id.to_string());
                }
            }
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

struct LinkedCite {
    source_id: String,
    source_key: String,
    role: String,
}

pub(crate) struct AuthorizedCites {
    cites: Vec<LinkedCite>,
}

impl AuthorizedCites {
    pub(crate) fn authorize(
        conn: &rusqlite::Connection,
        principal: &str,
        entry_paths: &[Vec<String>],
        source_ids: &[String],
    ) -> Result<Self, Value> {
        if source_ids.is_empty() {
            return Ok(Self { cites: Vec::new() });
        }
        let policy =
            crate::db::promotion::resolve(conn).map_err(|e| super::invalid_field(e, "evidence"))?;
        let mut cites = Vec::new();
        for source_id in source_ids {
            let row: Option<(String, String, String)> = conn.query_row(&format!("SELECT e.source_key, g.role, g.scope_label FROM {} JOIN sources s ON s.source_id=e.source_id WHERE e.principal=?1 AND e.source_id=?2 AND g.enabled=1 AND g.role!='delivery_only' AND g.policy_epoch=({}) AND s.availability='owned_inline' AND {}", observation::EVENT_GRANT_JOIN, crate::db::records::POLICY_EPOCH_SELECT, observation::LIVE_HEAD_SQL), rusqlite::params![principal, source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(|e| super::unavailable(e))?;
            let Some((source_key, role, scope_label)) = row else {
                return Err(cite_reject(
                    source_id,
                    format!(
                        "observation `{source_id}` is not authorized, disabled, retracted, stale, unavailable, or missing"
                    ),
                ));
            };
            if !ObservationRole::parse(&role)
                .is_some_and(|parsed| policy.cite_allowed(parsed.as_str()))
            {
                return Err(cite_reject(
                    source_id,
                    format!(
                        "observation `{source_id}` role `{role}` is not allowed by promote policy"
                    ),
                ));
            }
            if !entry_paths
                .iter()
                .all(|paths| observation::cite_scope_allowed(paths, &scope_label))
            {
                return Err(cite_reject(
                    source_id,
                    format!("observation `{source_id}` is not in this commit's path scope"),
                ));
            }
            cites.push(LinkedCite {
                source_id: source_id.clone(),
                source_key,
                role,
            });
        }
        Ok(Self { cites })
    }

    pub(crate) fn insert(
        &self,
        conn: &rusqlite::Connection,
        principal: &str,
        decision_ids: &[i64],
    ) -> Result<Vec<Value>, Value> {
        if self.cites.is_empty() {
            return Ok(Vec::new());
        }
        if decision_ids.is_empty() {
            return Err(super::invalid_field(
                "commit did not produce a decision to attach evidence",
                "evidence",
            ));
        }
        let now = crate::handlers::now_iso();
        for cite in &self.cites {
            for decision_id in decision_ids {
                conn.execute("INSERT OR IGNORE INTO decision_observation_evidence (decision_id,source_id,principal,source_key,role,relationship,created_at) VALUES(?1,?2,?3,?4,?5,'promoted_from',?6)", rusqlite::params![decision_id, cite.source_id, principal, cite.source_key, cite.role, now]).map_err(|e| super::unavailable(e))?;
                let attached: i64 = conn.query_row("SELECT COUNT(*) FROM decision_observation_evidence WHERE decision_id=?1 AND source_id=?2", rusqlite::params![decision_id, cite.source_id], |r| r.get(0)).map_err(|e| super::unavailable(e))?;
                if attached < 1 {
                    return Err(
                        json!({"status":ResponseStatus::Unavailable.as_str(),"error":format!("observation `{}` could not be attached to decision {decision_id}", cite.source_id),"field":"evidence","source_id":cite.source_id}),
                    );
                }
            }
        }
        Ok(self.as_response())
    }

    fn as_response(&self) -> Vec<Value> {
        self.cites.iter().map(|cite| json!({"source_id": cite.source_id, "source_key": cite.source_key, "role": cite.role, "relationship": "promoted_from", "expand": format!("obs:{}", cite.source_id)})).collect()
    }
}

fn cite_reject(source_id: &str, error: impl Into<String>) -> Value {
    let mut out = super::invalid_field(error, "evidence");
    out["source_id"] = json!(source_id);
    out
}

pub(crate) fn decision_id_from_outcome(outcome: &crate::runtime::DepositOutcome) -> Option<i64> {
    if let Some(id) = outcome
        .target_id
        .or_else(|| outcome.entry.get("id").and_then(Value::as_i64))
    {
        return Some(id);
    }
    if outcome.entry.get("action").and_then(Value::as_str) == Some("merged") {
        return outcome.entry.get("target_id").and_then(Value::as_i64);
    }
    None
}

pub(crate) fn observations_for_decision(
    conn: &rusqlite::Connection,
    decision_id: i64,
) -> Vec<Value> {
    let Ok(mut stmt) = conn.prepare("SELECT l.source_id,l.source_key,l.role,l.relationship, COALESCE(json_extract(r.body_json, '$.observation.text'), '') FROM decision_observation_evidence l JOIN observation_events e ON e.source_id=l.source_id AND e.principal=l.principal JOIN revisions r ON r.revision_id=e.revision_id WHERE l.decision_id=?1 ORDER BY l.source_id") else {
        return Vec::new();
    };
    let rows = stmt
        .query_map([decision_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| ())
        .into_iter()
        .flatten()
        .flatten()
        .collect::<Vec<_>>();
    rows.into_iter().map(|(source_id, source_key, role, relationship, text)| json!({"source_id": source_id, "source_key": source_key, "role": role, "relationship": relationship, "text": text, "expand": format!("obs:{source_id}"), "trust": {"kind": "attributed_observation", "instruction": false, "privilege": "none", "provenance": source_key}})).collect()
}
