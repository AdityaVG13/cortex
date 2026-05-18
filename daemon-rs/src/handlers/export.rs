// SPDX-License-Identifier: MIT
//! Export and import handlers.
//!
//! GET  /export?format=json|sql  -- dump all active memories + decisions
//! POST /import                  -- restore from a JSON export payload

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use serde_json::json;

use super::{ensure_auth_rated, json_response};
use crate::api_types::{ExportFormat, ImportOptions, ImportPayload};
use crate::export_data::{
    DEFAULT_EXPORT_PAGE_LIMIT, MAX_EXPORT_PAGE_LIMIT, export_json_page_value,
    import_payload as import_data,
};
use crate::state::RuntimeState;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<ExportFormat>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub memories_offset: Option<usize>,
    pub decisions_offset: Option<usize>,
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
        ExportFormat::Json => {
            let limit = query
                .limit
                .unwrap_or(DEFAULT_EXPORT_PAGE_LIMIT)
                .clamp(1, MAX_EXPORT_PAGE_LIMIT);
            let offset = query.offset.unwrap_or(0);
            let memories_offset = query.memories_offset.unwrap_or(offset);
            let decisions_offset = query.decisions_offset.unwrap_or(offset);
            json_response(
                StatusCode::OK,
                export_json_page_value(&conn, limit, memories_offset, decisions_offset),
            )
        }
        ExportFormat::Sql => json_response(
            StatusCode::BAD_REQUEST,
            json!({
                "error": "HTTP SQL export is disabled because it requires a full in-memory export; use the CLI export command instead"
            }),
        ),
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

    let mut conn = state.db.lock().await;
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
    match import_data(&mut conn, &payload, &options) {
        Ok(counts) => json_response(
            StatusCode::OK,
            json!({
                "imported": {
                    "memories": counts.memories,
                    "decisions": counts.decisions,
                }
            }),
        ),
        Err(detail) => json_response(
            StatusCode::BAD_REQUEST,
            json!({
                "error": "import failed",
                "detail": detail,
            }),
        ),
    }
}
