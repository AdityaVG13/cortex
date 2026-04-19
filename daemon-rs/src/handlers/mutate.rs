// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::{ensure_admin, ensure_auth_rated, json_response, log_event, now_iso};
use crate::db::{archive_entries_scoped, checkpoint_wal_best_effort};
use crate::state::RuntimeState;

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ForgetRequest {
    pub keyword: Option<String>,
    pub source: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ResolveRequest {
    #[serde(rename = "keepId", alias = "winnerId")]
    pub keep_id: Option<i64>,
    pub action: Option<String>,
    #[serde(rename = "supersededId", alias = "loserId")]
    pub superseded_id: Option<i64>,
    #[serde(rename = "conflictId", alias = "id")]
    pub conflict_id: Option<String>,
    pub classification: Option<String>,
    pub notes: Option<String>,
    #[serde(rename = "resolvedBy", alias = "resolved_by")]
    pub resolved_by: Option<String>,
    pub similarity: Option<f64>,
}

#[derive(Deserialize, Default)]
pub struct ArchiveRequest {
    pub table: Option<String>,
    pub ids: Option<Vec<i64>>,
}

#[derive(Deserialize, Default)]
pub struct ConflictListQuery {
    pub status: Option<String>,
    pub classification: Option<String>,
    #[serde(rename = "id")]
    pub conflict_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize, Default)]
pub struct PermissionGrantRequest {
    pub client: Option<String>,
    pub permission: Option<String>,
    pub scope: Option<String>,
    #[serde(rename = "grantedBy", alias = "granted_by")]
    pub granted_by: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct PermissionRevokeRequest {
    pub client: Option<String>,
    pub permission: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStatusFilter {
    Open,
    Resolved,
    All,
}

impl ConflictStatusFilter {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(Self::Open),
            Some(value) => match value.to_ascii_lowercase().as_str() {
                "open" => Ok(Self::Open),
                "resolved" => Ok(Self::Resolved),
                "all" => Ok(Self::All),
                _ => Err("Invalid status filter. Expected open, resolved, or all.".to_string()),
            },
        }
    }

    fn includes_open(self) -> bool {
        matches!(self, Self::Open | Self::All)
    }

    fn includes_resolved(self) -> bool {
        matches!(self, Self::Resolved | Self::All)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConflictListOptions {
    pub status: ConflictStatusFilter,
    pub classification: Option<String>,
    pub conflict_id: Option<String>,
    pub limit: usize,
}

impl Default for ConflictListOptions {
    fn default() -> Self {
        Self {
            status: ConflictStatusFilter::Open,
            classification: None,
            conflict_id: None,
            limit: 100,
        }
    }
}

impl ConflictListOptions {
    fn from_query(query: ConflictListQuery) -> Result<Self, String> {
        let status = ConflictStatusFilter::parse(query.status.as_deref())?;
        let classification = match query
            .classification
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(raw) => Some(normalize_conflict_classification(raw).ok_or_else(|| {
                "Invalid classification filter. Expected AGREES, CONTRADICTS, REFINES, or UNRELATED."
                    .to_string()
            })?),
            None => None,
        };
        let conflict_id = query
            .conflict_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(id) = conflict_id.as_deref() {
            if parse_conflict_id(id).is_none() {
                return Err(
                    "Invalid conflict id. Expected decision:<id>:<id> or <id>:<id>.".into(),
                );
            }
        }
        Ok(Self {
            status,
            classification,
            conflict_id,
            limit: query.limit.unwrap_or(100).clamp(1, 500),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionMetadata {
    pub conflict_id: Option<String>,
    pub classification: Option<String>,
    pub notes: Option<String>,
    pub resolved_by: Option<String>,
    pub similarity: Option<f64>,
}

fn normalize_permission_client_id(raw: &str) -> Option<String> {
    let before_model = raw
        .split('(')
        .next()
        .unwrap_or(raw)
        .trim()
        .to_ascii_lowercase();
    let normalized: String = before_model
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn parse_permission(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "read" => Some("read"),
        "write" => Some("write"),
        "admin" => Some("admin"),
        _ => None,
    }
}

fn normalize_permission_scope(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "*".to_string())
}

pub fn list_permissions(conn: &Connection, owner_id: i64) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT client_id, permission, scope, granted_by, granted_at
             FROM client_permissions
             WHERE owner_id = ?1
             ORDER BY client_id ASC, permission ASC, scope ASC",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![owner_id], |row| {
            Ok(json!({
                "client": row.get::<_, String>(0)?,
                "permission": row.get::<_, String>(1)?,
                "scope": row.get::<_, String>(2)?,
                "grantedBy": row.get::<_, String>(3)?,
                "grantedAt": row.get::<_, String>(4)?,
            }))
        })
        .map_err(|err| err.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}

pub fn grant_permission(
    conn: &Connection,
    owner_id: i64,
    client: &str,
    permission: &str,
    scope: &str,
    granted_by: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by, granted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(owner_id, client_id, permission, scope)
         DO UPDATE SET granted_by = excluded.granted_by, granted_at = excluded.granted_at",
        params![owner_id, client, permission, scope, granted_by],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}

pub fn revoke_permission(
    conn: &Connection,
    owner_id: i64,
    client: &str,
    permission: &str,
    scope: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM client_permissions
         WHERE owner_id = ?1 AND client_id = ?2 AND permission = ?3 AND scope = ?4",
        params![owner_id, client, permission, scope],
    )
    .map_err(|err| err.to_string())
}

pub fn parse_conflict_id(raw: &str) -> Option<(i64, i64)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let payload = trimmed
        .strip_prefix("decision:")
        .or_else(|| trimmed.strip_prefix("decision_pair:"))
        .unwrap_or(trimmed);
    let mut parts = payload.split(':');
    let a = parts.next()?.trim().parse::<i64>().ok()?;
    let b = parts.next()?.trim().parse::<i64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((a.min(b), a.max(b)))
}

