//! Type-specific promotion rules. There is no global confidence threshold:
//! each rule names its source authority, its evidence population, its
//! exclusions (counterexamples) and its revocation triggers, and records
//! them in the promoted revision so the scope of the claim stays inspectable.

use super::records::{append_revision, heads, revision_body, NewRevision, DEFAULT_SCOPE};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const PROMOTION_RULES_VERSION: &str = "promotion/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionRule {
    /// One authenticated user statement is enough for a personal preference.
    Preference,
    /// One checker predicate on one artifact establishes one verified result.
    CheckerResult,
    /// A reusable procedure needs matching preconditions across genuinely
    /// independent successful cases; copies are not trials.
    Procedure,
    /// A cross-project lesson needs explicit scope widening, authority and a
    /// disconfirming-case check in the target scope.
    CrossProjectLesson,
}

impl PromotionRule {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw.trim().to_ascii_lowercase().as_str() {
            "preference" => Self::Preference,
            "checker_result" | "checker" => Self::CheckerResult,
            "procedure" => Self::Procedure,
            "cross_project_lesson" | "lesson" => Self::CrossProjectLesson,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::CheckerResult => "checker_result",
            Self::Procedure => "procedure",
            Self::CrossProjectLesson => "cross_project_lesson",
        }
    }
}

fn head_body(conn: &Connection, record_id: &str) -> Result<Value, String> {
    let head = heads(conn, record_id)
        .map_err(|e| e.to_string())?
        .first()
        .cloned()
        .ok_or_else(|| format!("{record_id} has no head"))?;
    revision_body(conn, &head)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{head} has no body"))
}

fn record_kind(conn: &Connection, record_id: &str) -> Result<String, String> {
    conn.query_row(
        "SELECT kind FROM records WHERE record_id = ?1",
        params![record_id],
        |r| r.get(0),
    )
    .map_err(|_| format!("unknown record {record_id}"))
}

/// Records of one kind whose `preconditions` field equals `preconditions`.
fn matching(
    conn: &Connection,
    kind: &str,
    preconditions: &Value,
) -> Result<Vec<(String, Value)>, String> {
    let mut stmt = conn.prepare("SELECT r.record_id, v.body_json FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.kind = ?1 ORDER BY r.record_id").map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![kind], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    Ok(rows
        .into_iter()
        .filter_map(|(id, body)| serde_json::from_str::<Value>(&body).ok().map(|b| (id, b)))
        .filter(|(_, b)| &b["preconditions"] == preconditions)
        .collect())
}

pub struct Promotion<'a> {
    pub rule: PromotionRule,
    pub principal: &'a str,
    pub agent: &'a str,
    pub authority: Option<&'a str>,
    pub sources: Vec<String>,
    pub text: &'a str,
    pub preconditions: Value,
    pub target_scope: Option<&'a str>,
}

