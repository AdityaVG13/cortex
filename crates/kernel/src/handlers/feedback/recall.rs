use rusqlite::{params, Connection};

const MAX_BOOST: f64 = 0.3;
const MIN_BOOST: f64 = -0.2;
const DECAY_HALF_LIFE_DAYS: f64 = 30.0;
pub const IMMUNITY_THRESHOLD: i64 = 5;
pub const IMMUNITY_WINDOW_DAYS: i64 = 14;
pub fn compute_boosts(
    conn: &Connection,
    sources: &[String],
    query_vector: Option<&[f32]>,
) -> std::collections::HashMap<String, f64> {
    let mut boosts = std::collections::HashMap::new();
    if sources.is_empty() {
        return boosts;
    }
    let decay_lambda = (2.0f64).ln() / DECAY_HALF_LIFE_DAYS;
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
            let query_weight = query_similarity_weight(query_vector, None);
            Ok((
                source,
                signal * query_weight * (-decay_lambda * age_days).exp(),
            ))
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
fn query_similarity_weight(_current_query: Option<&[f32]>, _stored_blob: Option<&[u8]>) -> f64 {
    1.0
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
pub fn parse_source(source: &str) -> (String, Option<i64>) {
    if let Some(rest) = source.strip_prefix("decision::") {
        let id = rest.parse::<i64>().ok();
        ("decision".to_string(), id)
    } else if let Some(rest) = source.strip_prefix("memory::") {
        let id = rest.parse::<i64>().ok();
        ("memory".to_string(), id)
    } else {
        ("unknown".to_string(), None)
    }
}
