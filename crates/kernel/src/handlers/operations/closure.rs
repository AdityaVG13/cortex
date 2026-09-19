//! Dependency roles and evidence closure.
//!
//! Every relation between revisions carries a role with explicit invalidation
//! and rendering rules. Unknown roles are rejected at write time, never
//! treated as optional. Closure attaches, to each selected claim, the
//! qualifiers, counterevidence and unresolved contrary heads that change its
//! meaning; a claim is never served without its material exception.

use rusqlite::{Connection, params};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyRole {
    RequiredQualifier,
    Support,
    Counterevidence,
    ProcedurePrecondition,
    ExternalState,
    Policy,
    NegativePredicateRange,
    OptionalContext,
}

impl DependencyRole {
    pub const ALL: [DependencyRole; 8] = [
        Self::RequiredQualifier,
        Self::Support,
        Self::Counterevidence,
        Self::ProcedurePrecondition,
        Self::ExternalState,
        Self::Policy,
        Self::NegativePredicateRange,
        Self::OptionalContext,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequiredQualifier => "required_qualifier",
            Self::Support => "support",
            Self::Counterevidence => "counterevidence",
            Self::ProcedurePrecondition => "procedure_precondition",
            Self::ExternalState => "external_state",
            Self::Policy => "policy",
            Self::NegativePredicateRange => "negative_predicate_range",
            Self::OptionalContext => "optional_context",
        }
    }
    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.as_str() == raw.trim().to_ascii_lowercase())
    }
    /// Must travel with the claim: dropping it changes the claim's meaning.
    pub fn is_required(self) -> bool {
        matches!(
            self,
            Self::RequiredQualifier
                | Self::Counterevidence
                | Self::ProcedurePrecondition
                | Self::Policy
        )
    }
    /// Invalidates dependents when the target changes.
    pub fn invalidates_on_change(self) -> bool {
        !matches!(self, Self::OptionalContext)
    }
}

/// Record a typed relation from one revision to another. Unknown roles fail.
pub fn add_relation(
    conn: &Connection,
    sequence: i64,
    from_revision: &str,
    to_revision: &str,
    role: &str,
    rule_version: Option<&str>,
) -> Result<String, String> {
    let parsed = DependencyRole::parse(role).ok_or_else(|| {
        format!(
            "unknown dependency role `{role}`; known: {}",
            DependencyRole::ALL
                .iter()
                .map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let relation_id = format!("rel:{from_revision}->{to_revision}:{}", parsed.as_str());
    conn.execute("INSERT OR REPLACE INTO relations (relation_id, from_revision, to_revision, scope_id, relation_kind, rule_version, recorded_sequence) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![relation_id, from_revision, to_revision, crate::db::records::DEFAULT_SCOPE, parsed.as_str(), rule_version, sequence]).map_err(|e| e.to_string())?;
    Ok(relation_id)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureItem {
    pub role: DependencyRole,
    pub revision: String,
    pub text: String,
}

/// Closure of a revision: every required-role dependency with its text, plus
/// open legacy conflicts for the legacy decision behind it.
pub fn close_revision(
    conn: &Connection,
    revision_id: &str,
    legacy_decision_id: Option<i64>,
) -> Result<(Vec<ClosureItem>, Vec<Value>), String> {
    let mut items = Vec::new();
    let mut stmt = conn.prepare("SELECT relation_kind, to_revision FROM relations WHERE from_revision = ?1 ORDER BY relation_kind, to_revision").map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![revision_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for (kind, to_revision) in rows {
        let Some(role) = DependencyRole::parse(&kind) else {
            return Err(format!(
                "unknown dependency role `{kind}` on {revision_id}; a claim is not served without its exceptions"
            ));
        };
        if !role.is_required() && role != DependencyRole::Support {
            continue;
        }
        let body =
            crate::db::records::revision_body(conn, &to_revision).map_err(|e| e.to_string())?;
        let Some(body) = body else {
            if role.is_required() {
                return Err(format!(
                    "required {role} target `{to_revision}` has no body",
                    role = role.as_str()
                ));
            }
            continue;
        };
        let text = body["text"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_default();
        if text.is_empty() && role.is_required() {
            return Err(format!(
                "required {role} target `{to_revision}` has no text",
                role = role.as_str()
            ));
        }
        items.push(ClosureItem {
            role,
            revision: to_revision,
            text,
        });
    }
    let mut contrary = Vec::new();
    if let Some(decision_id) = legacy_decision_id {
        let mut stmt = conn.prepare("SELECT c.id, c.classification, c.status, o.id, o.decision FROM decision_conflicts c JOIN decisions o ON o.id = CASE WHEN c.source_decision_id = ?1 THEN c.target_decision_id ELSE c.source_decision_id END WHERE (c.source_decision_id = ?1 OR c.target_decision_id = ?1) AND c.classification = 'CONTRADICTS' AND c.status = 'open' ORDER BY c.id").map_err(|e| e.to_string())?;
        let rows = stmt.query_map(params![decision_id], |r| Ok(json!({"conflict": r.get::<_, i64>(0)?, "classification": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "other": format!("decision::{}", r.get::<_, i64>(3)?), "text": r.get::<_, String>(4)?}))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        contrary.extend(rows);
    }
    Ok((items, contrary))
}
