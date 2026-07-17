use super::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;
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
            Ok(json!({"client":row.get::<_,String>(0)?,
"permission":row.get::<_,String>(1)?,"scope":row.get::<_,String>(2)?,"grantedBy":row.get::<_,String>(3)?,"grantedAt":row.get::<_,
String>(4)?,}))
        })
        .map_err(|err| err.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}
pub fn grant_permission(
    conn: &Connection, owner_id: i64, client: &str, permission: &str, scope: &str, granted_by: &str,
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
pub fn revoke_permission(conn: &Connection, owner_id: i64, client: &str, permission: &str, scope: &str) -> Result<usize, String> {
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
pub(crate) fn conflict_id_from_pair(a: i64, b: i64) -> String {
    let (left, right) = (a.min(b), a.max(b));
    format!("decision:{left}:{right}")
}
pub(crate) fn normalize_conflict_classification(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_uppercase();
    match normalized.as_str() {
        "AGREES" | "CONTRADICTS" | "REFINES" | "UNRELATED" => Some(normalized),
        _ => None,
    }
}
pub(crate) fn default_classification_for_action(action: &str) -> &'static str {
    match action {
        "merge" => "REFINES",
        "archive" => "UNRELATED",
        _ => "CONTRADICTS",
    }
}
pub(crate) struct DecisionNodeRecord {
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
pub(crate) fn build_decision_node(record: DecisionNodeRecord) -> Value {
    let source_agent_legacy = record.source_agent.clone();
    let created_at_legacy = record.created_at.clone();
    let updated_at_legacy = record.updated_at.clone();
    json!({"id":
record.id,"decision":record.decision,"context":record.context,"sourceAgent":source_agent_legacy,"source_agent":record.source_agent
,"sourceClient":record.source_client,"sourceModel":record.source_model,"reasoningDepth":record.reasoning_depth,"confidence":record
.confidence,"trustScore":record.trust_score,"status":record.status,"createdAt":created_at_legacy,"created_at":record.created_at,
"updatedAt":updated_at_legacy,"updated_at":record.updated_at,})
}
pub(crate) fn decision_node_missing(id: i64) -> Value {
    json!({"id":id,
"missing":true})
}
pub(crate) fn fetch_decision_nodes_by_ids(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, Value>, String> {
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
pub(crate) fn decision_text(node: &Value) -> Option<&str> {
    node.get("decision").and_then(|value| value.as_str())
}
pub(crate) fn trust_snapshot(node: &Value) -> Value {
    json!({"id":node.get("id").cloned().
unwrap_or(Value::Null),"confidence":node.get("confidence").cloned().unwrap_or(Value::Null),"trustScore":node.get("trustScore").
cloned().unwrap_or(Value::Null),"sourceClient":node.get("sourceClient").cloned().unwrap_or(Value::Null),"sourceModel":node.get(
"sourceModel").cloned().unwrap_or(Value::Null),"reasoningDepth":node.get("reasoningDepth").cloned().unwrap_or(Value::Null),
"sourceAgent":node.get("sourceAgent").cloned().unwrap_or(Value::Null),})
}
pub(crate) fn preferred_winner_id(left: &Value, right: &Value) -> Option<i64> {
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
pub(crate) fn conflict_matches_filters(conflict: &Value, options: &ConflictListOptions) -> bool {
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
pub(crate) fn legacy_pair_from_conflict(conflict: &Value) -> Value {
    let left = conflict.get("left").cloned().unwrap_or(Value::Null);
    let right = conflict.get("right").cloned().unwrap_or(Value::Null);
    json!({"left":{"id":left.get("id").cloned().unwrap_or(
Value::Null),"decision":left.get("decision").cloned().unwrap_or(Value::Null),"context":left.get("context").cloned().unwrap_or(
Value::Null),"source_agent":left.get("source_agent").cloned().or_else(||left.get("sourceAgent").cloned()).unwrap_or(Value::Null),
"confidence":left.get("confidence").cloned().unwrap_or(Value::Null),"created_at":left.get("created_at").cloned().or_else(||left.
get("createdAt").cloned()).unwrap_or(Value::Null),},"right":{"id":right.get("id").cloned().unwrap_or(Value::Null),"decision":right
.get("decision").cloned().unwrap_or(Value::Null),"context":right.get("context").cloned().unwrap_or(Value::Null),"source_agent":
right.get("source_agent").cloned().or_else(||right.get("sourceAgent").cloned()).unwrap_or(Value::Null),"confidence":right.get(
"confidence").cloned().unwrap_or(Value::Null),"created_at":right.get("created_at").cloned().or_else(||right.get("createdAt").
cloned()).unwrap_or(Value::Null),},})
}
pub(crate) fn list_open_conflicts(conn: &Connection, limit: usize) -> Result<Vec<Value>, String> {
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
            Ok(json!({"id":conflict_id,"status":"open",
"classification":classification,"similarity":similarity,"left":left,"right":right,"trustContext":{"left":trust_snapshot(&left),
"right":trust_snapshot(&right),"recommendedWinnerId":preferred_winner_id(&left,&right),},"resolution":Value::Null}))
        })
        .map_err(|err| err.to_string())?;
    Ok(rows.filter_map(Result::ok).collect())
}
pub(crate) fn list_resolved_conflicts(conn: &Connection, limit: usize) -> Result<Vec<Value>, String> {
    #[derive(Debug)]
    pub(crate) struct ResolvedConflictSeed {
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
        let (left_id, right_id) = parse_conflict_id(&conflict_id)
            .unwrap_or_else(|| (winner_id.min(superseded_id.unwrap_or(winner_id)), winner_id.max(superseded_id.unwrap_or(winner_id))));
        let action = data.get("action").and_then(|value| value.as_str()).unwrap_or("keep").to_string();
        let classification = data
            .get("classification")
            .and_then(|value| value.as_str())
            .and_then(normalize_conflict_classification)
            .unwrap_or_else(|| default_classification_for_action(&action).to_string());
        let resolved_by = data.get("resolvedBy").and_then(|value| value.as_str()).map(str::to_string).or(source_agent.clone());
        let resolved_at = data.get("resolvedAt").and_then(|value| value.as_str()).map(str::to_string).unwrap_or(created_at);
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
        let left = decision_nodes.get(&seed.left_id).cloned().unwrap_or_else(|| decision_node_missing(seed.left_id));
        let right = decision_nodes.get(&seed.right_id).cloned().unwrap_or_else(|| decision_node_missing(seed.right_id));
        let similarity = seed.similarity.or_else(|| {
            let left_text = decision_text(&left)?;
            let right_text = decision_text(&right)?;
            Some(crate::conflict::jaccard_similarity(left_text, right_text))
        });
        conflicts.push(json!({"id":seed.conflict_id,"status":
"resolved","classification":seed.classification,"similarity":similarity,"left":left,"right":right,"trustContext":{"left":
trust_snapshot(&left),"right":trust_snapshot(&right),"recommendedWinnerId":preferred_winner_id(&left,&right),},"resolution":{
"action":seed.action,"winnerId":seed.winner_id,"supersededId":seed.superseded_id,"resolvedAt":seed.resolved_at,"resolvedBy":seed.
resolved_by,"notes":seed.notes,"classification":seed.resolution_classification,}}));
    }
    Ok(conflicts)
}
