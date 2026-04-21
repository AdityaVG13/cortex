// SPDX-License-Identifier: MIT
//! Export and import handlers.
//!
//! GET  /export?format=json|sql  -- dump all active memories + decisions
//! POST /import                  -- restore from a JSON export payload

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::{ensure_auth_rated, json_response};
use crate::api_types::{ExportFormat, ImportOptions, ImportPayload};
use crate::export_data::{export_json_value, export_sql_text, import_payload as import_data};
use crate::state::RuntimeState;
use axum::response::IntoResponse;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<ExportFormat>,
}

pub async fn handle_export(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let conn = state.db_read.lock().await;

    match query.format.unwrap_or(ExportFormat::Json) {
        ExportFormat::Json => json_response(StatusCode::OK, export_json_value(&conn)),
        ExportFormat::Sql => {
            let body = export_sql_text(&conn);
            let mut resp = (StatusCode::OK, body).into_response();
            if let Ok(v) = "text/plain; charset=utf-8".parse() {
                resp.headers_mut().insert("content-type", v);
            }
            resp
        }
    }
}

pub async fn handle_import(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ImportPayload>,
) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let conn = state.db.lock().await;
    let options = if state.team_mode {
        ImportOptions {
            owner_id: state.default_owner_id,
            visibility: Some("private".to_string()),
            source_agent_fallback: "import-http".to_string(),
        }
    } else {
        ImportOptions {
            source_agent_fallback: "import-http".to_string(),
            ..ImportOptions::default()
        }
    };
    let counts = import_data(&conn, &payload, &options);

    json_response(
        StatusCode::OK,
        json!({
            "imported": {
                "memories": counts.memories,
                "decisions": counts.decisions,
            }
        }),
    )
}
