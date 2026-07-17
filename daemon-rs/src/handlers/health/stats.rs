use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;
pub async fn handle_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let mut stmt = match conn.prepare("SELECT data, created_at FROM events WHERE type = 'recall_query' ORDER BY created_at ASC") {
        Ok(stmt) => stmt,
        Err(e) => {
            return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error":e.to_string()}));
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
    let mut queries = 0_i64;
    let mut saved = 0_i64;
    let mut spent = 0_i64;
    for (data, _) in &rows {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
            queries += 1;
            saved += value.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
            spent += value.get("spent").and_then(|v| v.as_i64()).unwrap_or(0);
        }
    }
    json_response(StatusCode::OK, json!({"queries":queries,"saved":saved,"spent":spent}))
}