fn conflict_id_from_pair(a: i64, b: i64) -> String {
    let (left, right) = (a.min(b), a.max(b));
    format!("decision:{left}:{right}")
}

fn normalize_conflict_classification(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_uppercase();
    match normalized.as_str() {
        "AGREES" | "CONTRADICTS" | "REFINES" | "UNRELATED" => Some(normalized),
        _ => None,
    }
}

fn default_classification_for_action(action: &str) -> &'static str {
    match action {
        "merge" => "REFINES",
        "archive" => "UNRELATED",
        _ => "CONTRADICTS",
    }
}

struct DecisionNodeRecord {
    id: i64,
    decision: String,
    context: Option<String>,
    source_agent: Option<String>,
    source_client: Option<String>,
    source_model: Option<String>,
    reasoning_depth: Option<String>,
    confidence: Option<f64>,
    trust_score: Option<f64>,
    status: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
}

fn build_decision_node(record: DecisionNodeRecord) -> Value {
    let source_agent_legacy = record.source_agent.clone();
    let created_at_legacy = record.created_at.clone();
    let updated_at_legacy = record.updated_at.clone();
    json!({
        "id": record.id,
        "decision": record.decision,
        "context": record.context,
        "sourceAgent": source_agent_legacy,
        "source_agent": record.source_agent,
        "sourceClient": record.source_client,
        "sourceModel": record.source_model,
        "reasoningDepth": record.reasoning_depth,
        "confidence": record.confidence,
        "trustScore": record.trust_score,
        "status": record.status,
        "createdAt": created_at_legacy,
        "created_at": record.created_at,
        "updatedAt": updated_at_legacy,
        "updated_at": record.updated_at,
    })
}

fn decision_node_missing(id: i64) -> Value {
    json!({
        "id": id,
        "missing": true
    })
}

fn fetch_decision_nodes_by_ids(
    conn: &Connection,
    ids: &[i64],
) -> Result<HashMap<i64, Value>, String> {
    let mut unique_ids = ids.to_vec();
    unique_ids.sort_unstable();
    unique_ids.dedup();
    if unique_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = vec!["?"; unique_ids.len()].join(", ");
    let sql = format!(
        "SELECT id, decision, context, source_agent, source_client, source_model, reasoning_depth,
                confidence, trust_score, status, created_at, updated_at
         FROM decisions
         WHERE id IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(unique_ids.iter()), |row| {
            let id: i64 = row.get(0)?;
            Ok((
                id,
                build_decision_node(DecisionNodeRecord {
                    id,
                    decision: row.get::<_, String>(1)?,
                    context: row.get::<_, Option<String>>(2)?,
                    source_agent: row.get::<_, Option<String>>(3)?,
                    source_client: row.get::<_, Option<String>>(4)?,
                    source_model: row.get::<_, Option<String>>(5)?,
                    reasoning_depth: row.get::<_, Option<String>>(6)?,
                    confidence: row.get::<_, Option<f64>>(7)?,
                    trust_score: row.get::<_, Option<f64>>(8)?,
                    status: row.get::<_, Option<String>>(9)?,
                    created_at: row.get::<_, Option<String>>(10)?,
                    updated_at: row.get::<_, Option<String>>(11)?,
                }),
            ))
        })
        .map_err(|err| err.to_string())?;

    let mut out = HashMap::with_capacity(unique_ids.len());
    for row in rows.flatten() {
        out.insert(row.0, row.1);
    }
    Ok(out)
}

fn decision_text(node: &Value) -> Option<&str> {
    node.get("decision").and_then(|value| value.as_str())
}

fn trust_snapshot(node: &Value) -> Value {
    json!({
        "id": node.get("id").cloned().unwrap_or(Value::Null),
        "confidence": node.get("confidence").cloned().unwrap_or(Value::Null),
        "trustScore": node.get("trustScore").cloned().unwrap_or(Value::Null),
        "sourceClient": node.get("sourceClient").cloned().unwrap_or(Value::Null),
        "sourceModel": node.get("sourceModel").cloned().unwrap_or(Value::Null),
        "reasoningDepth": node.get("reasoningDepth").cloned().unwrap_or(Value::Null),
        "sourceAgent": node.get("sourceAgent").cloned().unwrap_or(Value::Null),
    })
}

