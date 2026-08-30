use super::arg_str;
use crate::handlers::SourceIdentity;
use crate::state::RuntimeState;
use rusqlite::OptionalExtension;
use serde_json::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientPermission {
    Read,
    Write,
    Admin,
}
impl ClientPermission {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ClientPermission::Read => "read",
            ClientPermission::Write => "write",
            ClientPermission::Admin => "admin",
        }
    }
}
pub(crate) fn parse_client_permission(raw: &str) -> Option<ClientPermission> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "read" => Some(ClientPermission::Read),
        "write" => Some(ClientPermission::Write),
        "admin" => Some(ClientPermission::Admin),
        _ => None,
    }
}
pub(crate) fn required_permission_for_tool(tool_name: &str) -> Option<ClientPermission> {
    match tool_name {
        "cortex_boot"
        | "cortex_boot_audit"
        | "cortex_reconnect"
        | "cortex_peek"
        | "cortex_recall"
        | "cortex_recall_policy_explain"
        | "cortex_semantic_recall"
        | "cortex_agent_feedback_stats"
        | "cortex_health"
        | "cortex_digest"
        | "cortex_unfold"
        | "cortex_focus_status"
        | "cortex_lastCall" => Some(ClientPermission::Read),
        "cortex_store" | "cortex_agent_feedback_record" | "cortex_focus_start" | "cortex_focus_end" | "cortex_diary" => Some(ClientPermission::Write),
        "cortex_forget"
        | "cortex_resolve"
        | "cortex_conflicts_list"
        | "cortex_conflicts_get"
        | "cortex_conflicts_resolve"
        | "cortex_permissions_list"
        | "cortex_permissions_grant"
        | "cortex_permissions_revoke"
        | "cortex_consensus_promote"
        | "cortex_memory_decay_run"
        | "cortex_eval_run" => Some(ClientPermission::Admin),
        _ => None,
    }
}
pub(crate) fn normalize_permission_client_id(raw: &str) -> String {
    let before_model = raw.split('(').next().unwrap_or(raw).trim().to_ascii_lowercase();
    let normalized: String = before_model.chars().filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_').collect();
    if normalized.is_empty() {
        "mcp".to_string()
    } else {
        normalized
    }
}
pub(crate) fn source_client_for_permissions(source: Option<&SourceIdentity>, args: &Value) -> String {
    let raw = source.map(|identity| identity.agent.as_str()).or_else(|| arg_str(args, &["source_agent", "agent"])).unwrap_or("mcp");
    normalize_permission_client_id(raw)
}
pub(crate) fn permission_satisfies(granted: &str, required: ClientPermission) -> bool {
    match required {
        ClientPermission::Read => matches!(granted, "read" | "write" | "admin"),
        ClientPermission::Write => matches!(granted, "write" | "admin"),
        ClientPermission::Admin => granted == "admin",
    }
}
pub(crate) fn has_client_permission(
    conn: &rusqlite::Connection, owner_id: i64, client_id: &str, scope: &str, required: ClientPermission,
) -> Result<bool, String> {
    let configured_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM client_permissions WHERE owner_id = ?1", rusqlite::params![owner_id], |row| row.get(0))
        .map_err(|err| err.to_string())?;
    if configured_rows == 0 {
        return Ok(true);
    }
    let mut stmt = conn
        .prepare(
            "SELECT permission FROM client_permissions
             WHERE owner_id = ?1
               AND (client_id = ?2 OR client_id = '*')
               AND (scope = ?3 OR scope = '*')",
        )
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![owner_id, client_id, scope], |row| row.get::<_, String>(0))
        .map_err(|err| err.to_string())?;
    for granted in rows.flatten() {
        if permission_satisfies(granted.trim(), required) {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(crate) fn caller_has_team_admin_role(conn: &rusqlite::Connection, caller_id: i64) -> Result<bool, String> {
    let role = conn
        .query_row("SELECT role FROM users WHERE id = ?1", rusqlite::params![caller_id], |row| row.get::<_, String>(0))
        .optional()
        .map_err(|err| err.to_string())?;
    Ok(matches!(role.as_deref(), Some("owner" | "admin")))
}
pub(crate) async fn enforce_client_permission(
    state: &RuntimeState, caller_id: Option<i64>, tool_name: &str, args: &Value, source: Option<&SourceIdentity>,
) -> Result<(), String> {
    let Some(required) = required_permission_for_tool(tool_name) else {
        return Ok(());
    };
    let owner_id = if state.team_mode { caller_id.unwrap_or_default() } else { 0 };
    let client_id = source_client_for_permissions(source, args);
    let conn = state.db_read.lock().await;
    if state.team_mode && required == ClientPermission::Admin && !caller_has_team_admin_role(&conn, owner_id)? {
        return Err(format!("Permission denied: team admin role required for '{tool_name}'"));
    }
    let allowed = has_client_permission(&conn, owner_id, &client_id, tool_name, required)?;
    drop(conn);
    if allowed {
        return Ok(());
    }
    Err(format!("Permission denied: client '{client_id}' lacks '{}' permission for '{tool_name}'", required.as_str()))
}
