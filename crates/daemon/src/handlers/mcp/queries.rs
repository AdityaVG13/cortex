use cortex_kernel::db::table_has_column;
use cortex_kernel::handlers::recall::{RecallContext, is_visible};
use cortex_kernel::handlers::store::{agent_match_params, same_agent};
use cortex_logic::protocol::{ACTIVE_TEMPORAL_SQL, nonempty_opt, optional_ident_match_sql};
use serde_json::{Value, json};

fn last_call_sql(owner_scoped: bool) -> String {
    let vis = if owner_scoped { "owner_id, visibility" } else { "NULL AS owner_id, NULL AS visibility" };
    format!(
        "SELECT kind, id, created_at, source_agent, summary, detail, owner_id, visibility FROM (SELECT 'memory' AS kind, id, created_at, source_agent, substr(text, 1, 240) AS summary, json_object('text', text, 'source', source, 'type', type) AS detail, {vis} FROM memories WHERE {ACTIVE_TEMPORAL_SQL} UNION ALL SELECT 'decision' AS kind, id, created_at, source_agent, substr(decision, 1, 240) AS summary, json_object('decision', decision, 'context', context, 'type', type) AS detail, {vis} FROM decisions WHERE {ACTIVE_TEMPORAL_SQL} UNION ALL SELECT 'event' AS kind, id, created_at, source_agent, substr(COALESCE(data, type), 1, 240) AS summary, json_object('type', type, 'data', data) AS detail, NULL AS owner_id, NULL AS visibility FROM events) WHERE (?1 = 'any' OR kind = ?1) AND {} AND (?4 IS NULL OR owner_id = ?4 OR visibility IN ('shared', 'team')) ORDER BY julianday(NULLIF(TRIM(created_at), '')) DESC, id DESC LIMIT 32",
        optional_ident_match_sql("COALESCE(source_agent, '')", 2)
    )
}

pub(crate) fn fetch_last_call(conn: &rusqlite::Connection, kind: Option<&str>, agent_filter: Option<&str>, ctx: &RecallContext) -> Result<Value, String> {
    if ctx.team_mode && ctx.caller_id.is_none() {
        return Ok(json!({"found": false}));
    }
    let normalized_kind = nonempty_opt(kind).unwrap_or("any");
    let agent_filter = nonempty_opt(agent_filter);
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
    let sql = last_call_sql(owner_scoped_entries);
    let mut stmt = conn.prepare(&sql).map_err(|err| err.to_string())?;
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
        let (row_kind, id, created_at, source_agent, summary, detail, owner_id, visibility) = row.map_err(|err| err.to_string())?;
        // Missing created_at is display-only. Skipping it hid the newest
        // row when every candidate in the window lacked a timestamp.
        let created_at = created_at.filter(|value| !value.trim().is_empty());
        if let Some(filter) = agent_filter {
            if !same_agent(source_agent.as_deref().unwrap_or(""), filter) {
                continue;
            }
        }
        if !is_visible(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        return Ok(
            json!({"found":true,"kind":row_kind,"id":id,"createdAt":created_at,"sourceAgent":source_agent,"summary":summary,"detail":serde_json::from_str::<Value>(&detail).unwrap_or(Value::String(detail)),}),
        );
    }
    Ok(json!({"found":false}))
}