pub fn promote(
    conn: &Connection,
    sequence: i64,
    promotion: Promotion<'_>,
) -> Result<(String, Value), String> {
    let (kind, population, exclusions, authority, revocation): (
        &str,
        Vec<String>,
        Vec<String>,
        String,
        Vec<&str>,
    ) = match promotion.rule {
        PromotionRule::Preference => {
            if !promotion.principal.starts_with("user:") && promotion.principal != "solo" {
                return Err(
                    "a preference needs an authenticated user statement, not an agent assertion"
                        .into(),
                );
            }
            (
                "preference",
                vec![format!("statement by {}", promotion.principal)],
                Vec::new(),
                promotion.principal.to_string(),
                vec!["user restates or withdraws the preference"],
            )
        }
        PromotionRule::CheckerResult => {
            let predicate = promotion.preconditions["predicate"].as_str().unwrap_or("");
            let artifact = promotion.preconditions["artifact"].as_str().unwrap_or("");
            let checker = promotion.preconditions["checker"].as_str().unwrap_or("");
            if predicate.is_empty() || artifact.is_empty() || checker.is_empty() {
                return Err("a checker result names predicate, artifact and checker".into());
            }
            if promotion.preconditions["passed"].as_bool() != Some(true) {
                return Err("a failing or unstated checker run establishes nothing".into());
            }
            (
                "verified_result",
                vec![format!("{checker}:{predicate}@{artifact}")],
                Vec::new(),
                checker.to_string(),
                vec!["artifact revision changes", "checker version changes"],
            )
        }
        PromotionRule::Procedure => {
            if promotion.preconditions.is_null() || promotion.preconditions == json!({}) {
                return Err("a procedure needs explicit preconditions".into());
            }
            let cases = matching(conn, "case", &promotion.preconditions)?;
            let successes: Vec<&(String, Value)> = cases
                .iter()
                .filter(|(_, b)| b["outcome"].as_str() == Some("success"))
                .collect();
            // Independence: distinct origin agents AND distinct observation
            // texts; ten copies of one log are one trial.
            let origins: BTreeSet<String> = successes
                .iter()
                .map(|(_, b)| {
                    format!(
                        "{}|{}",
                        b["agent"].as_str().unwrap_or("?"),
                        cortex_logic::traces::content_hash(b["text"].as_str().unwrap_or(""))
                    )
                })
                .collect();
            let distinct_agents: BTreeSet<&str> = successes
                .iter()
                .filter_map(|(_, b)| b["agent"].as_str())
                .collect();
            if successes.len() < 2 || origins.len() < 2 || distinct_agents.len() < 2 {
                return Err(format!("a procedure needs at least two independent successful cases with matching preconditions; found {} cases, {} independent origins, {} agents", successes.len(), origins.len(), distinct_agents.len()));
            }
            let counterexamples: Vec<String> =
                matching(conn, "counterexample", &promotion.preconditions)?
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
            (
                "procedure",
                successes.iter().map(|(id, _)| id.clone()).collect(),
                counterexamples,
                promotion.authority.unwrap_or(promotion.agent).to_string(),
                vec![
                    "a counterexample with matching preconditions",
                    "environment fingerprint outside the population",
                ],
            )
        }
        PromotionRule::CrossProjectLesson => {
            let Some(target) = promotion.target_scope else {
                return Err("a cross-project lesson needs an explicit target scope (widening is never implicit)".into());
            };
            let Some(authority) = promotion.authority else {
                return Err("a cross-project lesson needs an authority".into());
            };
            if promotion.sources.is_empty() {
                return Err("a cross-project lesson names its source cases".into());
            }
            for source in &promotion.sources {
                record_kind(conn, source)?;
            }
            // Disconfirming check in the target scope: any counterexample
            // whose `scope` names the target blocks the promotion.
            let mut stmt = conn.prepare("SELECT r.record_id, v.body_json FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.kind = 'counterexample'").map_err(|e| e.to_string())?;
            let disconfirming: Vec<String> = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?
                .flatten()
                .filter(|(_, body)| {
                    serde_json::from_str::<Value>(body)
                        .ok()
                        .map(|b| b["scope"].as_str() == Some(target))
                        .unwrap_or(false)
                })
                .map(|(id, _)| id)
                .collect();
            if !disconfirming.is_empty() {
                return Err(format!(
                    "disconfirming cases exist in {target}: {}",
                    disconfirming.join(", ")
                ));
            }
            (
                "lesson",
                promotion.sources.clone(),
                Vec::new(),
                authority.to_string(),
                vec![
                    "a counterexample in the target scope",
                    "authority withdrawn",
                ],
            )
        }
    };
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM records WHERE kind = ?1",
            params![kind],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let record_id = format!("{kind}:{}", count + 1);
    let body = json!({
        "text": promotion.text,
        "rule": promotion.rule.as_str(),
        "rule_version": PROMOTION_RULES_VERSION,
        "authority": authority,
        "population": population,
        "exclusions": exclusions,
        "preconditions": promotion.preconditions,
        "scope": promotion.target_scope.unwrap_or(DEFAULT_SCOPE),
        "revocation_triggers": revocation,
        "promoted_by": promotion.agent,
    });
    let epistemic = if kind == "verified_result" {
        "checker_verified"
    } else {
        "supported"
    };
    append_revision(
        conn,
        sequence,
        NewRevision {
            record_id: &record_id,
            kind,
            retention: "durable",
            body: body.clone(),
            epistemic_status: epistemic,
            parents: &[],
            replace_parents: true,
            representation_version: "promotion/1",
        },
    )
    .map_err(|e| e.to_string())?;
    for source in &population {
        if record_kind(conn, source).is_ok() {
            let from = heads(conn, &record_id)
                .map_err(|e| e.to_string())?
                .remove(0);
            let to = heads(conn, source)
                .map_err(|e| e.to_string())?
                .first()
                .cloned();
            if let Some(to) = to {
                let _ = crate::handlers::operations::add_relation(
                    conn,
                    sequence,
                    &from,
                    &to,
                    "support",
                    Some(PROMOTION_RULES_VERSION),
                );
            }
        }
    }
    for exclusion in &exclusions {
        let from = heads(conn, &record_id)
            .map_err(|e| e.to_string())?
            .remove(0);
        if let Some(to) = heads(conn, exclusion)
            .map_err(|e| e.to_string())?
            .first()
            .cloned()
        {
            let _ = crate::handlers::operations::add_relation(
                conn,
                sequence,
                &from,
                &to,
                "counterevidence",
                Some(PROMOTION_RULES_VERSION),
            );
        }
    }
    let _ = head_body;
    Ok((record_id, body))
}
