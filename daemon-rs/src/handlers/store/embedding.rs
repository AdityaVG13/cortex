use rusqlite::{params, Connection};
pub fn persist_decision_embedding(
    conn: &Connection,
    decision_id: i64,
    vector: &[f32],
    model_key: &str,
) -> Result<(), String> {
    let blob = crate::embeddings::vector_to_blob(vector);
    conn.execute(
        "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
         VALUES ('decision', ?1, ?2, ?3)",
        params![decision_id, blob, model_key],
    )
    .map(|_| ())
    .map_err(|e| format!("Failed to persist decision embedding: {e}"))
}
