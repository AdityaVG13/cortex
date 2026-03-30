use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;
use super::{estimate_tokens, json_response, now_iso};

// ─── Query types ─────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct BootQuery {
    pub profile: Option<String>,
    pub agent: Option<String>,
    pub budget: Option<usize>,
}

// ─── GET /boot ───────────────────────────────────────────────────────────────
///
/// Simplified boot handler for Task 4.
/// Returns a basic boot prompt with identity capsule and memory counts.
/// The full compiler (delta capsule, state.md parsing, per-profile budgets)
/// is Task 7.

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
    let _max_tokens = query.budget.unwrap_or(600);

    // Clear served content for this agent on boot
    {
        let mut served = state.served_content.lock().await;
        served.remove(&agent);
    }

    let conn = state.db.lock().await;

    // Count active memories and decisions
    let memory_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let decision_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions WHERE status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Build a basic identity capsule
    let identity_text =
        "User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits.";
    let assembled = format!(
        "## Identity\n{identity_text}\n\n## Stats\nMemories: {memory_count} | Decisions: {decision_count}\n\n## Note\nRust daemon boot — full compiler coming in Task 7."
    );
    let token_estimate = estimate_tokens(&assembled);

    // Record boot event
    let boot_ts = now_iso();
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "agent_boot",
            serde_json::to_string(&json!({"timestamp": boot_ts, "agent": agent.clone()}))
                .unwrap_or_default(),
            agent.clone()
        ],
    );

    // Estimate raw baseline for savings calculation
    let raw_baseline = {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default();
        let state_size = std::fs::metadata(
            std::path::Path::new(&home)
                .join(".claude")
                .join("state.md"),
        )
        .map(|m| m.len() as usize)
        .unwrap_or(0);
        let mem_dir = std::path::Path::new(&home)
            .join(".claude")
            .join("projects")
            .join("C--Users-aditya")
            .join("memory");
        let mem_size: usize = std::fs::read_dir(&mem_dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map(|x| x == "md")
                            .unwrap_or(false)
                    })
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len() as usize)
                    .sum()
            })
            .unwrap_or(0);
        estimate_tokens(&"x".repeat(state_size + mem_size))
    };
    let saved = raw_baseline.saturating_sub(token_estimate);
    let percent = if raw_baseline > 0 {
        (saved * 100) / raw_baseline
    } else {
        0
    };

    // Record savings event
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "boot_savings",
            serde_json::to_string(&json!({
                "agent": agent.clone(),
                "served": token_estimate,
                "baseline": raw_baseline,
                "saved": saved,
                "percent": percent
            }))
            .unwrap_or_default(),
            "rust-daemon"
        ],
    );
    checkpoint_wal_best_effort(&conn);

    state.emit(
        "agent_boot",
        json!({"agent": agent, "profile": profile.clone()}),
    );

    json_response(
        StatusCode::OK,
        json!({
            "bootPrompt": assembled,
            "tokenEstimate": token_estimate,
            "profile": if profile == "full" { "capsules" } else { &profile },
            "capsules": [
                { "name": "identity", "tokens": estimate_tokens(identity_text), "freshness": "stable", "truncated": false }
            ],
            "savings": {
                "rawBaseline": raw_baseline,
                "served": token_estimate,
                "saved": saved,
                "percent": percent
            }
        }),
    )
}