fn preferred_winner_id(left: &Value, right: &Value) -> Option<i64> {
    let left_id = left.get("id").and_then(|value| value.as_i64())?;
    let right_id = right.get("id").and_then(|value| value.as_i64())?;
    let left_trust = left
        .get("trustScore")
        .and_then(|value| value.as_f64())
        .or_else(|| left.get("confidence").and_then(|value| value.as_f64()))
        .unwrap_or(0.0);
    let right_trust = right
        .get("trustScore")
        .and_then(|value| value.as_f64())
        .or_else(|| right.get("confidence").and_then(|value| value.as_f64()))
        .unwrap_or(0.0);
    if (left_trust - right_trust).abs() < f64::EPSILON {
        Some(left_id.min(right_id))
    } else if left_trust >= right_trust {
        Some(left_id)
    } else {
        Some(right_id)
    }
}

fn conflict_matches_filters(conflict: &Value, options: &ConflictListOptions) -> bool {
    if let Some(expected) = options.classification.as_deref() {
        if conflict
            .get("classification")
            .and_then(|value| value.as_str())
            .map(|value| value != expected)
            .unwrap_or(true)
        {
            return false;
        }
    }
    if let Some(expected_id) = options.conflict_id.as_deref() {
        if conflict
            .get("id")
            .and_then(|value| value.as_str())
            .map(|value| value != expected_id)
            .unwrap_or(true)
        {
            return false;
        }
    }
    true
}

fn legacy_pair_from_conflict(conflict: &Value) -> Value {
    let left = conflict.get("left").cloned().unwrap_or(Value::Null);
    let right = conflict.get("right").cloned().unwrap_or(Value::Null);
    json!({
        "left": {
            "id": left.get("id").cloned().unwrap_or(Value::Null),
            "decision": left.get("decision").cloned().unwrap_or(Value::Null),
            "context": left.get("context").cloned().unwrap_or(Value::Null),
            "source_agent": left
                .get("source_agent")
                .cloned()
                .or_else(|| left.get("sourceAgent").cloned())
                .unwrap_or(Value::Null),
            "confidence": left.get("confidence").cloned().unwrap_or(Value::Null),
            "created_at": left
                .get("created_at")
                .cloned()
                .or_else(|| left.get("createdAt").cloned())
                .unwrap_or(Value::Null),
        },
        "right": {
            "id": right.get("id").cloned().unwrap_or(Value::Null),
            "decision": right.get("decision").cloned().unwrap_or(Value::Null),
            "context": right.get("context").cloned().unwrap_or(Value::Null),
            "source_agent": right
                .get("source_agent")
                .cloned()
                .or_else(|| right.get("sourceAgent").cloned())
                .unwrap_or(Value::Null),
            "confidence": right.get("confidence").cloned().unwrap_or(Value::Null),
            "created_at": right
                .get("created_at")
                .cloned()
                .or_else(|| right.get("createdAt").cloned())
                .unwrap_or(Value::Null),
        },
    })
}

fn list_open_conflicts(conn: &Connection, limit: usize) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT
                d1.id, d1.decision, d1.context, d1.source_agent, d1.source_client, d1.source_model, d1.reasoning_depth,
                d1.confidence, d1.trust_score, d1.status, d1.created_at, d1.updated_at,
                d2.id, d2.decision, d2.context, d2.source_agent, d2.source_client, d2.source_model, d2.reasoning_depth,
                d2.confidence, d2.trust_score, d2.status, d2.created_at, d2.updated_at
             FROM decisions d1
             JOIN decisions d2 ON d1.disputes_id = d2.id
             WHERE d1.status = 'disputed' AND d1.id > d2.id
             ORDER BY d1.created_at DESC
             LIMIT ?1",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(params![limit as i64], |row| {
            let left_id = row.get::<_, i64>(0)?;
            let left_decision = row.get::<_, String>(1)?;
            let right_id = row.get::<_, i64>(12)?;
            let right_decision = row.get::<_, String>(13)?;

            let left = build_decision_node(DecisionNodeRecord {
                id: left_id,
                decision: left_decision.clone(),
                context: row.get::<_, Option<String>>(2)?,
                source_agent: row.get::<_, Option<String>>(3)?,
                source_client: row.get::<_, Option<String>>(4)?,
                source_model: row.get::<_, Option<String>>(5)?,
                reasoning_depth: row.get::<_, Option<String>>(6)?,
                confidence: row.get::<_, Option<f64>>(7)?,
                trust_score: row.get::<_, Option<f64>>(8)?,
                status: row.get::<_, Option<String>>(9)?,
                created_at: row.get::<_, Option<String>>(10)?,
                updated_at: row.get::<_, Option<String>>(11)?,
            });
            let right = build_decision_node(DecisionNodeRecord {
                id: right_id,
                decision: right_decision.clone(),
                context: row.get::<_, Option<String>>(14)?,
                source_agent: row.get::<_, Option<String>>(15)?,
                source_client: row.get::<_, Option<String>>(16)?,
                source_model: row.get::<_, Option<String>>(17)?,
                reasoning_depth: row.get::<_, Option<String>>(18)?,
                confidence: row.get::<_, Option<f64>>(19)?,
                trust_score: row.get::<_, Option<f64>>(20)?,
                status: row.get::<_, Option<String>>(21)?,
                created_at: row.get::<_, Option<String>>(22)?,
                updated_at: row.get::<_, Option<String>>(23)?,
            });

            let similarity = crate::conflict::jaccard_similarity(&left_decision, &right_decision);
            let classification = "CONTRADICTS".to_string();
            let conflict_id = conflict_id_from_pair(left_id, right_id);

            Ok(json!({
                "id": conflict_id,
                "status": "open",
                "classification": classification,
                "similarity": similarity,
                "left": left,
                "right": right,
                "trustContext": {
                    "left": trust_snapshot(&left),
                    "right": trust_snapshot(&right),
                    "recommendedWinnerId": preferred_winner_id(&left, &right),
                },
                "resolution": Value::Null
            }))
        })
        .map_err(|err| err.to_string())?;

    Ok(rows.filter_map(Result::ok).collect())
}

