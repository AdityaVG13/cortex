use super::*;
use crate::embeddings;
use std::time::Duration;
pub(crate) type EmbeddingBackfillRows = Vec<(i64, String)>;
pub(crate) type EmbeddingBackfillTargets = (EmbeddingBackfillRows, EmbeddingBackfillRows);
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EmbeddingBackfillPassResult {
    pub(crate) queued_total: usize,
    pub(crate) computed_total: usize,
    pub(crate) passes_ran: usize,
    pub(crate) exhausted: bool,
}
pub(crate) fn backfill_batch_may_have_more(
    memory_count: usize,
    decision_count: usize,
    batch_size: usize,
) -> bool {
    memory_count >= batch_size || decision_count >= batch_size
}
pub(crate) fn collect_unembedded_targets_for_model(
    conn: &rusqlite::Connection,
    model_key: &str,
    limit: usize,
) -> EmbeddingBackfillTargets {
    let mem: EmbeddingBackfillRows = conn
        .prepare(
            "SELECT m.id, m.text FROM memories m \
             WHERE m.status = 'active' \
                AND NOT EXISTS (\
                    SELECT 1 FROM embeddings e \
                    WHERE e.target_type = 'memory' \
                      AND e.target_id = m.id \
                      AND LOWER(COALESCE(e.model, '')) = ?1\
                ) \
             ORDER BY m.id ASC \
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![model_key, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();
    let dec: EmbeddingBackfillRows = conn
        .prepare(
            "SELECT d.id, d.decision FROM decisions d \
             WHERE d.status = 'active' \
                AND NOT EXISTS (\
                    SELECT 1 FROM embeddings e \
                    WHERE e.target_type = 'decision' \
                      AND e.target_id = d.id \
                      AND LOWER(COALESCE(e.model, '')) = ?1\
                ) \
             ORDER BY d.id ASC \
             LIMIT ?2",
        )
        .and_then(|mut stmt| {
            stmt.query_map(rusqlite::params![model_key, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();
    (mem, dec)
}
pub(crate) fn count_unembedded_targets_for_model(
    conn: &rusqlite::Connection,
    model_key: &str,
) -> (usize, usize) {
    let memory_count = conn
        .query_row(
            "SELECT COUNT(*) FROM memories m \
             WHERE m.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'memory' \
                     AND e.target_id = m.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            rusqlite::params![model_key],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;
    let decision_count = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions d \
             WHERE d.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'decision' \
                     AND e.target_id = d.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            rusqlite::params![model_key],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;
    (memory_count, decision_count)
}
pub(crate) async fn build_embeddings_async(
    engine: std::sync::Arc<embeddings::EmbeddingEngine>,
    db: &std::sync::Arc<tokio::sync::Mutex<rusqlite::Connection>>,
    batch_size: usize,
    max_batches_per_pass: usize,
    lock_wait: Duration,
) -> EmbeddingBackfillPassResult {
    let model_key = engine.model_key();
    let mut result = EmbeddingBackfillPassResult::default();
    for _ in 0..max_batches_per_pass {
        let (unembedded_mem, unembedded_dec) = {
            let Some(conn) =
                acquire_background_db_lock(db, "embedding backfill scan", lock_wait).await
            else {
                break;
            };
            collect_unembedded_targets_for_model(&conn, model_key, batch_size)
        };
        let memory_count = unembedded_mem.len();
        let decision_count = unembedded_dec.len();
        let total = memory_count + decision_count;
        if total == 0 {
            result.exhausted = true;
            break;
        }
        result.passes_ran += 1;
        result.queued_total += total;
        let mut computed_batch = 0usize;
        let mut mem_results: Vec<(i64, Vec<u8>)> = Vec::new();
        for (id, text) in &unembedded_mem {
            if let Some(vec) = engine.clone().embed_async(text.clone()).await {
                mem_results.push((*id, embeddings::vector_to_blob(&vec)));
                computed_batch += 1;
            }
        }
        let mut dec_results: Vec<(i64, Vec<u8>)> = Vec::new();
        for (id, text) in &unembedded_dec {
            if let Some(vec) = engine.clone().embed_async(text.clone()).await {
                dec_results.push((*id, embeddings::vector_to_blob(&vec)));
                computed_batch += 1;
            }
        }
        {
            let Some(conn) =
                acquire_background_db_lock(db, "embedding backfill persist", lock_wait).await
            else {
                break;
            };
            for (id, blob) in &mem_results {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                     VALUES ('memory', ?1, ?2, ?3)",
                    rusqlite::params![id, blob, model_key],
                );
            }
            for (id, blob) in &dec_results {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO embeddings (target_type, target_id, vector, model) \
                     VALUES ('decision', ?1, ?2, ?3)",
                    rusqlite::params![id, blob, model_key],
                );
            }
        }
        result.computed_total += computed_batch;
        if !backfill_batch_may_have_more(memory_count, decision_count, batch_size) {
            result.exhausted = true;
            break;
        }
    }
    if result.queued_total > 0 {
        eprintln!(
"[embeddings] Built {}/{} embeddings this pass (passes={}, batch_size={}, max_batches={}, exhausted={})",result.computed_total,
result.queued_total,result.passes_ran,batch_size,max_batches_per_pass,result.exhausted);
    }
    result
}
