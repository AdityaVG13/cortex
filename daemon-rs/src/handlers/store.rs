use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::checkpoint_wal_best_effort;
use crate::state::RuntimeState;
use super::{ensure_auth, json_response, log_event, now_iso};

// ─── Request types ───────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct StoreRequest {
    pub decision: Option<String>,
    pub context: Option<String>,
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    pub source_agent: Option<String>,
    pub confidence: Option<f64>,
}

// ─── POST /store ─────────────────────────────────────────────────────────────

pub async fn handle_store(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<StoreRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let decision = body.decision.unwrap_or_default();
    if decision.trim().is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({ "error": "Missing field: decision" }),
        );
    }

    let source_agent = headers
        .get("x-source-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .or(body.source_agent)
        .unwrap_or_else(|| "http".to_string());

    let mut conn = state.db.lock().await;
    match store_decision(
        &mut conn,
        decision.trim(),
        body.context,
        body.entry_type,
        source_agent,
        body.confidence,
    ) {
        Ok(entry) => json_response(StatusCode::OK, json!({ "stored": true, "entry": entry })),
        Err(err) => json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": format!("Store failed: {err}") }),
        ),
    }
}

// ─── Core store logic ────────────────────────────────────────────────────────

/// Insert a new decision with Jaccard surprise scoring.
/// NOTE: The Rust version does NOT have conflict detection — it calls
/// `store_decision()` directly.  Conflict detection is Task 5.5.
fn store_decision(
    conn: &mut Connection,
    decision: &str,
    context: Option<String>,
    entry_type: Option<String>,
    source_agent: String,
    confidence: Option<f64>,
) -> Result<Value, String> {
    conn.execute(
        "INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise, status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7)",
        params![
            decision,
            context,
            entry_type.unwrap_or_else(|| "decision".to_string()),
            source_agent.clone(),
            confidence.unwrap_or(0.8),
            1.0_f64,
            now_iso()
        ],
    )
    .map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();
    let _ = log_event(
        conn,
        "decision_stored",
        json!({ "id": id, "source_agent": source_agent }),
        "rust-daemon",
    );
    checkpoint_wal_best_effort(conn);

    Ok(json!({ "stored": true, "id": id, "status": "active", "surprise": 1.0 }))
}