fn list_resolved_conflicts(conn: &Connection, limit: usize) -> Result<Vec<Value>, String> {
    #[derive(Debug)]
    struct ResolvedConflictSeed {
        conflict_id: String,
        left_id: i64,
        right_id: i64,
        winner_id: i64,
        superseded_id: Option<i64>,
        action: String,
        classification: String,
        similarity: Option<f64>,
        resolved_by: Option<String>,
        resolved_at: String,
        notes: Value,
        resolution_classification: Value,
    }

    let mut stmt = conn
        .prepare(
            "SELECT data, source_agent, created_at
             FROM events
             WHERE type = 'decision_resolve'
             ORDER BY id DESC
             LIMIT ?1",
        )
        .map_err(|err| err.to_string())?;

    let rows = stmt
        .query_map(params![limit as i64], |row| {
            let data_raw: String = row.get(0)?;
            let source_agent: Option<String> = row.get(1)?;
            let created_at: String = row.get(2)?;
            Ok((data_raw, source_agent, created_at))
        })
        .map_err(|err| err.to_string())?;

    let mut seeds = Vec::new();
    let mut decision_ids = Vec::new();
    for row in rows.flatten() {
        let (data_raw, source_agent, created_at) = row;
        let data: Value = serde_json::from_str(&data_raw).unwrap_or_else(|_| json!({}));
        let winner_id = data
            .get("winnerId")
            .and_then(|value| value.as_i64())
            .or_else(|| data.get("keepId").and_then(|value| value.as_i64()));
        let superseded_id = data.get("supersededId").and_then(|value| value.as_i64());
        let Some(winner_id) = winner_id else {
            continue;
        };

        let conflict_id = data
            .get("conflictId")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| superseded_id.map(|other| conflict_id_from_pair(winner_id, other)))
            .unwrap_or_else(|| conflict_id_from_pair(winner_id, winner_id));

        let (left_id, right_id) = parse_conflict_id(&conflict_id).unwrap_or_else(|| {
            (
                winner_id.min(superseded_id.unwrap_or(winner_id)),
                winner_id.max(superseded_id.unwrap_or(winner_id)),
            )
        });
        let action = data
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("keep")
            .to_string();
        let classification = data
            .get("classification")
            .and_then(|value| value.as_str())
            .and_then(normalize_conflict_classification)
            .unwrap_or_else(|| default_classification_for_action(&action).to_string());
        let resolved_by = data
            .get("resolvedBy")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(source_agent.clone());
        let resolved_at = data
            .get("resolvedAt")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or(created_at);

        decision_ids.push(left_id);
        decision_ids.push(right_id);
        seeds.push(ResolvedConflictSeed {
            conflict_id,
            left_id,
            right_id,
            winner_id,
            superseded_id,
            action,
            classification,
            similarity: data.get("similarity").and_then(|value| value.as_f64()),
            resolved_by,
            resolved_at,
            notes: data.get("notes").cloned().unwrap_or(Value::Null),
            resolution_classification: data.get("classification").cloned().unwrap_or(Value::Null),
        });
    }

    let decision_nodes = fetch_decision_nodes_by_ids(conn, &decision_ids)?;
    let mut conflicts = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let left = decision_nodes
            .get(&seed.left_id)
            .cloned()
            .unwrap_or_else(|| decision_node_missing(seed.left_id));
        let right = decision_nodes
            .get(&seed.right_id)
            .cloned()
            .unwrap_or_else(|| decision_node_missing(seed.right_id));
        let similarity = seed.similarity.or_else(|| {
            let left_text = decision_text(&left)?;
            let right_text = decision_text(&right)?;
            Some(crate::conflict::jaccard_similarity(left_text, right_text))
        });
        conflicts.push(json!({
            "id": seed.conflict_id,
            "status": "resolved",
            "classification": seed.classification,
            "similarity": similarity,
            "left": left,
            "right": right,
            "trustContext": {
                "left": trust_snapshot(&left),
                "right": trust_snapshot(&right),
                "recommendedWinnerId": preferred_winner_id(&left, &right),
            },
            "resolution": {
                "action": seed.action,
                "winnerId": seed.winner_id,
                "supersededId": seed.superseded_id,
                "resolvedAt": seed.resolved_at,
                "resolvedBy": seed.resolved_by,
                "notes": seed.notes,
                "classification": seed.resolution_classification,
            }
        }));
    }

    Ok(conflicts)
}

