use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use crate::compiler;
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;
use super::{json_response, now_iso};

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
    let agent = query
        .agent
        .or_else(|| {
            headers
                .get("x-source-agent")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let profile = query.profile.unwrap_or_else(|| "full".to_string());
    let max_tokens = query.budget.unwrap_or(600);

    // Clear served content for this agent on boot
    {
        let mut served = state.served_content.lock().await;
        served.remove(&agent);
    }

    let conn = state.db.lock().await;

    // Clean expired conductor state before compiling
    let _ = clean_expired_locks(&conn);
    let _ = clean_expired_sessions(&conn);

    // Compile the boot prompt using the full capsule compiler
    let result = compiler::compile(&conn, &state.home, &agent, max_tokens);

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
    conn.execute(
        "DELETE FROM locks WHERE expires_at < ?1",
        rusqlite::params![now_iso()],
    )?;
    Ok(())
}

fn clean_expired_sessions(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM sessions WHERE expires_at < ?1",
        rusqlite::params![now_iso()],
    )?;
    Ok(())
}
