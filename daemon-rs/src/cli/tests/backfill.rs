// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    fn backfill_batch_may_have_more_only_when_a_table_hits_limit() {
        assert!(!backfill_batch_may_have_more(0, 0, 32));
        assert!(!backfill_batch_may_have_more(31, 8, 32));
        assert!(!backfill_batch_may_have_more(8, 31, 32));
        assert!(backfill_batch_may_have_more(32, 8, 32));
        assert!(backfill_batch_may_have_more(8, 32, 32));
        assert!(backfill_batch_may_have_more(32, 32, 32));
    }

    #[test]
    fn collect_unembedded_targets_for_model_rebuilds_mismatched_embeddings() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["legacy memory", "memory::legacy"],
        )
        .expect("insert memory legacy");
        let memory_legacy_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["current memory", "memory::current"],
        )
        .expect("insert memory current");
        let memory_current_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["legacy decision", "ctx::legacy"],
        )
        .expect("insert decision legacy");
        let decision_legacy_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["current decision", "ctx::current"],
        )
        .expect("insert decision current");
        let decision_current_id = conn.last_insert_rowid();

        let sample_blob = crate::embeddings::vector_to_blob(&[0.1, 0.2, 0.3]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            rusqlite::params![memory_legacy_id, sample_blob.clone()],
        )
        .expect("insert legacy memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
            rusqlite::params![memory_current_id, sample_blob.clone()],
        )
        .expect("insert current memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'OTHER-MODEL')",
            rusqlite::params![decision_legacy_id, sample_blob.clone()],
        )
        .expect("insert legacy decision embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-minilm-l6-v2')",
            rusqlite::params![decision_current_id, sample_blob],
        )
        .expect("insert current decision embedding");

        let (memories, decisions) =
            collect_unembedded_targets_for_model(&conn, "all-minilm-l6-v2", 256);
        let memory_ids: std::collections::HashSet<i64> =
            memories.iter().map(|(id, _)| *id).collect();
        let decision_ids: std::collections::HashSet<i64> =
            decisions.iter().map(|(id, _)| *id).collect();

        assert!(
            memory_ids.contains(&memory_legacy_id),
            "mismatched memory model should be queued for re-embedding"
        );
        assert!(
            !memory_ids.contains(&memory_current_id),
            "matching memory model should not be queued"
        );
        assert!(
            decision_ids.contains(&decision_legacy_id),
            "mismatched decision model should be queued for re-embedding"
        );
        assert!(
            !decision_ids.contains(&decision_current_id),
            "matching decision model should not be queued"
        );
    }

    #[test]
    fn collect_unembedded_targets_for_model_respects_limit_per_table() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        for idx in 0..3 {
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
                 VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
                rusqlite::params![format!("memory-{idx}"), format!("memory::{idx}")],
            )
            .expect("insert memory");
        }
        for idx in 0..3 {
            conn.execute(
                "INSERT INTO decisions (decision, context, status, score, merged_count, quality, created_at, updated_at)
                 VALUES (?1, ?2, 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
                rusqlite::params![format!("decision-{idx}"), format!("decision::{idx}")],
            )
            .expect("insert decision");
        }

        let (memories, decisions) =
            collect_unembedded_targets_for_model(&conn, "all-minilm-l6-v2", 1);
        assert_eq!(memories.len(), 1, "memory queue should honor LIMIT");
        assert_eq!(decisions.len(), 1, "decision queue should honor LIMIT");
        assert_eq!(memories[0].0, 1, "memory selection should be deterministic");
        assert_eq!(
            decisions[0].0, 1,
            "decision selection should be deterministic"
        );
    }

    #[test]
    fn count_unembedded_targets_for_model_reports_model_specific_backlog() {
        let conn = rusqlite::Connection::open_in_memory().expect("open sqlite");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-backlog", "tests::count"],
        )
        .expect("insert active backlog memory");
        let memory_backlog_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-current", "tests::count"],
        )
        .expect("insert active current memory");
        let memory_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'archived', 1.0, datetime('now'), datetime('now'))",
            rusqlite::params!["memory-archived", "tests::count"],
        )
        .expect("insert archived memory");

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-backlog", "tests::count"],
        )
        .expect("insert active backlog decision");
        let decision_backlog_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-current", "tests::count"],
        )
        .expect("insert active current decision");
        let decision_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at)
             VALUES (?1, ?2, 'tester', 'archived', 1.0, 0, 70, datetime('now'), datetime('now'))",
            rusqlite::params!["decision-archived", "tests::count"],
        )
        .expect("insert archived decision");

        let sample_blob = crate::embeddings::vector_to_blob(&[0.1, 0.2, 0.3]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            rusqlite::params![memory_backlog_id, sample_blob.clone()],
        )
        .expect("insert legacy memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-minilm-l6-v2')",
            rusqlite::params![memory_current_id, sample_blob.clone()],
        )
        .expect("insert current memory embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'other-model')",
            rusqlite::params![decision_backlog_id, sample_blob.clone()],
        )
        .expect("insert legacy decision embedding");
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, 'all-MiniLM-L6-v2')",
            rusqlite::params![decision_current_id, sample_blob],
        )
        .expect("insert current decision embedding");

        let (memory_count, decision_count) =
            count_unembedded_targets_for_model(&conn, "all-minilm-l6-v2");
        assert_eq!(
            memory_count, 1,
            "exactly one active memory should be pending"
        );
        assert_eq!(
            decision_count, 1,
            "exactly one active decision should be pending"
        );
    }

    #[test]
}
