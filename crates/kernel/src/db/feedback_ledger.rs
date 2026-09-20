//! Feedback separation: exposure (a source was in a View), use (the agent
//! reported using it), success (the task outcome) and credit (use ∧ success)
//! are four different counts. Stats are policy-isolated per scope and task
//! family. The adaptive policy stays off until a held-out benefit exists and
//! can never lower authorization, erase, relabel as verified or withhold a
//! constraint.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const DDL: &str = "CREATE TABLE IF NOT EXISTS outcome_feedback (id INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL DEFAULT 'default', task_family TEXT NOT NULL DEFAULT 'general', task TEXT, prior_view_receipt TEXT, selected_action TEXT, outcome TEXT NOT NULL CHECK(outcome IN ('success','partial','failure')), exposed_json TEXT NOT NULL DEFAULT '[]', used_json TEXT NOT NULL DEFAULT '[]', harmful_reuse INTEGER NOT NULL DEFAULT 0, wrong_scope INTEGER NOT NULL DEFAULT 0, agent TEXT NOT NULL, recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));";

pub fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(DDL)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OutcomeFeedback {
    pub scope: String,
    pub task_family: String,
    pub task: Option<String>,
    pub prior_view_receipt: Option<String>,
    pub selected_action: Option<String>,
    pub outcome: String,
    pub exposed: Vec<String>,
    pub used: Vec<String>,
    pub harmful_reuse: bool,
    pub wrong_scope: bool,
    pub agent: String,
}

/// Exposed sources are read from the prior View receipt when the caller did
/// not list them: exposure is what the View contained, not what was used.
pub fn exposed_from_receipt(conn: &Connection, receipt_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT record_id FROM view_aliases WHERE receipt_id = ?1 ORDER BY alias")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([receipt_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn normalize_outcome(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "success" | "ok" | "pass" => Ok("success"),
        "partial" | "mixed" | "degraded" => Ok("partial"),
        "failure" | "fail" | "error" => Ok("failure"),
        _ => Err("outcome must be success|partial|failure".into()),
    }
}

pub fn record(conn: &Connection, fb: &OutcomeFeedback) -> Result<i64, String> {
    ensure(conn).map_err(|e| e.to_string())?;
    let outcome = normalize_outcome(&fb.outcome)?;
    let exposed_json = serde_json::to_string(&fb.exposed).map_err(|e| e.to_string())?;
    let used_json = serde_json::to_string(&fb.used).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO outcome_feedback (scope, task_family, task, prior_view_receipt, selected_action, outcome, exposed_json, used_json, harmful_reuse, wrong_scope, agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![if fb.scope.is_empty() { "default" } else { &fb.scope }, if fb.task_family.is_empty() { "general" } else { &fb.task_family }, fb.task, fb.prior_view_receipt, fb.selected_action, outcome, exposed_json, used_json, fb.harmful_reuse as i64, fb.wrong_scope as i64, fb.agent]).map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct SourceStats {
    pub exposure: u64,
    pub used: u64,
    pub success: u64,
    pub credit: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct FamilyStats {
    pub scope: String,
    pub task_family: String,
    pub outcomes: u64,
    pub successes: u64,
    pub liabilities: Liabilities,
    pub sources: BTreeMap<String, SourceStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Liabilities {
    pub harmful_reuse: u64,
    pub wrong_scope: u64,
}

/// Policy-isolated statistics for one (scope, task family). Nothing here
/// crosses scopes: a source's success elsewhere is not credit here.
pub fn family_stats(
    conn: &Connection,
    scope: &str,
    task_family: &str,
) -> Result<FamilyStats, String> {
    ensure(conn).map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT outcome, exposed_json, used_json, harmful_reuse, wrong_scope FROM outcome_feedback WHERE scope = ?1 AND task_family = ?2").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![scope, task_family], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut stats = FamilyStats {
        scope: scope.into(),
        task_family: task_family.into(),
        ..Default::default()
    };
    for (outcome, exposed, used, harmful, wrong) in rows {
        stats.outcomes += 1;
        let ok = outcome == "success";
        if ok {
            stats.successes += 1;
        }
        stats.liabilities.harmful_reuse += harmful as u64;
        stats.liabilities.wrong_scope += wrong as u64;
        let exposed: Vec<String> = serde_json::from_str(&exposed).unwrap_or_default();
        let used: Vec<String> = serde_json::from_str(&used).unwrap_or_default();
        for s in &exposed {
            stats.sources.entry(s.clone()).or_default().exposure += 1;
        }
        for s in &used {
            let e = stats.sources.entry(s.clone()).or_default();
            e.used += 1;
            if ok {
                e.success += 1;
                e.credit += 1;
            }
        }
    }
    Ok(stats)
}

/// Choices the deferred contextual bandit may ever pick between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeChoice {
    ViewSize,
    EvidenceFamily,
    CaseVsTimeline,
    AlternatePlan,
}

/// Actions the policy is forbidden to take under any reward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForbiddenAction {
    LowerAuthorization,
    EraseSource,
    RelabelAsVerified,
    WithholdConstraint,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AdaptivePolicy {
    pub enabled: bool,
    pub reason: String,
    pub held_out_benefit: Option<f64>,
    pub samples: u64,
}

pub const MIN_HELD_OUT_SAMPLES: u64 = 30;

/// The bandit is off until a held-out benefit (success rate with policy −
/// without, on held-out tasks) is positive over enough samples. Popularity
/// of retrieval is not benefit.
pub fn adaptive_policy(held_out_with: (u64, u64), held_out_without: (u64, u64)) -> AdaptivePolicy {
    let (sw, nw) = held_out_with;
    let (so, no) = held_out_without;
    let samples = nw.min(no);
    if samples < MIN_HELD_OUT_SAMPLES {
        return AdaptivePolicy {
            enabled: false,
            reason: format!("held-out samples {samples} < {MIN_HELD_OUT_SAMPLES}"),
            held_out_benefit: None,
            samples,
        };
    }
    let benefit = sw as f64 / nw.max(1) as f64 - so as f64 / no.max(1) as f64;
    if benefit <= 0.0 {
        return AdaptivePolicy {
            enabled: false,
            reason: "no held-out benefit".into(),
            held_out_benefit: Some(benefit),
            samples,
        };
    }
    AdaptivePolicy {
        enabled: true,
        reason: "held-out benefit shown".into(),
        held_out_benefit: Some(benefit),
        samples,
    }
}

/// Safety envelope: the only things a policy may vary are the safe choices.
pub fn envelope_check(proposed: &Value) -> Result<SafeChoice, Value> {
    let kind = proposed.get("choice").and_then(Value::as_str).unwrap_or("");
    if let Some(forbidden) = proposed.get("forbidden").and_then(Value::as_str) {
        return Err(
            json!({"status": "denied", "error": "outside the safety envelope", "action": forbidden}),
        );
    }
    match kind {
        "view_size" => Ok(SafeChoice::ViewSize),
        "evidence_family" => Ok(SafeChoice::EvidenceFamily),
        "case_vs_timeline" => Ok(SafeChoice::CaseVsTimeline),
        "alternate_plan" => Ok(SafeChoice::AlternatePlan),
        other => Err(
            json!({"status": "denied", "error": "outside the safety envelope", "action": other, "allowed": ["view_size","evidence_family","case_vs_timeline","alternate_plan"]}),
        ),
    }
}
