use crate::state::RuntimeState;
use chrono::{Duration, Utc};
use cortex_kernel::handlers::now_iso;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub agent: String,
    pub model: Option<String>,
}

/// Register a session from an explicit, transport-independent identity.
pub async fn register_agent_presence(
    cx: &asupersync::Cx, state: &RuntimeState, source: &SourceIdentity, caller_id: Option<i64>, project: &str, description_prefix: &str,
) -> Result<(), String> {
    let owner_id = if state.team_mode { caller_id.or(state.default_owner_id) } else { None };
    let conn = state.db.lock(cx).await.map_err(|err| err.to_string())?;
    let now = now_iso();
    let expires_at = (Utc::now() + Duration::hours(2)).to_rfc3339();
    let session_id = format!("session-{}", uuid::Uuid::new_v4());
    let description = source
        .model
        .as_deref()
        .map(|model| format!("{description_prefix} · {model}"))
        .unwrap_or_else(|| description_prefix.to_string());
    if let Some(owner_id) = owner_id {
        conn.execute("INSERT INTO sessions (agent, owner_id, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?6, ?7) ON CONFLICT(owner_id, agent) DO UPDATE SET description = excluded.description, project = excluded.project, files_json = excluded.files_json, last_heartbeat = excluded.last_heartbeat, expires_at = excluded.expires_at", rusqlite::params![source.agent.as_str(), owner_id, session_id, project, description, now, expires_at]).map_err(|err| err.to_string())?;
    } else {
        conn.execute("INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) VALUES (?1, ?2, ?3, '[]', ?4, ?5, ?5, ?6) ON CONFLICT(agent) DO UPDATE SET description = excluded.description, project = excluded.project, files_json = excluded.files_json, last_heartbeat = excluded.last_heartbeat, expires_at = excluded.expires_at", rusqlite::params![source.agent.as_str(), session_id, project, description, now, expires_at]).map_err(|err| err.to_string())?;
    }
    Ok(())
}
