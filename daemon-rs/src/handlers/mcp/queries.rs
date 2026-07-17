use crate::handlers::recall::RecallContext;
use crate::state::RuntimeState;
use serde_json::{json, Value};
pub(crate) fn recall_owner_scope(ctx: &RecallContext) -> String {
    if !ctx.team_mode {
        return "solo".to_string();
    }
    match ctx.caller_id {
        Some(owner_id) => format!("team:{owner_id}"),
        None => "team:none".to_string(),
    }
}
pub(crate) async fn clear_served_scope_for_boot(state: &RuntimeState, agent: &str, ctx: &RecallContext) {
    let scope_prefix = format!("{}::{agent}::", recall_owner_scope(ctx));
    let mut served = state.served_content.lock().await;
    served.retain(|key, _| !key.starts_with(&scope_prefix) && !key.starts_with(&format!("{agent}::")) && key != agent);
}
pub(crate) fn can_view_last_call(owner_id: Option<i64>, visibility: Option<&str>, ctx: &RecallContext) -> bool {
    if !ctx.team_mode {
        return true;
    }
    let Some(caller_id) = ctx.caller_id else {
        return false;
    };
    let Some(owner_id) = owner_id else {
        return false;
    };
    owner_id == caller_id || matches!(visibility, Some("shared") | Some("team"))
}
pub(crate) fn table_has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = match conn.prepare(&pragma) {
        Ok(stmt) => stmt,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    let found = rows.flatten().any(|name| name == column);
    drop(stmt);
    found
}
pub(crate) fn fetch_last_call(
    conn: &rusqlite::Connection, kind: Option<&str>, agent_filter: Option<&str>, ctx: &RecallContext,
) -> Result<Value, String> {
    let normalized_kind = kind.map(str::trim).filter(|value| !value.is_empty()).unwrap_or("any");
    let agent_filter = agent_filter.map(str::trim).filter(|value| !value.is_empty()).map(str::to_lowercase);
    let owner_scoped_entries = table_has_column(conn, "memories", "owner_id")
        && table_has_column(conn, "memories", "visibility")
        && table_has_column(conn, "decisions", "owner_id")
        && table_has_column(conn, "decisions", "visibility");
    let sql = if owner_scoped_entries {
        "
            SELECT kind, id, created_at, source_agent, summary, detail, owner_id, visibility
            FROM (
              SELECT 'memory' AS kind, id, created_at, source_agent,
                     substr(text, 1, 240) AS summary,
                     json_object('text', text, 'source', source, 'type', type) AS detail,
                     owner_id, visibility
              FROM memories
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     owner_id, visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
            ORDER BY CAST(strftime('%s', created_at) AS INTEGER) DESC, id DESC
            LIMIT 32
        "
    } else {
        "
            SELECT kind, id, created_at, source_agent, summary, detail, owner_id, visibility
            FROM (
              SELECT 'memory' AS kind, id, created_at, source_agent,
                     substr(text, 1, 240) AS summary,
                     json_object('text', text, 'source', source, 'type', type) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM memories
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR expires_at > datetime('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
            ORDER BY CAST(strftime('%s', created_at) AS INTEGER) DESC, id DESC
            LIMIT 32
        "
    };
    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![normalized_kind], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|err| err.to_string())?;
    for row in rows.flatten() {
        let (row_kind, id, created_at, source_agent, summary, detail, owner_id, visibility) = row;
        if let Some(filter) = agent_filter.as_deref() {
            let current = source_agent.as_deref().map(str::to_lowercase).unwrap_or_default();
            if current != filter {
                continue;
            }
        }
        if row_kind != "event" && !can_view_last_call(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        return Ok(json!({"found":true,"kind":row_kind
,"id":id,"createdAt":created_at,"sourceAgent":source_agent,"summary":summary,"detail":serde_json::from_str::<Value>(&detail).
unwrap_or(Value::String(detail)),}));
    }
    Ok(json!({"found":false}))
}
