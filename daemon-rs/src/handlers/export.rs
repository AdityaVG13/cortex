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

use super::{ensure_auth, json_error, json_response};
use crate::export_data::{
    export_json_value, export_sql_text, import_payload as import_data, ExportFormat, ImportOptions,
    ImportPayload,
};
use crate::state::RuntimeState;
use axum::response::IntoResponse;

#[derive(Deserialize)]
pub struct ExportQuery {
    pub format: Option<String>,
}

pub async fn handle_export(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }

    let format = query.format.as_deref().unwrap_or("json");
    let conn = state.db.lock().await;

    match ExportFormat::parse(format) {
        Some(ExportFormat::Json) => json_response(StatusCode::OK, export_json_value(&conn)),
        Some(ExportFormat::Sql) => {
            let body = export_sql_text(&conn);
            let mut resp = (StatusCode::OK, body).into_response();
            if let Ok(v) = "text/plain; charset=utf-8".parse() {
                resp.headers_mut().insert("content-type", v);
            }
            resp
        }
        None => json_error(
            StatusCode::BAD_REQUEST,
            "Unsupported format: use json or sql",
        ),
    }
}

pub async fn handle_import(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ImportPayload>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
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
