use super::arg_str;
use crate::handlers::SourceIdentity;
use crate::state::RuntimeState;
use cortex_kernel::handlers::store::normalize_client_slug;
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
pub(crate) fn required_permission_for_tool(tool_name: &str) -> Option<ClientPermission> {
    // Only names with a real dispatcher path. Removed historical tools
    // (diary, forget, reconnect, boot_audit, …) return None and surface as
    // UNKNOWN_TOOL instead of a half-advertised "recognised" dead end.
    match tool_name {
        // Canonical eight operations
        "cortex_capabilities" | "cortex_orient" | "cortex_query" | "cortex_expand" => Some(ClientPermission::Read),
        "cortex_commit" | "cortex_checkpoint" | "cortex_feedback" => Some(ClientPermission::Write),
        "cortex_resolve" => Some(ClientPermission::Admin),
        // Working aliases of the eight
        "cortex_boot" | "cortex_unfold" => Some(ClientPermission::Read),
        "cortex_store" | "cortex_focus_start" | "cortex_focus_end" => Some(ClientPermission::Write),
        "cortex_conflicts_resolve" => Some(ClientPermission::Admin),
        // Specialised legacy surfaces that still dispatch
        "cortex_recall" | "cortex_peek" | "cortex_semantic_recall" | "cortex_health" | "cortex_digest" | "cortex_lastCall" | "cortex_agent_feedback_stats" => {
            Some(ClientPermission::Read)
        }
        "cortex_agent_feedback_record" => Some(ClientPermission::Write),
        "cortex_permissions_list" => Some(ClientPermission::Read),
        "cortex_permissions_grant" | "cortex_permissions_revoke" => Some(ClientPermission::Admin),
        _ => None,
    }
}
pub(crate) fn normalize_permission_client_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed == "*" {
        return "*".to_string();
    }
    normalize_client_slug(trimmed, "mcp")
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
    let configured_rows = crate::db::count_sql(conn, "SELECT COUNT(*) FROM client_permissions WHERE owner_id = ?1", rusqlite::params![owner_id])?;
    if configured_rows == 0 {
        return Ok(true);
    }
    let mut stmt = conn
        .prepare("SELECT client_id, permission FROM client_permissions WHERE owner_id = ?1 AND (scope = ?2 OR scope = '*')")
        .map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![owner_id, scope], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|err| err.to_string())?;
    for row in rows {
        let (stored_client, granted) = row.map_err(|err| err.to_string())?;
        let stored_norm = normalize_permission_client_id(&stored_client);
        if stored_client != "*" && stored_norm != client_id && stored_client != client_id {
            continue;
        }
        let granted = granted.trim().to_ascii_lowercase();
        if permission_satisfies(&granted, required) {
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
    cx: &asupersync::Cx, state: &RuntimeState, caller_id: Option<i64>, tool_name: &str, args: &Value, source: Option<&SourceIdentity>,
) -> Result<(), String> {
    let Some(required) = required_permission_for_tool(tool_name) else {
        return Ok(());
    };
    let owner_id = if state.team_mode { caller_id.unwrap_or_default() } else { 0 };
    let client_id = source_client_for_permissions(source, args);
    let conn = state.db_read.lock(cx).await.map_err(|err| err.to_string())?;
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
