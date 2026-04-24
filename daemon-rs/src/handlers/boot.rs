// SPDX-License-Identifier: MIT
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;
use std::time::Instant;

use super::{ensure_auth_with_caller_rated, json_response, now_iso};
use crate::compiler;
use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;

/// C5 — default retention window for `boot_audits`. Rows older than this
/// are pruned at the start of each boot call. Override via the
/// `CORTEX_BOOT_AUDIT_RETENTION_DAYS` env var (0 disables prune).
///
/// 90 days matches audit-trail norms (longer than the 30-day draft spec
/// because an audit log that self-deletes too aggressively defeats the
/// purpose -- debugging "why did boot capsule X appear three weeks ago"
/// breaks at 30). Rows are small metadata only; storage impact at
/// typical 10-boots/day is well under 1 MB/year even without compression.
const BOOT_AUDIT_RETENTION_DAYS_DEFAULT: i64 = 90;

fn boot_audit_retention_days() -> i64 {
    std::env::var("CORTEX_BOOT_AUDIT_RETENTION_DAYS")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&v| v >= 0)
        .unwrap_or(BOOT_AUDIT_RETENTION_DAYS_DEFAULT)
}

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
    super::register_agent_presence(&state, &source, caller_id, "http", "HTTP boot session").await;
    let agent = source.agent;

    let profile = query.profile.unwrap_or_else(|| "full".to_string());
    let max_tokens = query.budget.unwrap_or(600);
    let boot_started = Instant::now();

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

    // C5 — boot audit trail. Record one row per /boot call + prune anything
    // older than BOOT_AUDIT_RETENTION_DAYS. Failures are logged but never
    // block the boot response; audit rows are diagnostic, not critical-path.
    let latency_ms = boot_started.elapsed().as_millis() as i64;
    let token_savings = result
        .savings
        .get("saved")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let capsules_count = result.capsules.len() as i64;
    let capsules_json = serde_json::to_string(&result.capsules)
        .unwrap_or_else(|_| "[]".to_string());
    let retention_days = boot_audit_retention_days();
    if retention_days > 0 {
        if let Err(e) = conn.execute(
            &format!(
                "DELETE FROM boot_audits WHERE created_at < datetime('now', '-{retention_days} days')"
            ),
            [],
        ) {
            eprintln!("[boot_audits] prune failed: {e}");
        }
    }
    if let Err(e) = conn.execute(
        "INSERT INTO boot_audits (agent, profile, budget_tokens, token_estimate,
                                  token_savings, capsules_count, capsules_json, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            agent,
            profile,
            max_tokens as i64,
            result.token_estimate as i64,
            token_savings,
            capsules_count,
            capsules_json,
            latency_ms,
        ],
    ) {
        eprintln!("[boot_audits] insert failed: {e}");
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
            "savings": result.savings,
            "tokenUsage": {
                "used": result.token_estimate,
                "saved": result.savings.get("saved").and_then(|value| value.as_i64()).unwrap_or(0),
                "budget": max_tokens
            },
            "tokenUsageLine": format!(
                "Token usage: used {} tokens, saved {} of {} during boot compile.",
                result.token_estimate,
                result.savings.get("saved").and_then(|value| value.as_i64()).unwrap_or(0),
                max_tokens
            )
        }),
    )
}

// ─── GET /boot/audit ─────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct BootAuditQuery {
    pub agent: Option<String>,
    pub limit: Option<usize>,
}

/// Returns the most recent `boot_audits` rows, newest first. Optional
/// `agent` filter narrows to one agent; `limit` caps the returned rows
/// (default 50, ceiling 500).
pub async fn handle_boot_audit(
    State(state): State<RuntimeState>,
    Query(query): Query<BootAuditQuery>,
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

    let limit = query.limit.unwrap_or(50).min(500);
    let conn = state.db.lock().await;

    let rows_result: Result<Vec<serde_json::Value>, rusqlite::Error> = match &query.agent {
        Some(agent) => conn
            .prepare(
                "SELECT id, agent, profile, budget_tokens, token_estimate,
                        token_savings, capsules_count, latency_ms, created_at
                 FROM boot_audits
                 WHERE agent = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![agent, limit as i64], row_to_json)?
                    .collect()
            }),
        None => conn
            .prepare(
                "SELECT id, agent, profile, budget_tokens, token_estimate,
                        token_savings, capsules_count, latency_ms, created_at
                 FROM boot_audits
                 ORDER BY id DESC
                 LIMIT ?1",
            )
            .and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![limit as i64], row_to_json)?
                    .collect()
            }),
    };

    match rows_result {
        Ok(rows) => json_response(
            StatusCode::OK,
            json!({
                "audits": rows,
                "count": rows.len(),
                "retention_days": boot_audit_retention_days(),
            }),
        ),
        Err(e) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("boot_audits query failed: {e}") }),
        ),
    }
}

fn row_to_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({
        "id":             row.get::<_, i64>(0)?,
        "agent":          row.get::<_, String>(1)?,
        "profile":        row.get::<_, String>(2)?,
        "budget_tokens":  row.get::<_, i64>(3)?,
        "token_estimate": row.get::<_, i64>(4)?,
        "token_savings":  row.get::<_, i64>(5)?,
        "capsules_count": row.get::<_, i64>(6)?,
        "latency_ms":     row.get::<_, Option<i64>>(7)?,
        "created_at":     row.get::<_, String>(8)?,
    }))
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