pub fn list_conflicts_payload(
    conn: &Connection,
    options: &ConflictListOptions,
) -> Result<Value, String> {
    let mut open_conflicts = if options.status.includes_open() {
        list_open_conflicts(conn, options.limit)?
    } else {
        Vec::new()
    };
    let mut resolved_conflicts = if options.status.includes_resolved() {
        list_resolved_conflicts(conn, options.limit)?
    } else {
        Vec::new()
    };

    open_conflicts.retain(|entry| conflict_matches_filters(entry, options));
    resolved_conflicts.retain(|entry| conflict_matches_filters(entry, options));

    let mut conflicts = Vec::with_capacity(open_conflicts.len() + resolved_conflicts.len());
    if options.status.includes_open() {
        conflicts.extend(open_conflicts.clone());
    }
    if options.status.includes_resolved() {
        conflicts.extend(resolved_conflicts.clone());
    }

    let pairs: Vec<Value> = open_conflicts
        .iter()
        .map(legacy_pair_from_conflict)
        .collect();
    let conflict = if options.conflict_id.is_some() {
        conflicts.first().cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };

    Ok(json!({
        "statusFilter": options.status.as_str(),
        "classificationFilter": options.classification,
        "conflictIdFilter": options.conflict_id,
        "openCount": open_conflicts.len(),
        "resolvedCount": resolved_conflicts.len(),
        "count": conflicts.len(),
        "pairs": pairs,
        "conflicts": conflicts,
        "conflict": conflict,
    }))
}

#[allow(clippy::result_large_err)]
fn ensure_admin_surface(
    headers: &HeaderMap,
    state: &RuntimeState,
    conn: &Connection,
) -> Result<Option<i64>, Response> {
    if state.team_mode {
        ensure_admin(headers, state, conn).map(Some)
    } else {
        Ok(None)
    }
}

// ─── POST /forget ────────────────────────────────────────────────────────────

pub async fn handle_forget(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ForgetRequest>,
) -> Response {
    let keyword = body.keyword.or(body.source).unwrap_or_default();
    if keyword.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing field: keyword" }),
        );
    }

    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let mut conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    match forget_keyword_scoped(&mut conn, keyword.trim(), owner_id) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "affected": affected })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Forget failed: {err}") }),
        ),
    }
}

pub fn forget_keyword_scoped(
    conn: &mut Connection,
    keyword: &str,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    let pattern = format!("%{}%", keyword.to_lowercase());
    let now = now_iso();
    let (memories, decisions) = if let Some(owner_id) = owner_id {
        let memories = conn
            .execute(
                "UPDATE memories SET score = score * 0.3, updated_at = ?2 \
                 WHERE owner_id = ?3 AND status = 'active' AND (lower(text) LIKE ?1 OR lower(source) LIKE ?1)",
                params![pattern.clone(), now.clone(), owner_id],
            )
            .map_err(|e| e.to_string())?;
        let decisions = conn
            .execute(
                "UPDATE decisions SET score = score * 0.3, updated_at = ?2 \
                 WHERE owner_id = ?3 AND status = 'active' AND (lower(decision) LIKE ?1 OR lower(context) LIKE ?1)",
                params![pattern, now, owner_id],
            )
            .map_err(|e| e.to_string())?;
        (memories, decisions)
    } else {
        let memories = conn
            .execute(
                "UPDATE memories SET score = score * 0.3, updated_at = ?2 \
                 WHERE status = 'active' AND (lower(text) LIKE ?1 OR lower(source) LIKE ?1)",
                params![pattern.clone(), now.clone()],
            )
            .map_err(|e| e.to_string())?;
        let decisions = conn
            .execute(
                "UPDATE decisions SET score = score * 0.3, updated_at = ?2 \
                 WHERE status = 'active' AND (lower(decision) LIKE ?1 OR lower(context) LIKE ?1)",
                params![pattern, now],
            )
            .map_err(|e| e.to_string())?;
        (memories, decisions)
    };
    let affected = memories + decisions;
    if affected > 0 {
        let _ = log_event(
            conn,
            "forget",
            json!({ "keyword": keyword, "affected": affected, "ownerId": owner_id }),
            "rust-daemon",
        );
        checkpoint_wal_best_effort(conn);
    }
    Ok(affected)
}

