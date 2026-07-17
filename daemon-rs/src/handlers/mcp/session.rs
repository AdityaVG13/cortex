use super::{mcp_session_description, mcp_session_owner_id, normalize_mcp_agent_label};
use crate::handlers::now_iso;
use crate::state::RuntimeState;
use chrono::{Duration, Utc};
pub(crate) async fn upsert_mcp_session(
    state: &RuntimeState,
    caller_id: Option<i64>,
    raw_agent: &str,
    model: Option<&str>,
    description_prefix: &str,
) -> Result<(String, String), String> {
    let agent = normalize_mcp_agent_label(raw_agent, model)?;
    let owner_id = mcp_session_owner_id(state, caller_id)?;
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let description = mcp_session_description(description_prefix, model);
    let conn = state.db.lock().await;
    if let Some(owner_id) = owner_id {
        conn.execute(
"INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, ?3, 'mcp', '[]', ?4, ?5, ?5, ?6)
             ON CONFLICT(owner_id, agent) DO UPDATE SET
               description = CASE
                   WHEN sessions.description IS NULL OR trim(sessions.description) = '' THEN excluded.description
                   ELSE sessions.description
               END,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at"
,rusqlite::params![agent,owner_id,session_id,description,now,expires_at],).map_err(|e|e.to_string())?;
    } else {
        conn.execute(
"INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
             VALUES (?1, ?2, 'mcp', '[]', ?3, ?4, ?4, ?5)
             ON CONFLICT(agent) DO UPDATE SET
               description = CASE
                   WHEN sessions.description IS NULL OR trim(sessions.description) = '' THEN excluded.description
                   ELSE sessions.description
               END,
               project = excluded.project,
               files_json = excluded.files_json,
               last_heartbeat = excluded.last_heartbeat,
               expires_at = excluded.expires_at"
,rusqlite::params![agent,session_id,description,now,expires_at],).map_err(|e|e.to_string())?;
    }
    crate::db::checkpoint_wal_best_effort(&conn);
    Ok((agent, expires_at))
}
