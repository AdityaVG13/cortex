// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use super::{ensure_auth_with_caller_rated, json_response, now_iso};
use crate::compiler;
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

// ─── Query types ─────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct BootQuery {
    pub profile: Option<String>,
    pub agent: Option<String>,
    pub budget: Option<usize>,
}

// ─── GET /boot ───────────────────────────────────────────────────────────────

pub async fn handle_boot(
    State(state): State<RuntimeState>,
    Query(query): Query<BootQuery>,
    headers: HeaderMap,
) -> Response {
    let caller_id = match ensure_auth_with_caller_rated(&headers, &state).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    if state.team_mode && caller_id.is_none() {
        return json_response(
            StatusCode::FORBIDDEN,
            json!({ "error": "Team mode requires a caller-scoped ctx_ API key" }),
        );
    }
    let source = super::resolve_source_identity(&headers, query.agent.as_deref().unwrap_or("mcp"));
    let agent = source.agent;
    super::register_agent_presence_from_headers(&state, &headers, caller_id).await;

    let profile = query.profile.unwrap_or_else(|| "full".to_string());
    let max_tokens = query.budget.unwrap_or(600);

    // Clear served content for this agent on boot
    {
        let mut served = state.served_content.lock().await;
        let scope_prefix = if state.team_mode {
            match caller_id {
                Some(owner_id) => format!("team:{owner_id}::{agent}::"),
                None => format!("team:none::{agent}::"),
            }
        } else {
            format!("solo::{agent}::")
        };
        // Clear current scoped keys plus legacy pre-scope keys.
        served.retain(|key, _| {
            !key.starts_with(&scope_prefix)
                && !key.starts_with(&format!("{agent}::"))
                && key != &agent
        });
    }

    let conn = state.db.lock().await;

    // Clean expired conductor state before compiling
    let _ = clean_expired_locks(&conn);
    let _ = clean_expired_sessions(&conn);

    // Compile the boot prompt using the full capsule compiler
    let result = compiler::compile(&conn, &state.home, &agent, max_tokens);

    // Auto-ack feed on boot: advance last_seen_id to the latest feed entry.
    if let Ok(latest_id) = conn.query_row(
        "SELECT id FROM feed ORDER BY timestamp DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    ) {
        let feed_ack_owner = if state.team_mode { caller_id } else { None };
        if let Some(owner_id) = feed_ack_owner {
            let _ = conn.execute(
                "INSERT INTO feed_acks (owner_id, agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3, datetime('now')) \
                 ON CONFLICT(owner_id, agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
                rusqlite::params![owner_id, agent, latest_id],
            );
        } else {
            let _ = conn.execute(
                "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, datetime('now')) \
                 ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
                rusqlite::params![agent, latest_id],
            );
        }
    }

    checkpoint_wal_best_effort(&conn);

    state.emit(
        "agent_boot",
        json!({"agent": agent, "profile": profile.clone()}),
    );

    json_response(
        StatusCode::OK,
        json!({
            "bootPrompt": result.boot_prompt,
            "tokenEstimate": result.token_estimate,
            "profile": if profile == "full" { "capsules" } else { &profile },
            "capsules": result.capsules,
            "savings": result.savings
        }),
    )
}

// ─── Cleanup helpers (shared with conductor but needed before compile) ──────

fn clean_expired_locks(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    if let Some(owner_id) = current_owner_id(conn) {
        conn.execute(
            "DELETE FROM locks WHERE owner_id = ?1 AND expires_at < ?2",
            rusqlite::params![owner_id, now_iso()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM locks WHERE expires_at < ?1",
            rusqlite::params![now_iso()],
        )?;
    }
    Ok(())
}

fn clean_expired_sessions(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    if let Some(owner_id) = current_owner_id(conn) {
        conn.execute(
            "DELETE FROM sessions WHERE owner_id = ?1 AND expires_at < ?2",
            rusqlite::params![owner_id, now_iso()],
        )?;
    } else {
        conn.execute(
            "DELETE FROM sessions WHERE expires_at < ?1",
            rusqlite::params![now_iso()],
        )?;
    }
    Ok(())
}

fn current_owner_id(conn: &rusqlite::Connection) -> Option<i64> {
    conn.query_row(
        "SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<i64>().ok())
}
