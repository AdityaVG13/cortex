//! Operator-owned `promote-policy/1`. Missing policy keeps builtin defaults.
//! A stored document that is not that schema, or that has the wrong shape,
//! fails closed.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

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

pub const SCHEMA: &str = "promote-policy/1";
pub const DDL: &str = "CREATE TABLE IF NOT EXISTS promotion_policy (singleton INTEGER PRIMARY KEY CHECK (singleton = 1), document TEXT NOT NULL, updated_at TEXT NOT NULL);";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePolicy {
    pub principals: Vec<String>,
    pub revocation: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiteRoles {
    AllExceptDeliveryOnly,
    Allow(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicy {
    preference: RulePolicy,
    checker_result: RulePolicy,
    procedure: RulePolicy,
    lesson: RulePolicy,
    cite_roles: CiteRoles,
}

impl ResolvedPolicy {
    pub fn defaults() -> Self {
        Self {
            preference: default_rule_policy(PromotionRule::Preference),
            checker_result: default_rule_policy(PromotionRule::CheckerResult),
            procedure: default_rule_policy(PromotionRule::Procedure),
            lesson: default_rule_policy(PromotionRule::CrossProjectLesson),
            cite_roles: CiteRoles::AllExceptDeliveryOnly,
        }
    }

    pub fn rule(&self, rule: &PromotionRule) -> &RulePolicy {
        match rule {
            PromotionRule::Preference => &self.preference,
            PromotionRule::CheckerResult => &self.checker_result,
            PromotionRule::Procedure => &self.procedure,
            PromotionRule::CrossProjectLesson => &self.lesson,
        }
    }

    fn rule_mut(&mut self, rule: &PromotionRule) -> &mut RulePolicy {
        match rule {
            PromotionRule::Preference => &mut self.preference,
            PromotionRule::CheckerResult => &mut self.checker_result,
            PromotionRule::Procedure => &mut self.procedure,
            PromotionRule::CrossProjectLesson => &mut self.lesson,
        }
    }

    pub fn allows_principal(&self, rule: &PromotionRule, principal: &str) -> bool {
        principal_allowed(principal, &self.rule(rule).principals)
    }

    pub fn cite_allowed(&self, role: &str) -> bool {
        if role == "delivery_only" || !citeable_role(role) {
            return false;
        }
        match &self.cite_roles {
            CiteRoles::AllExceptDeliveryOnly => true,
            CiteRoles::Allow(roles) => roles.iter().any(|allowed| allowed == role),
        }
    }
}

pub fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

pub fn set(conn: &Connection, document: &Value) -> Result<(), String> {
    ensure(conn)?;
    parse_document(document)?;
    let encoded = serde_json::to_string(document).map_err(|e| e.to_string())?;
    conn.execute("INSERT OR REPLACE INTO promotion_policy (singleton, document, updated_at) VALUES (1, ?1, strftime('%Y-%m-%dT%H:%M:%fZ','now'))", params![encoded]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load(conn: &Connection) -> Result<Option<Value>, String> {
    ensure(conn)?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT document FROM promotion_policy WHERE singleton = 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    raw.map(|text| serde_json::from_str(&text).map_err(|e| e.to_string()))
        .transpose()
}

pub fn resolve(conn: &Connection) -> Result<ResolvedPolicy, String> {
    match load(conn)? {
        None => Ok(ResolvedPolicy::defaults()),
        Some(document) => parse_document(&document),
    }
}

pub fn principal_denied(rule: &PromotionRule, principal: &str, policy: &ResolvedPolicy) -> String {
    let allowed = &policy.rule(rule).principals;
    if matches!(rule, PromotionRule::Preference) && allowed == &default_principals(rule) {
        "a preference needs an authenticated user statement, not an agent assertion".into()
    } else {
        format!("principal {} may not promote {}", principal, rule.as_str())
    }
}

fn default_rule_policy(rule: PromotionRule) -> RulePolicy {
    RulePolicy {
        principals: default_principals(&rule),
        revocation: default_revocation(&rule),
    }
}

fn default_principals(rule: &PromotionRule) -> Vec<String> {
    match rule {
        PromotionRule::Preference => vec!["solo".into(), "user:*".into()],
        _ => vec!["*".into()],
    }
}

fn default_revocation(rule: &PromotionRule) -> Vec<String> {
    match rule {
        PromotionRule::Preference => vec!["user restates or withdraws the preference".into()],
        PromotionRule::CheckerResult => vec![
            "artifact revision changes".into(),
            "checker version changes".into(),
        ],
        PromotionRule::Procedure => vec![
            "a counterexample with matching preconditions".into(),
            "environment fingerprint outside the population".into(),
        ],
        PromotionRule::CrossProjectLesson => vec![
            "a counterexample in the target scope".into(),
            "authority withdrawn".into(),
        ],
    }
}

fn principal_allowed(principal: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        pattern == "*"
            || pattern
                .strip_suffix('*')
                .is_some_and(|prefix| principal.starts_with(prefix))
            || pattern == principal
    })
}

fn require_object<'a>(value: &'a Value, msg: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| msg.into())
}

fn parse_document(document: &Value) -> Result<ResolvedPolicy, String> {
    let obj = require_object(document, "promote policy must be an object")?;
    reject_unknown_keys(obj, &["schema", "rules", "commit_evidence"], "")?;
    if obj.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err("promote policy schema must be promote-policy/1".into());
    }
    let mut parsed = ResolvedPolicy::defaults();
    if let Some(rules) = document.get("rules") {
        let obj = require_object(rules, "promote policy rules must be an object")?;
        let mut seen = BTreeSet::new();
        for (name, body) in obj {
            let Some(rule) = PromotionRule::parse(name) else {
                return Err(format!("promote policy unknown rule `{name}`"));
            };
            if !seen.insert(rule.as_str()) {
                return Err(format!(
                    "promote policy rules.{name} duplicates {}",
                    rule.as_str()
                ));
            }
            let body = require_object(
                body,
                &format!("promote policy rules.{name} must be an object"),
            )?;
            reject_unknown_keys(
                body,
                &["principals", "revocation"],
                &format!("rules.{name}"),
            )?;
            let overlay = parsed.rule_mut(&rule);
            if let Some(principals) = body.get("principals") {
                overlay.principals = string_list(principals, &format!("rules.{name}.principals"))?;
            }
            if let Some(revocation) = body.get("revocation") {
                overlay.revocation = string_list(revocation, &format!("rules.{name}.revocation"))?;
            }
        }
    }
    if let Some(evidence) = document.get("commit_evidence") {
        let obj = require_object(evidence, "promote policy commit_evidence must be an object")?;
        reject_unknown_keys(obj, &["source_roles"], "commit_evidence")?;
        if let Some(roles) = obj.get("source_roles") {
            parsed.cite_roles = CiteRoles::Allow(
                string_list(roles, "commit_evidence.source_roles")?
                    .into_iter()
                    .map(|role| parse_citeable_role(&role))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    Ok(parsed)
}

fn reject_unknown_keys(
    obj: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "promote policy{} has unknown field `{key}`",
                if label.is_empty() {
                    String::new()
                } else {
                    format!(" {label}")
                }
            ));
        }
    }
    Ok(())
}

fn citeable_role(role: &str) -> bool {
    matches!(
        role,
        "document" | "user_statement" | "agent_assertion" | "tool_report"
    )
}

fn parse_citeable_role(role: &str) -> Result<String, String> {
    let role = role.trim();
    if role == "delivery_only" {
        return Err("commit_evidence.source_roles cannot include delivery_only".into());
    }
    citeable_role(role)
        .then(|| role.to_string())
        .ok_or_else(|| format!("commit_evidence.source_roles unknown role `{role}`"))
}

fn string_list(value: &Value, field: &str) -> Result<Vec<String>, String> {
    let Some(items) = value.as_array() else {
        return Err(format!("{field} must be an array of strings"));
    };
    items
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{field} must be an array of strings"))
        })
        .collect()
}
