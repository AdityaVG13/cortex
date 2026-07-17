// SPDX-License-Identifier: MIT
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use chrono::Utc;
use rusqlite::{params, OpenFlags};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use crate::handlers::{client_ip, ensure_auth_rated, ensure_ssrf_protection, json_response, truncate_chars};
use crate::state::RuntimeState;


use super::*;
pub async fn handle_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }

    let conn = state.db_read.lock().await;
    let mut stmt = match conn.prepare(
        "SELECT data, created_at FROM events WHERE type = 'recall_query' ORDER BY created_at ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": e.to_string() }),
            );
        }
    };

    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            let data_str: String = row.get(0)?;
            let created_at: Option<String> = row.get(1)?;
            Ok((data_str, created_at.unwrap_or_default()))
        })
        .map(|iter| iter.filter_map(|row| row.ok()).collect())
        .unwrap_or_default();

    json_response(StatusCode::OK, build_recall_stats_payload_from_rows(&rows))
}

