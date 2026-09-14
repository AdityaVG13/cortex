use cortex_kernel::handlers::recall::RecallContext;
use cortex_kernel::handlers::store::{agent_match_params, same_agent};
use serde_json::{json, Value};
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
pub(crate) fn fetch_last_call(conn: &rusqlite::Connection, kind: Option<&str>, agent_filter: Option<&str>, ctx: &RecallContext) -> Result<Value, String> {
    if ctx.team_mode && ctx.caller_id.is_none() {
        return Ok(json!({"found": false}));
    }
    let normalized_kind = kind.map(str::trim).filter(|value| !value.is_empty()).unwrap_or("any");
    let agent_filter = agent_filter.map(str::trim).filter(|value| !value.is_empty());
    // Filter in SQL. A LIMIT-then-filter window hid the matching row when
    // 32 newer other-agent (or unowned event) rows came first.
    let agent_params = agent_filter.and_then(agent_match_params);
    let (agent_ident, agent_like) = match agent_params.as_ref() {
        Some((ident, like)) => (Some(ident.as_str()), Some(like.as_str())),
        None if agent_filter.is_some() => (Some("\u{0}"), Some("\u{0} (%")),
        None => (None, None),
    };
    let team_owner = if ctx.team_mode { ctx.caller_id } else { None };
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
              WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     owner_id, visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
              AND (?2 IS NULL OR lower(trim(COALESCE(source_agent, ''))) = ?2
                   OR lower(trim(COALESCE(source_agent, ''))) LIKE ?3 ESCAPE '\\')
              AND (?4 IS NULL OR owner_id = ?4 OR visibility IN ('shared', 'team'))
            ORDER BY julianday(NULLIF(TRIM(created_at), '')) DESC, id DESC
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
              WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))
              UNION ALL
              SELECT 'decision' AS kind, id, created_at, source_agent,
                     substr(decision, 1, 240) AS summary,
                     json_object('decision', decision, 'context', context, 'type', type) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM decisions
              WHERE status = 'active' AND (expires_at IS NULL OR TRIM(expires_at) = '' OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR TRIM(valid_from) = '' OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR TRIM(valid_until) = '' OR julianday(valid_until) > julianday('now'))
              UNION ALL
              SELECT 'event' AS kind, id, created_at, source_agent,
                     substr(COALESCE(data, type), 1, 240) AS summary,
                     json_object('type', type, 'data', data) AS detail,
                     NULL AS owner_id, NULL AS visibility
              FROM events
            )
            WHERE (?1 = 'any' OR kind = ?1)
              AND (?2 IS NULL OR lower(trim(COALESCE(source_agent, ''))) = ?2
                   OR lower(trim(COALESCE(source_agent, ''))) LIKE ?3 ESCAPE '\\')
              AND (?4 IS NULL OR owner_id = ?4 OR visibility IN ('shared', 'team'))
            ORDER BY julianday(NULLIF(TRIM(created_at), '')) DESC, id DESC
            LIMIT 32
        "
    };
    let mut stmt = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![normalized_kind, agent_ident, agent_like, team_owner], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|err| err.to_string())?;
    for row in rows {
        let (row_kind, id, created_at, source_agent, summary, detail, owner_id, visibility) =
            row.map_err(|err| err.to_string())?;
        // Missing created_at is display-only. Skipping it hid the newest
        // row when every candidate in the window lacked a timestamp.
        let created_at = created_at.filter(|value| !value.trim().is_empty());
        if let Some(filter) = agent_filter {
            if !same_agent(source_agent.as_deref().unwrap_or(""), filter) {
                continue;
            }
        }
        if !can_view_last_call(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        return Ok(json!({"found":true,"kind":row_kind
,"id":id,"createdAt":created_at,"sourceAgent":source_agent,"summary":summary,"detail":serde_json::from_str::<Value>(&detail).
unwrap_or(Value::String(detail)),}));
    }
    Ok(json!({"found":false}))
}
