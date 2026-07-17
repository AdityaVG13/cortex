use crate::embeddings;
use crate::handlers::{ensure_auth_rated, json_error, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};
const MAX_BOOST: f64 = 0.3;
const MIN_BOOST: f64 = -0.2;
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;
pub const IMMUNITY_THRESHOLD: i64 = 5;
pub const IMMUNITY_WINDOW_DAYS: i64 = 14;
#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub query: Option<String>,
    pub sources: Vec<String>,
    pub signal: Option<f64>,
    pub agent: Option<String>,
}
pub async fn handle_feedback(State(state): State<RuntimeState>, headers: HeaderMap, Json(body): Json<FeedbackRequest>) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    if body.sources.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "sources array is empty");
    }
    let signal = body.signal.unwrap_or(1.0).clamp(-1.0, 1.0);
    let agent = body.agent.as_deref().unwrap_or("http");
    let query_text = body.query.as_deref().unwrap_or("");
    let query_embedding = match state.embedding_engine.clone() {
        Some(engine) => engine.embed_query_async(query_text.to_string()).await.map(|v| embeddings::vector_to_blob(&v)),
        None => None,
    };
    let conn = state.db.lock().await;
    let mut stored = 0usize;
    for source in &body.sources {
        let (result_type, result_id) = parse_source(source);
        match conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![query_text, query_embedding, source, result_type, result_id, signal, agent,],
        ) {
            Ok(_) => stored += 1,
            Err(e) => eprintln!("[feedback] Failed to store for {source}: {e}"),
        }
    }
    json_response(
        StatusCode::OK,
        json!({"stored":stored,"signal":signal,"sources":
body.sources,}),
    )
}
pub fn record_unfold_feedback(conn: &Connection, sources: &[String], agent: &str, query_text: &str, query_blob: Option<&[u8]>) {
    for source in sources {
        let (result_type, result_id) = parse_source(source);
        let _ = conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6)",
            params![query_text, query_blob, source, result_type, result_id, agent],
        );
    }
}
pub fn compute_boosts(conn: &Connection, sources: &[String], query_vector: Option<&[f32]>) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    if sources.is_empty() {
        return boosts;
    }
    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;
    let placeholders = sources.iter().enumerate().map(|(i, _)| format!("?{}", i + 1)).collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT result_source, signal, query_embedding, julianday('now') - julianday(created_at) AS age_days \
         FROM recall_feedback WHERE result_source IN ({placeholders})"
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::types::ToSql> = sources.iter().map(|s| s as &dyn rusqlite::types::ToSql).collect();
        if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
            let source: String = row.get(0)?;
            let signal: f64 = row.get(1)?;
            let query_blob: Option<Vec<u8>> = row.get(2)?;
            let age_days: f64 = row.get::<_, f64>(3)?.max(0.0);
            let query_weight = query_similarity_weight(query_vector, query_blob.as_deref());
            Ok((source, signal * query_weight * (-decay_lambda * age_days).exp()))
        }) {
            for row in rows.flatten() {
                *boosts.entry(row.0).or_insert(0.0) += row.1;
            }
        }
    }
    for v in boosts.values_mut() {
        *v = v.clamp(MIN_BOOST, MAX_BOOST);
    }
    boosts
}
fn query_similarity_weight(current_query: Option<&[f32]>, stored_blob: Option<&[u8]>) -> f64 {
    let Some(current_query) = current_query else {
        return 1.0;
    };
    let Some(stored_blob) = stored_blob else {
        return 0.6;
    };
    let stored_vec = embeddings::blob_to_vector(stored_blob);
    if stored_vec.is_empty() {
        return 0.6;
    }
    let sim = embeddings::cosine_similarity(current_query, &stored_vec).clamp(0.0, 1.0);
    0.2 + (sim as f64 * 0.8)
}
pub fn has_retrieval_immunity(conn: &Connection, source: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM recall_feedback \
         WHERE result_source = ?1 AND signal > 0 \
         AND julianday('now') - julianday(created_at) <= ?2",
        params![source, IMMUNITY_WINDOW_DAYS],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        >= IMMUNITY_THRESHOLD
}
fn parse_source(source: &str) -> (String, Option<i64>) {
    if let Some(rest) = source.strip_prefix("decision::") {
        let id = rest.parse::<i64>().ok();
        ("decision".to_string(), id)
    } else if let Some(_rest) = source.strip_prefix("memory::") {
        ("memory".to_string(), None)
    } else {
        ("unknown".to_string(), None)
    }
}
pub async fn handle_feedback_stats(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM recall_feedback", [], |row| row.get(0)).unwrap_or(0);
    let positive: i64 = conn
        .query_row("SELECT COUNT(*) FROM recall_feedback WHERE signal > 0", [], |row| row.get(0))
        .unwrap_or(0);
    let negative: i64 = conn
        .query_row("SELECT COUNT(*) FROM recall_feedback WHERE signal < 0", [], |row| row.get(0))
        .unwrap_or(0);
    let unique_sources: i64 = conn
        .query_row("SELECT COUNT(DISTINCT result_source) FROM recall_feedback", [], |row| row.get(0))
        .unwrap_or(0);
    let top: Vec<Value> = conn
        .prepare(
            "SELECT result_source, SUM(signal) as total_signal, COUNT(*) as hits \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) <= 30 \
             GROUP BY result_source ORDER BY total_signal DESC LIMIT 10",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| {
                Ok(json!({"source":row.get::<_,String>(0)?,"totalSignal":row.get::<_,f64>(
1)?,"hits":row.get::<_,i64>(2)?,}))
            })?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();
    json_response(
        StatusCode::OK,
        json!({
"total":total,"positive":positive,"negative":negative,"uniqueSources":unique_sources,"topBoosted":top,}),
    )
}
