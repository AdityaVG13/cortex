use super::*;use rusqlite::{params,Connection};pub(crate)fn aggregate_old_feedback(conn:&Connection)->usize{
aggregate_old_feedback_with_window(conn,FEEDBACK_AGGREGATION_DAYS)}pub(crate)fn aggregate_old_feedback_with_window(conn:&
Connection,aggregation_days:i64)->usize{let sources:Vec<(String,f64,i64)>=conn.prepare(
"SELECT result_source, SUM(signal), COUNT(*) \
             FROM recall_feedback \
             WHERE julianday('now') - julianday(created_at) > ?1 \
             GROUP BY result_source HAVING COUNT(*) > 1"
,).and_then(|mut stmt|{let rows=stmt.query_map(params![aggregation_days],|row|Ok((row.get::<_,String>(0)?,row.get::<_,f64>(1)?,row
.get::<_,i64>(2)?)))?;Ok(rows.flatten().collect())}).unwrap_or_default();if sources.is_empty(){return 0;}let mut aggregated=0usize
;for(source,net_signal,_count)in&sources{let deleted=conn.execute(
"DELETE FROM recall_feedback \
                 WHERE result_source = ?1 \
                 AND julianday('now') - julianday(created_at) > ?2"
,params![source,aggregation_days],).unwrap_or(0);if deleted>0{let _=conn.execute(
"INSERT INTO recall_feedback (query_text, result_source, result_type, signal, agent, created_at) \
                 VALUES ('[aggregated]', ?1, 'aggregated', ?2, 'compaction', datetime('now'))"
,params![source,net_signal],);aggregated+=deleted;}}aggregated}pub(crate)fn prune_old_benchmark_artifacts(conn:&Connection,
retention_days:i64,allow_vacuum:bool)->usize{purge_benchmark_artifacts_with_retention(conn,Some(retention_days),allow_vacuum).
total_deleted()}pub(crate)fn purge_benchmark_artifacts_with_retention(conn:&Connection,retention_days:Option<i64>,allow_vacuum:
bool)->BenchmarkPurgeResult{let mut result=BenchmarkPurgeResult{bytes_before:db_size_bytes(conn),..BenchmarkPurgeResult::default()
};let benchmark_source_pattern=format!("{BENCHMARK_SOURCE_AGENT_PREFIX}%");let retention_window=retention_days.map(|days|format!(
"-{days} days"));let _=conn.execute_batch(
"DROP TABLE IF EXISTS temp._benchmark_decision_ids;
         CREATE TEMP TABLE IF NOT EXISTS _benchmark_decision_ids (
           id INTEGER PRIMARY KEY
         );
         DELETE FROM _benchmark_decision_ids;"
,);match retention_window.as_deref(){Some(window)=>{let _=conn.execute(
"INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE (LOWER(COALESCE(type, '')) = 'benchmark' \
                        OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)) \
                   AND created_at < datetime('now', ?2)"
,params![benchmark_source_pattern.clone(),window],);}None=>{let _=conn.execute(
"INSERT INTO _benchmark_decision_ids (id) \
                 SELECT id \
                 FROM decisions \
                 WHERE LOWER(COALESCE(type, '')) = 'benchmark' \
                    OR LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1)"
,params![benchmark_source_pattern.clone()],);}}result.decision_conflicts_deleted=conn.execute(
"DELETE FROM decision_conflicts \
             WHERE source_decision_id IN (SELECT id FROM _benchmark_decision_ids) \
                OR target_decision_id IN (SELECT id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);result.embeddings_deleted=conn.execute(
"DELETE FROM embeddings \
             WHERE target_type = 'decision' \
               AND target_id IN (SELECT id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);result.cluster_members_deleted=conn.execute(
"DELETE FROM cluster_members \
             WHERE target_type = 'decision' \
               AND target_id IN (SELECT id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);result.cluster_members_deleted+=prune_orphan_cluster_members(conn);result.recall_feedback_deleted=conn.execute(
"DELETE FROM recall_feedback \
             WHERE result_source IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
                OR result_id IN (SELECT id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);result.co_occurrence_deleted=conn.execute(
"DELETE FROM co_occurrence \
             WHERE source_a IN (SELECT 'decision::' || id FROM _benchmark_decision_ids) \
                OR source_b IN (SELECT 'decision::' || id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);result.decisions_deleted=conn.execute(
"DELETE FROM decisions WHERE id IN (SELECT id FROM _benchmark_decision_ids)",[]).unwrap_or(0);result.events_deleted+=conn.execute(
"DELETE FROM events \
             WHERE type = 'decision_stored' \
               AND CAST(COALESCE(json_extract(data, '$.id'), 0) AS INTEGER) IN (SELECT id FROM _benchmark_decision_ids)"
,[],).unwrap_or(0);match retention_window.as_deref(){Some(window)=>{result.recall_feedback_deleted+=conn.execute(
"DELETE FROM recall_feedback \
                     WHERE (LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)) \
                       AND created_at < datetime('now', ?2)"
,params![benchmark_source_pattern.clone(),window],).unwrap_or(0);result.events_deleted+=conn.execute(
"DELETE FROM events \
                     WHERE (LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                            OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark') \
                       AND created_at < datetime('now', ?2)"
,params![benchmark_source_pattern.clone(),window],).unwrap_or(0);}None=>{result.recall_feedback_deleted+=conn.execute(
"DELETE FROM recall_feedback \
                     WHERE LOWER(COALESCE(agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(result_source, '')) LIKE LOWER(?1)"
,params![benchmark_source_pattern.clone()],).unwrap_or(0);result.events_deleted+=conn.execute(
"DELETE FROM events \
                     WHERE LOWER(COALESCE(source_agent, '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.source_agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE LOWER(?1) \
                        OR LOWER(COALESCE(json_extract(data, '$.entry_type'), '')) = 'benchmark'"
,params![benchmark_source_pattern.clone()],).unwrap_or(0);}}let _=conn.execute_batch(
"DROP TABLE IF EXISTS temp._benchmark_decision_ids;");let _=conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");if allow_vacuum
{let freelist_pages=freelist_count(conn);if freelist_pages>VACUUM_FREELIST_THRESHOLD_PAGES{let _=conn.execute_batch("VACUUM;");}}
result.bytes_after=db_size_bytes(conn);result}