// ─── POST /resolve ───────────────────────────────────────────────────────────

pub async fn handle_resolve(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ResolveRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let mut conn = state.db.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }

    let mut keep_id = body.keep_id;
    let mut superseded_id = body.superseded_id;
    if let Some((a, b)) = body.conflict_id.as_deref().and_then(parse_conflict_id) {
        if keep_id.is_none() {
            keep_id = Some(a);
        }
        if superseded_id.is_none() {
            superseded_id = keep_id.map(|winner| {
                if winner == a {
                    b
                } else if winner == b {
                    a
                } else {
                    b
                }
            });
        }
    }

    let keep_id = match keep_id {
        Some(value) => value,
        _ => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing fields: keepId, action" }),
            );
        }
    };
    let action = match body
        .action
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing fields: keepId, action" }),
            );
        }
    };

    let metadata = ResolutionMetadata {
        conflict_id: body.conflict_id.clone(),
        classification: body.classification.clone(),
        notes: body.notes.clone(),
        resolved_by: body.resolved_by.clone(),
        similarity: body.similarity,
    };

    match resolve_decision_with_metadata(&mut conn, keep_id, action, superseded_id, metadata) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Resolve failed: {err}") }),
        ),
    }
}

pub fn resolve_decision(
    conn: &mut Connection,
    keep_id: i64,
    action: &str,
    superseded_id: Option<i64>,
) -> Result<(), String> {
    resolve_decision_with_metadata(
        conn,
        keep_id,
        action,
        superseded_id,
        ResolutionMetadata::default(),
    )?;
    Ok(())
}

pub fn resolve_decision_with_metadata(
    conn: &mut Connection,
    keep_id: i64,
    action: &str,
    superseded_id: Option<i64>,
    metadata: ResolutionMetadata,
) -> Result<Value, String> {
    let resolved_at = now_iso();
    match action {
        "keep" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'superseded', supersedes_id = ?1, disputes_id = NULL, updated_at = ?3 WHERE id = ?2",
                    params![keep_id, other, resolved_at],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        "merge" => {
            conn.execute(
                "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                    params![other, resolved_at],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        "archive" => {
            conn.execute(
                "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                params![keep_id, resolved_at],
            )
            .map_err(|e| e.to_string())?;
            if let Some(other) = superseded_id {
                conn.execute(
                    "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
                    params![other, resolved_at],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("Invalid action. Expected keep, merge, or archive.".to_string()),
    }

    let classification = metadata
        .classification
        .as_deref()
        .and_then(normalize_conflict_classification)
        .unwrap_or_else(|| default_classification_for_action(action).to_string());
    let conflict_id = metadata
        .conflict_id
        .or_else(|| superseded_id.map(|other| conflict_id_from_pair(keep_id, other)));
    let resolved_by = metadata
        .resolved_by
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rust-daemon".to_string());

    let event_payload = json!({
        "conflictId": conflict_id,
        "keepId": keep_id,
        "winnerId": keep_id,
        "action": action,
        "supersededId": superseded_id,
        "classification": classification,
        "similarity": metadata.similarity,
        "resolvedBy": resolved_by,
        "resolvedAt": resolved_at,
        "notes": metadata.notes,
    });

    let _ = log_event(
        conn,
        "decision_resolve",
        event_payload.clone(),
        &resolved_by,
    );
    checkpoint_wal_best_effort(conn);
    Ok(json!({
        "resolved": true,
        "conflictId": event_payload.get("conflictId").cloned().unwrap_or(Value::Null),
        "winnerId": keep_id,
        "keepId": keep_id,
        "supersededId": superseded_id,
        "action": action,
        "classification": event_payload.get("classification").cloned().unwrap_or(Value::Null),
        "similarity": event_payload.get("similarity").cloned().unwrap_or(Value::Null),
        "resolvedBy": event_payload.get("resolvedBy").cloned().unwrap_or(Value::Null),
        "resolvedAt": event_payload.get("resolvedAt").cloned().unwrap_or(Value::Null),
        "notes": event_payload.get("notes").cloned().unwrap_or(Value::Null),
    }))
}

// ─── GET /conflicts ──────────────────────────────────────────────────────────

pub async fn handle_conflicts(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<ConflictListQuery>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }

    let options = match ConflictListOptions::from_query(query) {
        Ok(options) => options,
        Err(err) => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": err }));
        }
    };

    match list_conflicts_payload(&conn, &options) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Conflict query failed: {err}") }),
        ),
    }
}

// ─── GET /permissions ────────────────────────────────────────────────────────

pub async fn handle_permissions_list(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };

    match list_permissions(&conn, owner_id) {
        Ok(grants) => json_response(
            StatusCode::OK,
            json!({
                "ownerId": owner_id,
                "count": grants.len(),
                "grants": grants,
            }),
        ),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Permission list failed: {err}") }),
        ),
    }
}

// ─── POST /permissions/grant ────────────────────────────────────────────────

