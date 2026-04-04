//! Relevance Feedback Loop — learns which recalled results are actually useful.
//!
//! Signal sources:
//!   - Explicit: cortex_unfold() calls → positive signal for unfolded sources
//!   - Explicit: POST /feedback → caller reports useful/not-useful
//!
//! Feedback is stored in `recall_feedback` with the query embedding so future
//! recalls for similar queries can rerank based on what worked before.
//!
//! Reranking: boost = sum(signal * decay) for matching result_source,
//! where decay = exp(-age_days / 30). Capped at [-0.2, +0.3].

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::Json;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{ensure_auth, json_error, json_response};
use crate::embeddings;
use crate::state::RuntimeState;

// ─── Constants ──────────────────────────────────────────────────────────────

/// Max boost from feedback (prevents runaway amplification).
const MAX_BOOST: f64 = 0.3;
/// Max penalty from negative feedback.
const MIN_BOOST: f64 = -0.2;
/// Half-life for feedback signal decay (days).
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;
/// Minimum positive feedback signals in last 14 days to grant aging immunity.
pub const IMMUNITY_THRESHOLD: i64 = 5;
/// Window for aging immunity check (days).
pub const IMMUNITY_WINDOW_DAYS: i64 = 14;

// ─── POST /feedback ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct FeedbackRequest {
    pub query: Option<String>,
    pub sources: Vec<String>,
    pub signal: Option<f64>,
    pub agent: Option<String>,
}

pub async fn handle_feedback(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
    Json(body): Json<FeedbackRequest>,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    if body.sources.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "sources array is empty");
    }

    let signal = body.signal.unwrap_or(1.0).clamp(-1.0, 1.0);
    let agent = body.agent.as_deref().unwrap_or("http");
    let query_text = body.query.as_deref().unwrap_or("");

    let query_embedding = state
        .embedding_engine
        .as_ref()
        .and_then(|e| e.embed(query_text))
        .map(|v| embeddings::vector_to_blob(&v));

    let conn = state.db.lock().await;
    let mut stored = 0usize;
    for source in &body.sources {
        let (result_type, result_id) = parse_source(source);
        match conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                query_text,
                query_embedding,
                source,
                result_type,
                result_id,
                signal,
                agent,
            ],
        ) {
            Ok(_) => stored += 1,
            Err(e) => eprintln!("[feedback] Failed to store for {source}: {e}"),
        }
    }

    json_response(
        StatusCode::OK,
        json!({
            "stored": stored,
            "signal": signal,
            "sources": body.sources,
        }),
    )
}

// ─── Feedback recording (called internally by unfold) ───────────────────────

/// Record positive feedback for sources that were unfolded after a recall.
/// Called from the MCP unfold handler — no HTTP round-trip needed.
pub fn record_unfold_feedback(
    conn: &Connection,
    sources: &[String],
    agent: &str,
    engine: Option<&embeddings::EmbeddingEngine>,
    query_hint: Option<&str>,
) {
    let query_text = query_hint.unwrap_or("");
    let query_blob = engine
        .and_then(|e| e.embed(query_text))
        .map(|v| embeddings::vector_to_blob(&v));

    for source in sources {
        let (result_type, result_id) = parse_source(source);
        let _ = conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, result_id, signal, agent) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6)",
            params![query_text, query_blob, source, result_type, result_id, agent],
        );
    }
}

// ─── Reranking: compute boost for a result source ───────────────────────────

/// Compute a relevance boost for a given result source based on historical
/// feedback. Returns a value in [MIN_BOOST, MAX_BOOST].
///
/// Algorithm:
///   boost = sum(signal_i * exp(-age_days_i / HALF_LIFE)) for all feedback rows
///   clamped to [MIN_BOOST, MAX_BOOST]
///
/// This is O(feedback_rows_for_source) which stays small because:
/// - Each unfold generates ~2-3 rows
/// - Old feedback decays naturally
/// - We only scan rows for the specific source
#[allow(dead_code)]
pub fn compute_boost(conn: &Connection, result_source: &str) -> f64 {
    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;

    let boost: f64 = conn
        .prepare(
            "SELECT signal, julianday('now') - julianday(created_at) AS age_days \
             FROM recall_feedback WHERE result_source = ?1",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map(params![result_source], |row| {
                let signal: f64 = row.get(0)?;
                let age_days: f64 = row.get::<_, f64>(1)?.max(0.0);
                Ok(signal * (-decay_lambda * age_days).exp())
            })?;
            let mut total = 0.0f64;
            for v in rows.flatten() {
                total += v;
            }
            Ok(total)
        })
        .unwrap_or(0.0);

    boost.clamp(MIN_BOOST, MAX_BOOST)
}

