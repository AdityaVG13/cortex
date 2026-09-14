//! Mutation primitives (permissions, conflicts, forget, resolve). The HTTP
//! handlers live in the daemon; these are the in-process operations.

mod types;
pub use types::*;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub fn parse_conflict_id(raw: &str) -> Option<(i64, i64)> {
    let trimmed = raw.trim();
    // Longer prefix first: `decision:` is a prefix of `decision_pair:`.
    let payload = trimmed
        .strip_prefix("decision_pair:")
        .or_else(|| trimmed.strip_prefix("decision:"))
        .unwrap_or(trimmed);
    let mut parts = payload.split(':');
    let a = parts.next()?.trim().parse::<i64>().ok()?;
    let b = parts.next()?.trim().parse::<i64>().ok()?;
    parts.next().is_none().then_some((a.min(b), a.max(b)))
}

pub fn normalize_conflict_classification(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_uppercase();
    matches!(
        normalized.as_str(),
        "AGREES" | "CONTRADICTS" | "REFINES" | "UNRELATED"
    )
    .then_some(normalized)
}

pub fn list_permissions(conn: &Connection, owner_id: i64) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT client_id, permission, scope, granted_by, granted_at FROM client_permissions WHERE owner_id = ?1 ORDER BY client_id, permission, scope",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![owner_id], |row| {
            Ok(json!({"client":row.get::<_,String>(0)?,"permission":row.get::<_,String>(1)?,"scope":row.get::<_,String>(2)?,
                "grantedBy":row.get::<_,String>(3)?,"grantedAt":row.get::<_,String>(4)?}))
        })
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    Ok(rows)
}

fn normalize_client_permission(raw: &str) -> Result<String, String> {
    let permission = raw.trim().to_ascii_lowercase();
    if matches!(permission.as_str(), "read" | "write" | "admin") {
        Ok(permission)
    } else {
        Err(format!(
            "invalid permission `{raw}`; expected read, write, or admin"
        ))
    }
}

pub fn grant_permission(
    conn: &Connection,
    owner_id: i64,
    client: &str,
    permission: &str,
    scope: &str,
    granted_by: &str,
) -> Result<(), String> {
    // The first row closes default-open. Storing `Admin` or `foo` would
    // report success, trip the configured-row gate, and grant nothing.
    let permission = normalize_client_permission(permission)?;
    conn.execute(
        "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by, granted_at)
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
         ON CONFLICT(owner_id, client_id, permission, scope) DO UPDATE SET granted_by = excluded.granted_by, granted_at = excluded.granted_at",
        params![owner_id, client, permission, scope, granted_by],
    )
    .map(|_| ())
    .map_err(|err| err.to_string())
}

pub fn revoke_permission(
    conn: &Connection,
    owner_id: i64,
    client: &str,
    permission: &str,
    scope: &str,
) -> Result<usize, String> {
    let permission = normalize_client_permission(permission)?;
    conn.execute(
        "DELETE FROM client_permissions WHERE owner_id = ?1 AND client_id = ?2 AND lower(permission) = ?3 AND scope = ?4",
        params![owner_id, client, permission, scope],
    )
    .map_err(|err| err.to_string())
}

pub fn list_conflicts_payload(
    _conn: &Connection,
    options: &ConflictListOptions,
) -> Result<Value, String> {
    Ok(
        json!({"statusFilter":options.status.as_str(),"classificationFilter":options.classification,"conflictIdFilter":options.conflict_id,
        "openCount":0,"resolvedCount":0,"count":0,"pairs":[],"conflicts":[],"conflict":Value::Null}),
    )
}

/// Contains-pattern for `LIKE ? ESCAPE '\'`. Keyword `%` `_` `\` must stay
/// literals or `forget` of `100%` matches every memory whose text contains `100`.
fn like_contains(keyword: &str) -> String {
    let mut out = String::from("%");
    for ch in keyword.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('%');
    out
}

pub fn forget_keyword_scoped(
    conn: &mut Connection,
    keyword: &str,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    if keyword.trim().is_empty() {
        return Ok(0);
    }
    let pattern = like_contains(&keyword.to_lowercase());
    let updated = if let Some(owner_id) = owner_id {
        conn.execute(
            "UPDATE memories SET score = score * 0.3 WHERE owner_id = ?2 AND lower(text) LIKE ?1 ESCAPE '\\'",
            params![pattern, owner_id],
        )
    } else {
        conn.execute(
            "UPDATE memories SET score = score * 0.3 WHERE lower(text) LIKE ?1 ESCAPE '\\'",
            params![pattern],
        )
    };
    updated.map_err(|err| err.to_string())
}

pub fn resolve_decision_with_metadata(
    conn: &mut Connection,
    keep_id: i64,
    action: &str,
    superseded_id: Option<i64>,
    _metadata: ResolutionMetadata,
) -> Result<Value, String> {
    if !matches!(action, "keep" | "merge" | "archive") {
        return Err("Invalid action. Expected keep, merge, or archive.".to_string());
    }
    if superseded_id == Some(keep_id) {
        return Err("keep and superseded ids must differ".to_string());
    }
    let status = if action == "archive" {
        "archived"
    } else {
        "active"
    };
    let tx = conn.savepoint().map_err(|err| err.to_string())?;
    let kept = tx
        .execute(
            "UPDATE decisions SET status = ?2, disputes_id = NULL, updated_at = datetime('now') WHERE id = ?1",
            params![keep_id, status],
        )
        .map_err(|err| err.to_string())?;
    if kept == 0 {
        return Err(format!("decision `{keep_id}` was not found"));
    }
    if let Some(other) = superseded_id {
        // keep and merge both retire the loser; archive retires both sides.
        let other_status = if action == "archive" {
            "archived"
        } else {
            "superseded"
        };
        let other_updated = tx
            .execute(
                "UPDATE decisions SET status = ?2, disputes_id = NULL, updated_at = datetime('now') WHERE id = ?1",
                params![other, other_status],
            )
            .map_err(|err| err.to_string())?;
        if other_updated == 0 {
            return Err(format!("decision `{other}` was not found"));
        }
        tx.execute(
            "UPDATE decision_conflicts
             SET status = 'user_resolved',
                 resolution_strategy = ?3,
                 resolved_by = 'user',
                 resolved_at = datetime('now')
             WHERE status = 'open'
               AND (
                 (source_decision_id = ?1 AND target_decision_id = ?2)
                 OR (source_decision_id = ?2 AND target_decision_id = ?1)
               )",
            params![keep_id, other, action],
        )
        .map_err(|err| err.to_string())?;
    } else {
        tx.execute(
            "UPDATE decision_conflicts
             SET status = 'user_resolved',
                 resolution_strategy = ?2,
                 resolved_by = 'user',
                 resolved_at = datetime('now')
             WHERE status = 'open'
               AND (source_decision_id = ?1 OR target_decision_id = ?1)",
            params![keep_id, action],
        )
        .map_err(|err| err.to_string())?;
    }
    tx.commit().map_err(|err| err.to_string())?;
    Ok(
        json!({"resolved":true,"keepId":keep_id,"winnerId":keep_id,"supersededId":superseded_id,"action":action}),
    )
}