pub async fn handle_permissions_grant(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<PermissionGrantRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };

    let raw_client = match body
        .client
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing field: client" }),
            );
        }
    };
    let client = if raw_client == "*" {
        "*".to_string()
    } else if let Some(normalized) = normalize_permission_client_id(raw_client) {
        normalized
    } else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Invalid client id. Use letters, numbers, '-', '_'." }),
        );
    };

    let permission = match body
        .permission
        .as_deref()
        .and_then(parse_permission)
        .map(str::to_string)
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Invalid permission; expected read, write, or admin" }),
            );
        }
    };

    let scope = normalize_permission_scope(body.scope.as_deref());
    let granted_by = body
        .granted_by
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(normalize_permission_client_id)
        .unwrap_or_else(|| "control-center".to_string());

    match grant_permission(&conn, owner_id, &client, &permission, &scope, &granted_by) {
        Ok(()) => json_response(
            StatusCode::OK,
            json!({
                "granted": true,
                "ownerId": owner_id,
                "client": client,
                "permission": permission,
                "scope": scope,
            }),
        ),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Permission grant failed: {err}") }),
        ),
    }
}

// ─── POST /permissions/revoke ───────────────────────────────────────────────

pub async fn handle_permissions_revoke(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<PermissionRevokeRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(user_id) => user_id.unwrap_or(0),
        Err(resp) => return resp,
    };

    let raw_client = match body
        .client
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Missing field: client" }),
            );
        }
    };
    let client = if raw_client == "*" {
        "*".to_string()
    } else if let Some(normalized) = normalize_permission_client_id(raw_client) {
        normalized
    } else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Invalid client id. Use letters, numbers, '-', '_'." }),
        );
    };

    let permission = match body
        .permission
        .as_deref()
        .and_then(parse_permission)
        .map(str::to_string)
    {
        Some(value) => value,
        None => {
            return json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Invalid permission; expected read, write, or admin" }),
            );
        }
    };
    let scope = normalize_permission_scope(body.scope.as_deref());

    match revoke_permission(&conn, owner_id, &client, &permission, &scope) {
        Ok(deleted) => json_response(
            StatusCode::OK,
            json!({
                "revoked": deleted > 0,
                "deleted": deleted,
                "ownerId": owner_id,
                "client": client,
                "permission": permission,
                "scope": scope,
            }),
        ),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Permission revoke failed: {err}") }),
        ),
    }
}

// ─── POST /archive ───────────────────────────────────────────────────────────

pub async fn handle_archive(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<ArchiveRequest>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let table = body.table.unwrap_or_default();
    let ids = body.ids.unwrap_or_default();

    if table.is_empty() || ids.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing fields: table, ids" }),
        );
    }

    let conn = state.db.lock().await;
    let owner_id = match ensure_admin_surface(&headers, &state, &conn) {
        Ok(owner_id) => owner_id,
        Err(resp) => return resp,
    };
    match archive_entries_scoped(&conn, &table, &ids, owner_id) {
        Ok(affected) => json_response(StatusCode::OK, json!({ "archived": affected })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Archive failed: {err}") }),
        ),
    }
}

// ─── POST /shutdown ──────────────────────────────────────────────────────────