/// Batch compute boosts for multiple sources at once (avoids N queries).
/// Returns a map of source → boost value.
pub fn compute_boosts(
    conn: &Connection,
    sources: &[String],
) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    if sources.is_empty() {
        return boosts;
    }

    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;

    // Single query: fetch all feedback for any of the requested sources
    let placeholders = sources
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");

    let sql = format!(
        "SELECT result_source, signal, julianday('now') - julianday(created_at) AS age_days \
         FROM recall_feedback WHERE result_source IN ({placeholders})"
    );

    if let Ok(mut stmt) = conn.prepare(&sql) {
        let params: Vec<&dyn rusqlite::types::ToSql> = sources
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
            let source: String = row.get(0)?;
            let signal: f64 = row.get(1)?;
            let age_days: f64 = row.get::<_, f64>(2)?.max(0.0);
            Ok((source, signal * (-decay_lambda * age_days).exp()))
        }) {
            for row in rows.flatten() {
                *boosts.entry(row.0).or_insert(0.0) += row.1;
            }
        }
    }

    // Clamp all values
    for v in boosts.values_mut() {
        *v = v.clamp(MIN_BOOST, MAX_BOOST);
    }

    boosts
}

/// Check if a source has enough recent positive feedback to be immune from aging.
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

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Parse a source string into (type, optional ID).
/// "decision::42" → ("decision", Some(42))
/// "memory::my_project.md" → ("memory", None)
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

// ─── GET /feedback/stats ────────────────────────────────────────────────────

pub async fn handle_feedback_stats(
    State(state): State<RuntimeState>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = ensure_auth(&headers, &state) {
        return resp;
    }
    let conn = state.db.lock().await;

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM recall_feedback", [], |row| row.get(0))
        .unwrap_or(0);
    let positive: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recall_feedback WHERE signal > 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let negative: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recall_feedback WHERE signal < 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let unique_sources: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT result_source) FROM recall_feedback",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Top boosted sources
    let top: Vec<Value> = conn
        .prepare(
            "SELECT result_source, SUM(signal) as total_signal, COUNT(*) as hits \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) <= 30 \
             GROUP BY result_source ORDER BY total_signal DESC LIMIT 10",
        )
        .and_then(|mut stmt| {
            let rows = stmt.query_map([], |row| {
                Ok(json!({
                    "source": row.get::<_, String>(0)?,
                    "totalSignal": row.get::<_, f64>(1)?,
                    "hits": row.get::<_, i64>(2)?,
                }))
            })?;
            Ok(rows.flatten().collect())
        })
        .unwrap_or_default();

    json_response(
        StatusCode::OK,
        json!({
            "total": total,
            "positive": positive,
            "negative": negative,
            "uniqueSources": unique_sources,
            "topBoosted": top,
        }),
    )
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_compute_boost_no_feedback() {
        let conn = setup_test_db();
        let boost = compute_boost(&conn, "memory::nonexistent");
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn test_compute_boost_positive() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::foo', 'memory', 1.0, 'test')",
            [],
        )
        .unwrap();
        let boost = compute_boost(&conn, "memory::foo");
        assert!(boost > 0.0, "Positive signal should produce positive boost");
        assert!(boost <= MAX_BOOST, "Boost should be capped at MAX_BOOST");
    }

    #[test]
    fn test_compute_boost_negative() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::bar', 'memory', -1.0, 'test')",
            [],
        )
        .unwrap();
        let boost = compute_boost(&conn, "memory::bar");
        assert!(boost < 0.0, "Negative signal should produce negative boost");
        assert!(boost >= MIN_BOOST, "Boost should be capped at MIN_BOOST");
    }

    #[test]
    fn test_compute_boosts_batch() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::a', 'memory', 1.0, 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
             VALUES ('test', 'memory::b', 'memory', -0.5, 'test')",
            [],
        )
        .unwrap();

        let sources = vec![
            "memory::a".to_string(),
            "memory::b".to_string(),
            "memory::c".to_string(),
        ];
        let boosts = compute_boosts(&conn, &sources);
        assert!(boosts["memory::a"] > 0.0);
        assert!(boosts["memory::b"] < 0.0);
        assert!(!boosts.contains_key("memory::c"), "No feedback = no entry");
    }

    #[test]
    fn test_parse_source() {
        assert_eq!(
            parse_source("decision::42"),
            ("decision".to_string(), Some(42))
        );
        assert_eq!(parse_source("memory::foo.md"), ("memory".to_string(), None));
        assert_eq!(parse_source("other"), ("unknown".to_string(), None));
    }

    #[test]
    fn test_has_retrieval_immunity_below_threshold() {
        let conn = setup_test_db();
        // Insert fewer than IMMUNITY_THRESHOLD signals
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
                 VALUES ('q', 'memory::x', 'memory', 1.0, 'test')",
                [],
            ).unwrap();
        }
        assert!(!has_retrieval_immunity(&conn, "memory::x"));
    }

    #[test]
    fn test_has_retrieval_immunity_above_threshold() {
        let conn = setup_test_db();
        for _ in 0..IMMUNITY_THRESHOLD {
            conn.execute(
                "INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent) \
                 VALUES ('q', 'memory::y', 'memory', 1.0, 'test')",
                [],
            ).unwrap();
        }
        assert!(has_retrieval_immunity(&conn, "memory::y"));
    }
}
