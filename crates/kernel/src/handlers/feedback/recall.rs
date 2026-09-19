use rusqlite::{Connection, params};

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
    let placeholders = crate::handlers::recall::numbered_placeholders(1, sources.len());
    let sql = format!(
        "SELECT result_source, signal, julianday('now') - julianday(created_at) AS age_days FROM recall_feedback WHERE result_source IN ({placeholders})"
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
            Ok((
                source,
                signal
                    * query_similarity_weight(query_vector, None)
                    * (-decay_lambda * age_days).exp(),
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
pub fn has_retrieval_immunity(conn: &Connection, source: &str) -> Result<bool, String> {
    Ok(crate::db::count_sql(
        conn,
        "SELECT COUNT(*) FROM recall_feedback WHERE result_source = ?1 AND signal > 0 AND julianday('now') - julianday(created_at) <= ?2",
        params![source, IMMUNITY_WINDOW_DAYS],
    )? >= IMMUNITY_THRESHOLD)
}
pub fn parse_source(source: &str) -> (String, Option<i64>) {
    [("decision::", "decision"), ("memory::", "memory")]
        .into_iter()
        .find_map(|(prefix, kind)| {
            source
                .strip_prefix(prefix)
                .map(|rest| (kind.to_string(), rest.parse().ok()))
        })
        .unwrap_or_else(|| ("unknown".to_string(), None))
}