pub async fn handle_shutdown(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db.lock().await;
    if let Err(resp) = ensure_admin_surface(&headers, &state, &conn) {
        return resp;
    }

    // WAL checkpoint before exiting
    checkpoint_wal_best_effort(&conn);
    drop(conn);

    // Fire the oneshot shutdown signal
    let mut tx_guard = state.shutdown_tx.lock().await;
    if let Some(tx) = tx_guard.take() {
        let _ = tx.send(());
    }

    json_response(StatusCode::OK, json!({ "shutdown": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{HeaderValue, StatusCode};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn test_state(team_mode: bool) -> RuntimeState {
        let write_conn = test_conn();
        let read_conn = test_conn();
        let (events, _) = broadcast::channel(8);
        RuntimeState {
            db: Arc::new(Mutex::new(write_conn)),
            db_read: Arc::new(Mutex::new(read_conn)),
            token: Arc::new("test-token".to_string()),
            events,
            mcp_calls: Arc::new(AtomicU64::new(0)),
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            recall_history: Arc::new(Mutex::new(HashMap::new())),
            pre_cache: Arc::new(Mutex::new(HashMap::new())),
            served_content: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            home: PathBuf::from("."),
            db_path: PathBuf::from(":memory:"),
            token_path: PathBuf::from("cortex.token"),
            pid_path: PathBuf::from("cortex.pid"),
            port: 7437,
            embedding_engine: None,
            rate_limiter: crate::rate_limit::RateLimiter::new(),
            team_mode,
            default_owner_id: None,
            team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
            degraded_mode: Arc::new(AtomicBool::new(false)),
            db_corrupted: Arc::new(AtomicBool::new(false)),
            readiness: Arc::new(AtomicBool::new(true)),
            write_buffer_path: PathBuf::from("write_buffer.jsonl"),
            sqlite_vec_canary: crate::state::SqliteVecCanaryConfig {
                trial_percent: 0,
                force_off: false,
                route_mode: crate::state::SqliteVecRouteMode::Trial,
            },
        }
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers.insert("x-cortex-request", HeaderValue::from_static("desktop"));
        headers
    }

    fn insert_disputed_pair(conn: &Connection) -> (i64, i64) {
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'claude', 'claude', 0.72, 0.74, 'active')",
            params!["Always use SQLite for local dev", "DB policy"],
        )
        .unwrap();
        let first = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'codex', 'codex', 0.91, 0.95, 'active')",
            params!["Use PostgreSQL for production workloads", "DB policy"],
        )
        .unwrap();
        let second = conn.last_insert_rowid();

        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            params![second, first],
        )
        .unwrap();
        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            params![first, second],
        )
        .unwrap();

        (first, second)
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn conflict_list_reports_open_and_resolved_with_metadata() {
        let mut conn = test_conn();
        let (first, second) = insert_disputed_pair(&conn);

        let open_payload = list_conflicts_payload(&conn, &ConflictListOptions::default()).unwrap();
        assert_eq!(open_payload["openCount"].as_u64(), Some(1));
        assert_eq!(open_payload["count"].as_u64(), Some(1));
        assert_eq!(open_payload["pairs"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            open_payload["conflicts"][0]["classification"].as_str(),
            Some("CONTRADICTS")
        );

        let resolution = resolve_decision_with_metadata(
            &mut conn,
            second,
            "keep",
            Some(first),
            ResolutionMetadata {
                conflict_id: Some(conflict_id_from_pair(first, second)),
                classification: Some("CONTRADICTS".to_string()),
                notes: Some("Prefer higher trust score".to_string()),
                resolved_by: Some("codex".to_string()),
                similarity: Some(0.67),
            },
        )
        .unwrap();

        assert_eq!(resolution["resolved"].as_bool(), Some(true));
        assert_eq!(resolution["winnerId"].as_i64(), Some(second));
        assert_eq!(resolution["supersededId"].as_i64(), Some(first));

        let resolved_payload = list_conflicts_payload(
            &conn,
            &ConflictListOptions {
                status: ConflictStatusFilter::Resolved,
                classification: Some("CONTRADICTS".to_string()),
                conflict_id: Some(conflict_id_from_pair(first, second)),
                limit: 100,
            },
        )
        .unwrap();
        assert_eq!(resolved_payload["resolvedCount"].as_u64(), Some(1));
        assert_eq!(
            resolved_payload["conflicts"][0]["resolution"]["resolvedBy"].as_str(),
            Some("codex")
        );
        assert_eq!(
            resolved_payload["conflicts"][0]["resolution"]["notes"].as_str(),
            Some("Prefer higher trust score")
        );
    }

    #[tokio::test]
    async fn conflicts_endpoint_requires_admin_in_team_mode() {
        let state = test_state(true);
        let response = handle_conflicts(
            State(state),
            auth_headers("test-token"),
            Query(ConflictListQuery::default()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Admin endpoints require team mode")
        );
    }

    #[test]
    fn permission_grant_list_and_revoke_round_trip() {
        let conn = test_conn();
        grant_permission(&conn, 0, "codex", "admin", "*", "control-center").unwrap();
        grant_permission(
            &conn,
            0,
            "claude",
            "read",
            "cortex_recall",
            "control-center",
        )
        .unwrap();

        let grants = list_permissions(&conn, 0).unwrap();
        assert_eq!(grants.len(), 2);
        assert_eq!(grants[0]["client"].as_str(), Some("claude"));
        assert_eq!(grants[1]["client"].as_str(), Some("codex"));

        let deleted = revoke_permission(&conn, 0, "claude", "read", "cortex_recall").unwrap();
        assert_eq!(deleted, 1);

        let grants = list_permissions(&conn, 0).unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["client"].as_str(), Some("codex"));
    }

    #[tokio::test]
    async fn permissions_endpoint_requires_admin_in_team_mode() {
        let state = test_state(true);
        let response = handle_permissions_list(State(state), auth_headers("test-token")).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Admin endpoints require team mode")
        );
    }

    #[test]
    fn conflict_filter_rejects_invalid_classification() {
        let err = ConflictListOptions::from_query(ConflictListQuery {
            status: Some("open".to_string()),
            classification: Some("contradictory".to_string()),
            conflict_id: None,
            limit: Some(20),
        })
        .expect_err("invalid classification should be rejected");
        assert!(err.contains("Invalid classification filter"));
    }

    #[tokio::test]
    async fn permissions_grant_rejects_invalid_client_shape() {
        let state = test_state(false);
        let response = handle_permissions_grant(
            State(state),
            auth_headers("test-token"),
            Json(PermissionGrantRequest {
                client: Some("!!!".to_string()),
                permission: Some("read".to_string()),
                scope: Some("*".to_string()),
                granted_by: Some("control-center".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Invalid client id. Use letters, numbers, '-', '_'.")
        );
    }
}
