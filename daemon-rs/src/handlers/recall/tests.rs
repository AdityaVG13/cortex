// SPDX-License-Identifier: MIT
// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::store::{persist_decision_embedding, store_decision_with_input_embedding};
    use rusqlite::params;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    static SHARED_TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct StaticReranker;

    impl crate::rerank::Reranker for StaticReranker {
        fn name(&self) -> &'static str {
            "static_test_reranker"
        }

        fn model_size_mb(&self) -> u64 {
            1
        }

        fn rerank(
            &self,
            _query: &str,
            candidates: &[crate::rerank::RerankCandidate],
            fusion_alpha: f64,
        ) -> Result<Vec<crate::rerank::RerankedScore>, String> {
            let scores = candidates
                .iter()
                .map(|candidate| {
                    let score = if candidate.id == "memory::winner" {
                        10.0
                    } else {
                        -10.0
                    };
                    (candidate.id.clone(), score)
                })
                .collect::<Vec<_>>();
            Ok(crate::rerank::fuse_scores(
                candidates,
                &scores,
                fusion_alpha,
            ))
        }
    }

    // ── is_visible tests ───────────────────────────────────────────

    fn solo_ctx() -> RecallContext {
        RecallContext {
            caller_id: None,
            team_mode: false,
        }
    }
    fn team_ctx(caller: i64) -> RecallContext {
        RecallContext {
            caller_id: Some(caller),
            team_mode: true,
        }
    }
    fn team_ctx_no_caller() -> RecallContext {
        RecallContext {
            caller_id: None,
            team_mode: true,
        }
    }

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn shared_test_state() -> RuntimeState {
        let unique_id = SHARED_TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "cortex-recall-shared-{}-{}-{}.db",
            std::process::id(),
            unique_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write_conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::db::configure(&write_conn).unwrap();
        crate::db::initialize_schema(&write_conn).unwrap();
        crate::db::run_pending_migrations(&write_conn);

        let read_conn = rusqlite::Connection::open(&db_path).unwrap();
        crate::db::configure(&read_conn).unwrap();
        crate::db::initialize_schema(&read_conn).unwrap();
        crate::db::run_pending_migrations(&read_conn);

        let (events, _) = broadcast::channel(8);
        let (brain_firing, _) = broadcast::channel(8);
        RuntimeState {
            db: Arc::new(Mutex::new(write_conn)),
            db_read: Arc::new(Mutex::new(read_conn)),
            token: Arc::new("test-token".to_string()),
            events,
            brain_firing,
            mcp_calls: Arc::new(AtomicU64::new(0)),
            mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
            recall_history: Arc::new(Mutex::new(HashMap::new())),
            pre_cache: Arc::new(Mutex::new(HashMap::new())),
            served_content: Arc::new(Mutex::new(HashMap::new())),
            shutdown_tx: Arc::new(Mutex::new(None)),
            home: PathBuf::from("."),
            db_path: db_path.clone(),
            token_path: PathBuf::from("cortex.token"),
            pid_path: PathBuf::from("cortex.pid"),
            port: 7437,
            embedding_engine: None,
            rate_limiter: crate::rate_limit::RateLimiter::new(),
            team_mode: false,
            default_owner_id: None,
            team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
            degraded_mode: Arc::new(AtomicBool::new(false)),
            db_corrupted: Arc::new(AtomicBool::new(false)),
            readiness: Arc::new(AtomicBool::new(true)),
            last_activity_unix_secs: Arc::new(AtomicU64::new(0)),
            write_buffer_path: PathBuf::from("write_buffer.jsonl"),
            sqlite_vec_canary: crate::state::SqliteVecCanaryConfig {
                trial_percent: 0,
                force_off: false,
                route_mode: crate::state::SqliteVecRouteMode::Trial,
            },
            rerank_config: crate::rerank::RerankConfig::off(),
            reranker: None,
        }
    }

    fn latest_recall_query_event(conn: &rusqlite::Connection) -> Value {
        let raw: String = conn
            .query_row(
                "SELECT data FROM events WHERE type = 'recall_query' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("latest recall_query event should exist");
        serde_json::from_str(&raw).expect("recall_query event should be valid json")
    }

    fn recall_item_for_rerank(source: &str, relevance: f64) -> RecallItem {
        RecallItem {
            source: source.to_string(),
            relevance,
            excerpt: format!("rerank fixture for {source}"),
            method: "hybrid".to_string(),
            tokens: Some(10),
            entropy: Some(0.5),
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
        }
    }

    #[test]
    fn primary_rerank_reorders_top_window_and_marks_method() {
        let mut state = shared_test_state();
        state.rerank_config = crate::rerank::RerankConfig {
            mode: crate::rerank::RerankMode::Primary,
            top_n: 2,
            fusion_alpha: 0.90,
        };
        state.reranker = Some(Arc::new(StaticReranker));
        let results = vec![
            recall_item_for_rerank("memory::baseline", 0.95),
            recall_item_for_rerank("memory::winner", 0.70),
            recall_item_for_rerank("memory::outside", 0.60),
        ];

        let (reranked, route) = maybe_apply_rerank(&state, "winner query", results, 240);

        assert_eq!(route["status"], "ok");
        assert_eq!(route["mode"], "primary");
        assert_eq!(route["applied"], true);
        assert_eq!(route["baselineTopSources"][0], "memory::baseline");
        assert_eq!(route["rerankedTopSources"][0], "memory::winner");
        assert_eq!(reranked[0].source, "memory::winner");
        assert_eq!(reranked[2].source, "memory::outside");
        assert!(reranked[0].method.ends_with("+rerank"));
    }

    #[test]
    fn shadow_rerank_reports_route_without_reordering() {
        let mut state = shared_test_state();
        state.rerank_config = crate::rerank::RerankConfig {
            mode: crate::rerank::RerankMode::Shadow,
            top_n: 2,
            fusion_alpha: 0.90,
        };
        state.reranker = Some(Arc::new(StaticReranker));
        let results = vec![
            recall_item_for_rerank("memory::baseline", 0.95),
            recall_item_for_rerank("memory::winner", 0.70),
            recall_item_for_rerank("memory::outside", 0.60),
        ];

        let (reranked, route) = maybe_apply_rerank(&state, "winner query", results, 240);

        assert_eq!(route["status"], "ok");
        assert_eq!(route["mode"], "shadow");
        assert_eq!(route["applied"], false);
        assert_eq!(route["baselineTopSources"][0], "memory::baseline");
        assert_eq!(route["rerankedTopSources"][0], "memory::winner");
        assert_eq!(reranked[0].source, "memory::baseline");
        assert!(!reranked[0].method.ends_with("+rerank"));
    }

    fn insert_memory_with_embedding(
        conn: &rusqlite::Connection,
        text: &str,
        source: &str,
        vector: &[f32],
    ) -> i64 {
        let model_key = crate::embeddings::selected_model_key();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params![text, source],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model)
             VALUES ('memory', ?1, ?2, ?3)",
            params![id, crate::embeddings::vector_to_blob(vector), model_key],
        )
        .unwrap();
        id
    }

    fn insert_memory_with_optional_source_and_embedding(
        conn: &rusqlite::Connection,
        text: &str,
        source: Option<&str>,
        vector: &[f32],
    ) -> i64 {
        let model_key = crate::embeddings::selected_model_key();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params![text, source],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model)
             VALUES ('memory', ?1, ?2, ?3)",
            params![id, crate::embeddings::vector_to_blob(vector), model_key],
        )
        .unwrap();
        id
    }

    fn store_decision_with_embedding(
        conn: &mut rusqlite::Connection,
        decision: &str,
        context: &str,
        vector: &[f32],
    ) {
        let (_, new_id) = store_decision_with_input_embedding(
            conn,
            decision,
            Some(context.to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            Some(vector),
            None,
        )
        .unwrap();

        if let Some(id) = new_id {
            persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key())
                .unwrap();
        }
    }

    fn insert_crystal_with_memory_members(
        conn: &rusqlite::Connection,
        label: &str,
        consolidated_text: &str,
        crystal_vector: &[f32],
        members: &[(&str, &str, &[f32])],
    ) -> (i64, String, Vec<String>) {
        let mut member_sources = Vec::with_capacity(members.len());
        let mut member_ids = Vec::with_capacity(members.len());
        for (text, source, vector) in members {
            let id = insert_memory_with_embedding(conn, text, source, vector);
            member_ids.push(id);
            member_sources.push((*source).to_string());
        }

        if conn
            .execute(
                "INSERT INTO memory_clusters (
                    label,
                    centroid,
                    consolidated_text,
                    member_count,
                    owner_id,
                    visibility,
                    created_at,
                    updated_at
                 ) VALUES (?1, NULL, ?2, ?3, 1, 'shared', datetime('now'), datetime('now'))",
                params![label, consolidated_text, members.len() as i64],
            )
            .is_err()
        {
            conn.execute(
                "INSERT INTO memory_clusters (
                    label,
                    centroid,
                    consolidated_text,
                    member_count,
                    created_at,
                    updated_at
                 ) VALUES (?1, NULL, ?2, ?3, datetime('now'), datetime('now'))",
                params![label, consolidated_text, members.len() as i64],
            )
            .unwrap();
        }
        let crystal_id = conn.last_insert_rowid();

        for member_id in member_ids {
            conn.execute(
                "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity)
                 VALUES (?1, 'memory', ?2, 1.0)",
                params![crystal_id, member_id],
            )
            .unwrap();
        }

        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model)
             VALUES ('crystal', ?1, ?2, ?3)",
            params![
                crystal_id,
                crate::embeddings::vector_to_blob(crystal_vector),
                crate::embeddings::selected_model_key()
            ],
        )
        .unwrap();

        (
            crystal_id,
            crystal_source(crystal_id, label),
            member_sources,
        )
    }

    #[test]
    fn search_memories_excludes_temporally_invalid_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('expired memory', 'note', 'expired-memory', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('active memory', 'note', 'active-memory', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, valid_from, created_at, updated_at)
             VALUES ('future memory', 'note', 'future-memory', 'active', datetime('now', '+1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, valid_until, created_at, updated_at)
             VALUES ('stale memory', 'note', 'stale-memory', 'active', datetime('now', '-1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_memories(&conn, "", 10, None).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();

        assert!(sources.contains(&"active-memory"));
        assert!(!sources.contains(&"expired-memory"));
        assert!(!sources.contains(&"future-memory"));
        assert!(!sources.contains(&"stale-memory"));
    }

    #[test]
    fn search_decisions_excludes_temporally_invalid_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired decision', 'expired-decision', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('active decision', 'active-decision', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, valid_from, created_at, updated_at)
             VALUES ('future decision', 'future-decision', 'active', datetime('now', '+1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, valid_until, created_at, updated_at)
             VALUES ('stale decision', 'stale-decision', 'active', datetime('now', '-1 day'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_decisions(&conn, "", 10, None).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();

        assert!(sources.contains(&"active-decision"));
        assert!(!sources.contains(&"expired-decision"));
        assert!(!sources.contains(&"future-decision"));
        assert!(!sources.contains(&"stale-decision"));
    }

    #[test]
    fn search_decisions_prefers_higher_trust_for_same_keyword_signal() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES ('daemon lock lease renewal flow', 'decision::low-trust', 'active', 0.7, 0.2, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES ('daemon lock lease renewal flow', 'decision::high-trust', 'active', 0.7, 0.9, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let ranked = search_decisions(&conn, "daemon lock lease", 10, None).unwrap();
        let high_idx = ranked
            .iter()
            .position(|item| item.source == "decision::high-trust")
            .expect("high trust row should be present");
        let low_idx = ranked
            .iter()
            .position(|item| item.source == "decision::low-trust")
            .expect("low trust row should be present");
        assert!(
            high_idx < low_idx,
            "high-trust decision should rank ahead of low-trust when text signal is equal"
        );
    }

    #[test]
    fn store_then_keyword_recall_ranks_expected_entry_first() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "Run a WAL checkpoint before daily backup rotation during daemon startup.",
            "memory::wal-checkpoint",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use rtk cargo clippy -- -D warnings so CI fails on every warning.",
            "decision::clippy-gate",
            &[0.0, 1.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use the expect skill for screenshot QA and breakpoint comparisons on the dashboard.",
            "decision::expect-skill",
            &[0.0, 0.0, 1.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Keep three recent backups and delete older cortex database snapshots on startup.",
            "decision::backup-retention",
            &[0.0, 0.0, 0.0, 1.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Truncate write_buffer.jsonl after buffered entries flush into SQLite.",
            "decision::write-buffer",
            &[0.0, 0.0, 0.0, 0.0, 1.0],
        );

        let results =
            run_budget_recall(&mut conn, "write buffer", 400, 5, &solo_ctx(), None).unwrap();

        assert!(!results.is_empty());
        assert_eq!(
            results[0].source,
            "decision::write-buffer",
            "unexpected keyword ranking: {:?}",
            results
                .iter()
                .map(|item| item.source.clone())
                .collect::<Vec<_>>()
        );
        assert!(matches!(results[0].method.as_str(), "keyword" | "hybrid"));
    }

    #[test]
    fn store_then_semantic_recall_keeps_expected_entry_in_top_three() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "Run a WAL checkpoint before daily backup rotation during daemon startup.",
            "memory::wal-checkpoint",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use rtk cargo clippy -- -D warnings so CI fails on every warning.",
            "decision::clippy-gate",
            &[0.0, 1.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Use the expect skill for screenshot QA and breakpoint comparisons on the dashboard.",
            "decision::expect-skill",
            &[0.0, 0.0, 1.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Keep three recent backups and delete older cortex database snapshots on startup.",
            "decision::backup-retention",
            &[0.0, 0.0, 0.0, 1.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "Truncate write_buffer.jsonl after buffered entries flush into SQLite.",
            "decision::write-buffer",
            &[0.0, 0.0, 0.0, 0.0, 1.0],
        );

        let results = run_budget_recall_with_engine(
            &mut conn,
            "aurora lattice signal",
            400,
            5,
            None,
            &solo_ctx(),
            None,
            None,
        )
        .unwrap();
        assert!(results.is_empty(), "keyword-only path should not match");

        let embedding_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
            .unwrap();
        assert_eq!(embedding_count, 5);

        let expect_context: String = conn
            .query_row(
                "SELECT context FROM decisions WHERE context = 'decision::expect-skill'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expect_context, "decision::expect-skill");

        let expect_blob: Vec<u8> = conn
            .query_row(
                "SELECT e.vector
                 FROM embeddings e
                 JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id
                 WHERE d.context = 'decision::expect-skill'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let expect_similarity = crate::embeddings::cosine_similarity(
            &[0.0, 0.0, 1.0, 0.0, 0.0],
            &crate::embeddings::blob_to_vector(&expect_blob),
        );
        assert!(expect_similarity > 0.99);

        let decision_embedding_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM embeddings e
                 JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decision_embedding_rows, 4);

        let mut manual_semantic_ranking: Vec<(String, f32)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT e.vector, d.context
                     FROM embeddings e
                     JOIN decisions d ON e.target_type = 'decision' AND e.target_id = d.id
                     WHERE d.status = 'active'",
                )
                .unwrap();
            stmt.query_map([], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .unwrap()
            .filter_map(|row| row.ok())
            .filter_map(|(blob, context)| {
                let similarity = crate::embeddings::cosine_similarity(
                    &[0.0, 0.0, 1.0, 0.0, 0.0],
                    &crate::embeddings::blob_to_vector(&blob),
                );
                (similarity > 0.3).then_some((context.unwrap_or_default(), similarity))
            })
            .collect()
        };
        manual_semantic_ranking
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        assert!(
            manual_semantic_ranking
                .iter()
                .any(|(source, _)| source == "decision::expect-skill"),
            "expected semantic candidates to include target, got {:?}",
            manual_semantic_ranking
        );
        let position = manual_semantic_ranking
            .iter()
            .position(|(source, _)| source == "decision::expect-skill")
            .unwrap_or_else(|| {
                panic!(
                    "expected semantic target to be recalled, got {:?}",
                    manual_semantic_ranking
                )
            });
        assert!(
            position < 3,
            "expected top-3 semantic rank, got {}",
            position + 1
        );
        assert_eq!(
            manual_semantic_ranking[position].0,
            "decision::expect-skill"
        );
    }

    #[test]
    fn semantic_candidate_collection_supports_solo_schema_without_team_columns() {
        let conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "daemon ownership lock arbitration with wal checkpoint fallback",
            "memory::solo-semantic",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "token budgeting and shallow entropy heuristics",
            "memory::solo-noise",
            &[0.0, 1.0, 0.0, 0.0, 0.0],
        );

        let query_vector = [0.98, 0.02, 0.0, 0.0, 0.0];
        let candidates = collect_semantic_candidates(
            &conn,
            &query_vector,
            "daemon ownership lock",
            &solo_ctx(),
            None,
        );
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.source == "memory::solo-semantic"),
            "solo schema semantic fallback should still surface matching embeddings: {:?}",
            candidates
                .iter()
                .map(|candidate| candidate.source.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_candidate_collection_excludes_temporally_invalid_rows() {
        let conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "daemon lock arbitration healthy path",
            "memory::semantic-valid",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "daemon lock arbitration future path",
            "memory::semantic-future",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "daemon lock arbitration stale path",
            "memory::semantic-stale",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        conn.execute(
            "UPDATE memories SET valid_from = datetime('now', '+1 day') WHERE source = 'memory::semantic-future'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE memories SET valid_until = datetime('now', '-1 day') WHERE source = 'memory::semantic-stale'",
            [],
        )
        .unwrap();

        let query_vector = [0.98, 0.02, 0.0, 0.0, 0.0];
        let candidates = collect_semantic_candidates(
            &conn,
            &query_vector,
            "daemon lock arbitration",
            &solo_ctx(),
            None,
        );
        let sources: Vec<String> = candidates
            .iter()
            .map(|candidate| candidate.source.clone())
            .collect();

        assert!(sources
            .iter()
            .any(|source| source == "memory::semantic-valid"));
        assert!(!sources
            .iter()
            .any(|source| source == "memory::semantic-future"));
        assert!(!sources
            .iter()
            .any(|source| source == "memory::semantic-stale"));
    }

    #[test]
    fn semantic_candidate_collection_honors_source_prefix_scope() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "owner daemon lock arbitration handoff policy",
            "amb::suite::memory::hit",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "owner daemon lock arbitration handoff policy",
            "other::memory::noise",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "owner lock restart policy",
            "amb::suite::decision::hit",
            &[0.9, 0.1, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "owner lock restart policy",
            "other::decision::noise",
            &[0.9, 0.1, 0.0, 0.0, 0.0],
        );

        let query_vector = [0.98, 0.02, 0.0, 0.0, 0.0];
        let candidates = collect_semantic_candidates(
            &conn,
            &query_vector,
            "owner daemon lock",
            &solo_ctx(),
            Some("amb::suite::"),
        );

        assert!(
            !candidates.is_empty(),
            "expected scoped semantic candidates"
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.source.starts_with("amb::suite::")),
            "semantic candidates leaked outside scoped prefix: {:?}",
            candidates
                .iter()
                .map(|candidate| candidate.source.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn shadow_semantic_rows_honor_source_prefix_scope() {
        let mut conn = test_conn();
        insert_memory_with_embedding(
            &conn,
            "session ownership lock chain",
            "amb::suite::memory::shadow",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "session ownership lock chain",
            "other::memory::shadow-noise",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "session ownership lock decision",
            "amb::suite::decision::shadow",
            &[0.9, 0.1, 0.0, 0.0, 0.0],
        );
        store_decision_with_embedding(
            &mut conn,
            "session ownership lock decision",
            "other::decision::shadow-noise",
            &[0.9, 0.1, 0.0, 0.0, 0.0],
        );

        let rows = collect_shadow_semantic_rows(&conn, &solo_ctx(), Some("amb::suite::"), 5);
        assert!(!rows.is_empty(), "expected scoped shadow semantic rows");
        assert!(
            rows.iter()
                .all(|row| row.source.starts_with("amb::suite::")),
            "shadow semantic rows leaked outside scoped prefix: {:?}",
            rows.iter()
                .map(|row| row.source.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn is_visible_solo_mode_always_true() {
        let ctx = solo_ctx();
        assert!(is_visible(None, None, &ctx));
        assert!(is_visible(Some(1), Some("private"), &ctx));
        assert!(is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_owner_sees_own() {
        let ctx = team_ctx(1);
        assert!(is_visible(Some(1), Some("private"), &ctx));
        assert!(is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_shared_visible_to_other() {
        let ctx = team_ctx(2);
        assert!(is_visible(Some(1), Some("shared"), &ctx));
        assert!(is_visible(Some(1), Some("team"), &ctx));
    }

    #[test]
    fn is_visible_team_private_hidden_from_other() {
        let ctx = team_ctx(2);
        assert!(!is_visible(Some(1), Some("private"), &ctx));
        assert!(!is_visible(Some(1), None, &ctx));
    }

    #[test]
    fn is_visible_team_none_caller_denied() {
        let ctx = team_ctx_no_caller();
        assert!(!is_visible(Some(1), Some("private"), &ctx));
        assert!(!is_visible(Some(1), Some("shared"), &ctx));
        assert!(!is_visible(None, None, &ctx));
    }

    #[test]
    fn is_visible_team_none_owner_denied() {
        let ctx = team_ctx(1);
        assert!(!is_visible(None, Some("shared"), &ctx));
        assert!(!is_visible(None, None, &ctx));
    }

    #[test]
    fn recall_scopes_are_owner_isolated_in_team_mode() {
        let a = team_ctx(101);
        let b = team_ctx(202);
        assert_ne!(recall_scope_key("codex", &a), recall_scope_key("codex", &b));
        assert_ne!(
            served_content_scope("codex", "fix migration race", &a),
            served_content_scope("codex", "fix migration race", &b)
        );
    }

    #[test]
    fn unfold_source_memory_requires_exact_source_match() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params!["alpha", "memory::alpha"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params!["alphabet", "memory::alphabet"],
        )
        .unwrap();

        let exact = unfold_source(&conn, "memory::alpha", &solo_ctx())
            .and_then(|v| v["text"].as_str().map(|s| s.to_string()))
            .unwrap();
        assert_eq!(exact, "alpha");
        assert!(unfold_source(&conn, "memory::alp", &solo_ctx()).is_none());
    }

    #[test]
    fn unfold_source_legacy_schema_decision_id_lookup_works() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'active', 1.0, datetime('now'), datetime('now'))",
            params!["ship fix", "decision::ship-fix"],
        )
        .unwrap();

        let id = conn.last_insert_rowid();
        let out = unfold_source(&conn, &format!("decision::{id}"), &solo_ctx())
            .and_then(|v| v["text"].as_str().map(|s| s.to_string()))
            .unwrap();

        assert!(out.contains("ship fix"));
        assert!(out.contains("Context: decision::ship-fix"));
    }

    #[test]
    fn unfold_source_legacy_schema_team_mode_denies_without_acl_columns() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params!["legacy", "memory::legacy"],
        )
        .unwrap();

        assert!(unfold_source(&conn, "memory::legacy", &team_ctx(1)).is_none());
    }

    #[test]
    fn unfold_source_team_schema_shared_visible_private_hidden() {
        let conn = test_conn();
        conn.execute("ALTER TABLE memories ADD COLUMN owner_id INTEGER", [])
            .unwrap();
        conn.execute(
            "ALTER TABLE memories ADD COLUMN visibility TEXT
             CHECK (visibility IN ('private', 'team', 'shared'))",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, owner_id, visibility, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, 10, 'private', datetime('now'), datetime('now'))",
            params!["secret", "memory::private-note"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, owner_id, visibility, created_at, updated_at)
             VALUES (?1, ?2, 'note', 'active', 1.0, 10, 'shared', datetime('now'), datetime('now'))",
            params!["shared", "memory::shared-note"],
        )
        .unwrap();

        assert!(unfold_source(&conn, "memory::private-note", &team_ctx(99)).is_none());

        let shared = unfold_source(&conn, "memory::shared-note", &team_ctx(99))
            .and_then(|v| v["text"].as_str().map(|s| s.to_string()))
            .unwrap();
        assert_eq!(shared, "shared");
    }

    #[test]
    fn unfold_source_crystal_returns_summary_and_members() {
        let conn = test_conn();
        let (_crystal_id, crystal_key, member_sources) = insert_crystal_with_memory_members(
            &conn,
            "daemon lease renewal",
            "Lease renewal prevents duplicate daemon spawns and stale lock ownership.",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
            &[
                (
                    "Daemon lease renewal keeps the single-daemon invariant intact during recovery.",
                    "memory::daemon-lease-renewal",
                    &[1.0, 0.0, 0.0, 0.0, 0.0],
                ),
                (
                    "Lock ownership heartbeat stops plugin reconnects from spawning another daemon.",
                    "memory::plugin-lock-heartbeat",
                    &[0.98, 0.02, 0.0, 0.0, 0.0],
                ),
            ],
        );

        let crystal =
            unfold_source(&conn, &crystal_key, &solo_ctx()).expect("crystal should unfold");
        let text = crystal["text"].as_str().expect("crystal text");
        let members = crystal["members"]
            .as_array()
            .expect("crystal members")
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>();

        assert_eq!(crystal["type"], "crystal");
        assert_eq!(crystal["source"], crystal_key);
        assert_eq!(crystal["label"], "daemon lease renewal");
        assert_eq!(crystal["memberCount"].as_i64(), Some(2));
        assert_eq!(
            members,
            member_sources
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert!(text.contains("Lease renewal prevents duplicate daemon spawns"));
        assert!(text.contains("Family members:"));
        for source in &member_sources {
            assert!(
                text.contains(source),
                "crystal unfold should list member source {source}"
            );
        }
    }

    #[test]
    fn recall_collapses_crystal_family_members_under_family_head() {
        let mut conn = test_conn();
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];
        let (_crystal_id, crystal_key, member_sources) = insert_crystal_with_memory_members(
            &conn,
            "daemon lease renewal",
            "Lease renewal prevents duplicate daemon spawns and stale lock ownership.",
            &query_vector,
            &[
                (
                    "Daemon lease renewal keeps the single-daemon invariant intact during recovery.",
                    "memory::daemon-lease-renewal",
                    &query_vector,
                ),
                (
                    "Lock ownership heartbeat stops plugin reconnects from spawning another daemon.",
                    "memory::plugin-lock-heartbeat",
                    &[0.98, 0.02, 0.0, 0.0, 0.0],
                ),
            ],
        );

        let results = run_recall_with_query_vector(
            &mut conn,
            "daemon lease renewal single daemon",
            5,
            Some(&query_vector),
            &solo_ctx(),
            None,
        )
        .expect("recall should succeed");

        let crystal = results
            .iter()
            .find(|item| item.source == crystal_key)
            .expect("crystal family head should be returned");
        assert_eq!(crystal.method, "crystal");
        assert_eq!(crystal.family_members, member_sources);
        let mut collapsed_sources = crystal.collapsed_sources.clone();
        let mut expected_collapsed = crystal.family_members.clone();
        collapsed_sources.sort();
        expected_collapsed.sort();
        assert_eq!(collapsed_sources, expected_collapsed);
        assert_eq!(
            crystal.collapsed_source_scores.len(),
            crystal.collapsed_sources.len()
        );
        assert_eq!(
            crystal.collapsed_source_scores[0].0,
            crystal.collapsed_sources[0]
        );
        assert!(
            crystal.collapsed_source_scores[0].1 >= crystal.collapsed_source_scores[1].1,
            "collapsed child scores should preserve the ranked collapse order"
        );
        assert!(
            results.iter().all(|item| !crystal
                .family_members
                .iter()
                .any(|source| source == &item.source)),
            "member hits should collapse under the crystal family head: {:?}",
            results
                .iter()
                .map(|item| item.source.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn crystal_family_head_prefers_best_query_bearing_child_excerpt() {
        let mut conn = test_conn();
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];
        let (_crystal_id, crystal_key, _) = insert_crystal_with_memory_members(
            &conn,
            "daemon lifecycle",
            "Background memo one. Background memo two. Background memo three. Daemon lifecycle policy covers lease renewal, ownership locks, and recovery. The best detail is plugin reconnect heartbeat because it prevents duplicate daemon startup during recovery.",
            &query_vector,
            &[
                (
                    "Alpha background note about generic lifecycle concerns.",
                    "memory::aaa-background",
                    &[0.88, 0.12, 0.0, 0.0, 0.0],
                ),
                (
                    "Plugin reconnect heartbeat stops duplicate daemon startup during recovery.",
                    "memory::zzz-plugin-heartbeat",
                    &query_vector,
                ),
            ],
        );

        let results = run_recall_with_query_vector(
            &mut conn,
            "plugin reconnect heartbeat",
            5,
            Some(&query_vector),
            &solo_ctx(),
            None,
        )
        .expect("recall should succeed");

        let crystal = results
            .iter()
            .find(|item| item.source == crystal_key)
            .expect("crystal family head should be returned");
        let excerpt = crystal.excerpt.to_ascii_lowercase();
        assert!(
            excerpt.contains("plugin")
                || excerpt.contains("heartbeat")
                || excerpt.contains("reconnect"),
            "crystal excerpt should surface the best query-bearing family detail, got: {}",
            crystal.excerpt
        );
        assert!(
            !excerpt.starts_with("background memo one"),
            "crystal excerpt should not be a raw leading slice, got: {}",
            crystal.excerpt
        );
    }

    // ── existing tests ─────────────────────────────────────────────

    #[test]
    fn recall_collapses_null_source_members_using_memory_id_canonical_key() {
        let mut conn = test_conn();
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];
        let member_id = insert_memory_with_optional_source_and_embedding(
            &conn,
            "Lease heartbeat ownership prevents duplicate daemon startup after reconnect.",
            None,
            &query_vector,
        );
        let canonical_member_source = format!("memory::{member_id}");

        if conn
            .execute(
                "INSERT INTO memory_clusters (
                    label,
                    centroid,
                    consolidated_text,
                    member_count,
                    owner_id,
                    visibility,
                    created_at,
                    updated_at
                 ) VALUES (?1, NULL, ?2, 1, 1, 'shared', datetime('now'), datetime('now'))",
                params![
                    "lease heartbeat",
                    "Lease heartbeat preserves single-daemon ownership across reconnects."
                ],
            )
            .is_err()
        {
            conn.execute(
                "INSERT INTO memory_clusters (
                    label,
                    centroid,
                    consolidated_text,
                    member_count,
                    created_at,
                    updated_at
                 ) VALUES (?1, NULL, ?2, 1, datetime('now'), datetime('now'))",
                params![
                    "lease heartbeat",
                    "Lease heartbeat preserves single-daemon ownership across reconnects."
                ],
            )
            .unwrap();
        }
        let crystal_id = conn.last_insert_rowid();
        let crystal_key = crystal_source(crystal_id, "lease heartbeat");

        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity)
             VALUES (?1, 'memory', ?2, 1.0)",
            params![crystal_id, member_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model)
             VALUES ('crystal', ?1, ?2, ?3)",
            params![
                crystal_id,
                crate::embeddings::vector_to_blob(&query_vector),
                crate::embeddings::selected_model_key()
            ],
        )
        .unwrap();

        let results = run_recall_with_query_vector(
            &mut conn,
            "lease heartbeat duplicate daemon startup",
            5,
            Some(&query_vector),
            &solo_ctx(),
            None,
        )
        .expect("recall should succeed");

        let crystal = results
            .iter()
            .find(|item| item.source == crystal_key)
            .expect("crystal family head should be returned");
        assert!(crystal.family_members.contains(&canonical_member_source));
        assert!(crystal.collapsed_sources.contains(&canonical_member_source));
        assert!(
            results
                .iter()
                .all(|item| item.source != canonical_member_source),
            "null-source member should collapse under the crystal family head"
        );
    }

    #[tokio::test]
    async fn deduped_crystal_can_fall_back_to_a_collapsed_child_source() {
        let state = shared_test_state();
        let query = "daemon lease renewal single daemon";
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];

        let (crystal_key, member_sources, first_results, second_results) = {
            let mut conn = state.db.lock().await;
            let (_crystal_id, crystal_key, member_sources) = insert_crystal_with_memory_members(
                &conn,
                "daemon lease renewal",
                "Lease renewal prevents duplicate daemon spawns and stale lock ownership.",
                &query_vector,
                &[
                    (
                        "Daemon lease renewal keeps the single-daemon invariant intact during recovery.",
                        "memory::daemon-lease-renewal",
                        &query_vector,
                    ),
                    (
                        "Lock ownership heartbeat stops plugin reconnects from spawning another daemon.",
                        "memory::plugin-lock-heartbeat",
                        &[0.98, 0.02, 0.0, 0.0, 0.0],
                    ),
                ],
            );

            let first_results = run_recall_with_query_vector(
                &mut conn,
                query,
                5,
                Some(&query_vector),
                &solo_ctx(),
                None,
            )
            .expect("first recall should succeed");
            let second_results = run_recall_with_query_vector(
                &mut conn,
                query,
                5,
                Some(&query_vector),
                &solo_ctx(),
                None,
            )
            .expect("second recall should succeed");
            (crystal_key, member_sources, first_results, second_results)
        };

        let first = dedup_and_mark_served(&state, "codex", query, &solo_ctx(), first_results).await;
        assert!(
            first.iter().any(|item| item.source == crystal_key),
            "first serve should emit the crystal family head"
        );

        let second =
            dedup_and_mark_served(&state, "codex", query, &solo_ctx(), second_results).await;
        assert!(
            second.iter().all(|item| item.source != crystal_key),
            "second serve should not repeat the crystal summary when a child fallback is available"
        );
        assert!(
            second
                .iter()
                .any(|item| member_sources.iter().any(|source| source == &item.source)),
            "second serve should fall back to a collapsed child source"
        );
    }

    #[tokio::test]
    async fn deduped_crystal_fallback_prefers_highest_ranked_child_over_lexical_order() {
        let state = shared_test_state();
        let query = "daemon lifecycle";
        let crystal_key;
        {
            let conn = state.db.lock().await;
            let (id, key, _member_sources) = insert_crystal_with_memory_members(
                &conn,
                "daemon lifecycle",
                "Daemon lifecycle summary.",
                &[1.0, 0.0, 0.0, 0.0, 0.0],
                &[
                    (
                        "Alpha lifecycle background details.",
                        "memory::aaa-background",
                        &[0.8, 0.2, 0.0, 0.0, 0.0],
                    ),
                    (
                        "Plugin reconnect heartbeat stops duplicate daemon startup.",
                        "memory::zzz-plugin-heartbeat",
                        &[1.0, 0.0, 0.0, 0.0, 0.0],
                    ),
                ],
            );
            let _ = id;
            crystal_key = key;
        }

        let crystal_result = RecallItem {
            source: crystal_key.clone(),
            relevance: 0.92,
            excerpt: "Daemon lifecycle summary.".to_string(),
            method: "crystal".to_string(),
            tokens: None,
            entropy: None,
            family_members: vec![
                "memory::aaa-background".to_string(),
                "memory::zzz-plugin-heartbeat".to_string(),
            ],
            collapsed_sources: vec![
                "memory::aaa-background".to_string(),
                "memory::zzz-plugin-heartbeat".to_string(),
            ],
            collapsed_source_scores: vec![
                ("memory::aaa-background".to_string(), 0.41),
                ("memory::zzz-plugin-heartbeat".to_string(), 0.91),
            ],
        };

        let first = dedup_and_mark_served(
            &state,
            "codex",
            query,
            &solo_ctx(),
            vec![crystal_result.clone()],
        )
        .await;
        assert!(
            first.iter().any(|item| item.source == crystal_key),
            "first serve should emit the crystal family head"
        );

        let second =
            dedup_and_mark_served(&state, "codex", query, &solo_ctx(), vec![crystal_result]).await;
        assert!(
            second.iter().all(|item| item.source != crystal_key),
            "second serve should not repeat the crystal summary"
        );
        assert!(
            second
                .iter()
                .any(|item| item.source == "memory::zzz-plugin-heartbeat"),
            "second serve should prefer the highest-ranked collapsed child, got {:?}",
            second
                .iter()
                .map(|item| item.source.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn cache_roundtrip_preserves_crystal_collapse_metadata() {
        let cached = serde_json::Value::Array(vec![recall_to_json(RecallItem {
            source: "crystal::42::daemon lifecycle".to_string(),
            relevance: 0.91,
            excerpt: "Daemon lifecycle summary.".to_string(),
            method: "crystal".to_string(),
            tokens: Some(18),
            entropy: Some(3.7),
            family_members: vec![
                "memory::aaa-background".to_string(),
                "memory::zzz-plugin-heartbeat".to_string(),
            ],
            collapsed_sources: vec![
                "memory::zzz-plugin-heartbeat".to_string(),
                "memory::aaa-background".to_string(),
            ],
            collapsed_source_scores: vec![
                ("memory::zzz-plugin-heartbeat".to_string(), 0.91),
                ("memory::aaa-background".to_string(), 0.41),
            ],
        })]);

        let items = deserialize_cache_entry(&cached).expect("cache entry should deserialize");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.source, "crystal::42::daemon lifecycle");
        assert_eq!(
            item.family_members,
            vec![
                "memory::aaa-background".to_string(),
                "memory::zzz-plugin-heartbeat".to_string(),
            ]
        );
        assert_eq!(
            item.collapsed_sources,
            vec![
                "memory::zzz-plugin-heartbeat".to_string(),
                "memory::aaa-background".to_string(),
            ]
        );
        assert_eq!(
            item.collapsed_source_scores,
            vec![
                ("memory::zzz-plugin-heartbeat".to_string(), 0.91),
                ("memory::aaa-background".to_string(), 0.41),
            ]
        );
    }

    #[test]
    fn test_shannon_entropy_empty() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn test_shannon_entropy_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn test_shannon_entropy_two_equal_chars() {
        let h = shannon_entropy("ab");
        assert!((h - 1.0).abs() < 0.001, "expected ~1.0, got {h}");
    }

    #[test]
    fn test_shannon_entropy_english_prose_range() {
        let prose = "The quick brown fox jumps over the lazy dog near the riverbank";
        let h = shannon_entropy(prose);
        assert!(
            h > 3.5 && h < 5.0,
            "english prose entropy {h} outside expected 3.5-5.0"
        );
    }

    #[test]
    fn test_shannon_entropy_boilerplate_lower() {
        let boilerplate = "aaabbbccc aaabbbccc aaabbbccc";
        let prose = "The zephyr-cache module uses LRU eviction with a 512-entry cap";
        assert!(shannon_entropy(boilerplate) < shannon_entropy(prose));
    }

    #[test]
    fn test_hash_content_deterministic() {
        assert_eq!(hash_content("test content"), hash_content("test content"));
    }

    #[test]
    fn test_hash_content_different() {
        assert_ne!(hash_content("content a"), hash_content("content b"));
    }

    #[test]
    fn test_extract_keywords_filters_stopwords() {
        let kw = extract_keywords("the quick brown fox jumps over a lazy dog");
        assert!(kw.contains(&"quick".to_string()));
        assert!(kw.contains(&"brown".to_string()));
        assert!(!kw.contains(&"the".to_string()));
        assert!(!kw.contains(&"an".to_string()));
    }

    #[test]
    fn test_extract_keywords_filters_short() {
        let kw = extract_keywords("go to db");
        assert!(kw.is_empty());
    }

    #[test]
    fn test_extract_search_keywords_keeps_short() {
        let kw = extract_search_keywords("go to db");
        assert!(kw.contains(&"go".to_string()));
        assert!(kw.contains(&"db".to_string()));
    }

    #[test]
    fn test_entity_alignment_metrics_with_terms_detects_structured_identifiers() {
        let query_entities = query_entity_terms("sqlite vec migration 012");
        let (matches, overlap) = entity_alignment_metrics_with_terms(
            "Schema migration 012 adds sqlite-vec trial routing.",
            &query_entities,
        );
        assert!(
            matches >= 2,
            "expected at least two aligned entity-like terms"
        );
        assert!(
            overlap > 0.45,
            "expected meaningful entity overlap, got {overlap}"
        );
    }

    #[test]
    fn test_entity_signal_boost_is_capped() {
        let boost = entity_signal_boost(12, 1.0);
        assert!(
            (boost - ENTITY_SIGNAL_MAX_BOOST).abs() < f64::EPSILON,
            "entity boost should be capped at {}",
            ENTITY_SIGNAL_MAX_BOOST
        );
    }

    #[test]
    fn test_query_entity_terms_fallback_keeps_short_technical_terms() {
        let terms = query_entity_terms("api sql db auth");
        assert!(terms.contains("api"));
        assert!(terms.contains("sql"));
        assert!(terms.contains("db"));
    }

    #[test]
    fn test_query_prefers_recency_detects_latest_intent() {
        assert!(query_prefers_recency(
            "latest daemon startup ownership policy"
        ));
        assert!(query_prefers_recency(
            "what is the current lock lease status?"
        ));
        assert!(!query_prefers_recency("daemon startup ownership policy"));
    }

    #[test]
    fn test_temporal_intent_multiplier_prefers_recent_items() {
        let now = Utc::now().timestamp_millis();
        let recent = temporal_intent_multiplier(now - 24 * 60 * 60 * 1000);
        let old = temporal_intent_multiplier(now - 180 * 24 * 60 * 60 * 1000);
        assert!(
            recent > old,
            "recent multiplier should exceed old multiplier ({recent} vs {old})"
        );
    }

    #[test]
    fn test_query_alignment_boost_rewards_exact_phrase_and_coverage() {
        let query = "daemon lock lease";
        let profile = QueryAlignmentProfile::from_query(query);
        let term_count = profile.term_count;
        let exact = query_alignment_boost_with_profile(
            "memory::daemon-lock",
            "daemon lock lease protects startup arbitration",
            &profile,
            term_count,
        );
        let weak = query_alignment_boost_with_profile(
            "memory::generic",
            "startup details with little overlap",
            &profile,
            term_count,
        );
        assert!(
            exact > weak,
            "exact phrase/coverage alignment should score higher ({exact} vs {weak})"
        );
    }

    #[test]
    fn test_peek_smoke_returns_sorted_relevant_matches() {
        let mut conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
             VALUES (?1, 'memory::daemon-lock-recent', 'note', 'active', 0.85, 0.9, 1, datetime('now', '-1 day'), datetime('now', '-1 day'), datetime('now', '-1 day'))",
            params!["daemon ownership lock lease prevents duplicate startup"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
             VALUES (?1, 'memory::daemon-lock-older', 'note', 'active', 0.83, 0.82, 2, datetime('now', '-10 days'), datetime('now', '-10 days'), datetime('now', '-10 days'))",
            params!["daemon lock lease policy and startup arbitration notes"],
        )
        .unwrap();

        let results = run_recall(&mut conn, "daemon lock lease startup", 5, &solo_ctx(), None)
            .expect("peek-style recall should succeed");
        assert!(!results.is_empty(), "peek should return at least one match");
        assert!(
            results
                .windows(2)
                .all(|pair| compare_relevance_desc_source_asc(
                    pair[0].relevance,
                    &pair[0].source,
                    pair[1].relevance,
                    &pair[1].source,
                ) != std::cmp::Ordering::Greater),
            "peek results should remain sorted by relevance/source"
        );
        assert!(
            results
                .iter()
                .all(|item| !item.excerpt.trim().is_empty() && !item.method.trim().is_empty()),
            "peek results should provide non-empty excerpt + method"
        );
    }

    #[test]
    fn test_peek_latest_intent_prefers_fresher_source() {
        let mut conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
             VALUES (?1, 'memory::policy-legacy', 'note', 'active', 0.96, 0.95, 6, datetime('now', '-160 days'), datetime('now', '-160 days'), datetime('now', '-160 days'))",
            params!["daemon startup ownership policy and lock lease contract"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
             VALUES (?1, 'memory::policy-current', 'note', 'active', 0.78, 0.8, 1, datetime('now', '-1 day'), datetime('now', '-1 day'), datetime('now', '-1 day'))",
            params!["current daemon startup ownership policy and lock lease contract"],
        )
        .unwrap();

        let results = run_recall_with_query_vector(
            &mut conn,
            "latest daemon startup ownership policy",
            4,
            None,
            &solo_ctx(),
            None,
        )
        .expect("latest-intent recall should succeed");
        assert!(
            results.first().map(|item| item.source.as_str()) == Some("memory::policy-current"),
            "latest-intent recall should prioritize fresher source: {results:?}"
        );
    }

    #[tokio::test]
    async fn recall_smoke_returns_stable_order_and_non_empty_excerpts() {
        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
                 VALUES (?1, 'memory::alpha', 'note', 'active', 0.8, 0.82, 2, datetime('now', '-2 days'), datetime('now', '-2 days'), datetime('now', '-2 days'))",
                params!["daemon startup lock lease details for alpha policy"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, trust_score, retrievals, last_accessed, created_at, updated_at)
                 VALUES (?1, 'memory::beta', 'note', 'active', 0.8, 0.82, 2, datetime('now', '-2 days'), datetime('now', '-2 days'), datetime('now', '-2 days'))",
                params!["daemon startup lock lease details for beta policy"],
            )
            .unwrap();
        }

        let payload_a = execute_unified_recall(
            &state,
            "daemon startup lock lease",
            320,
            6,
            "codex-smoke-a",
            &solo_ctx(),
            None,
        )
        .await
        .expect("first recall smoke call should succeed");
        let payload_b = execute_unified_recall(
            &state,
            "daemon startup lock lease",
            320,
            6,
            "codex-smoke-b",
            &solo_ctx(),
            None,
        )
        .await
        .expect("second recall smoke call should succeed");

        let sources = |payload: &Value| {
            payload["results"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| row.get("source").and_then(|value| value.as_str()))
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let excerpts_non_empty = |payload: &Value| {
            payload["results"]
                .as_array()
                .map(|rows| {
                    rows.iter().all(|row| {
                        row.get("excerpt")
                            .and_then(|value| value.as_str())
                            .map(|text| !text.trim().is_empty())
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        };
        let sorted_by_relevance_then_source = |payload: &Value| {
            payload["results"]
                .as_array()
                .map(|rows| {
                    rows.windows(2).all(|pair| {
                        let left = &pair[0];
                        let right = &pair[1];
                        let left_relevance = left
                            .get("relevance")
                            .and_then(|value| value.as_f64())
                            .unwrap_or(0.0);
                        let right_relevance = right
                            .get("relevance")
                            .and_then(|value| value.as_f64())
                            .unwrap_or(0.0);
                        let left_source = left
                            .get("source")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        let right_source = right
                            .get("source")
                            .and_then(|value| value.as_str())
                            .unwrap_or("");
                        compare_relevance_desc_source_asc(
                            left_relevance,
                            left_source,
                            right_relevance,
                            right_source,
                        ) != std::cmp::Ordering::Greater
                    })
                })
                .unwrap_or(false)
        };

        assert!(
            !sources(&payload_a).is_empty(),
            "recall smoke should return at least one result"
        );
        let mut a_sources = sources(&payload_a);
        let mut b_sources = sources(&payload_b);
        a_sources.sort();
        b_sources.sort();
        assert_eq!(
            a_sources, b_sources,
            "recall smoke should keep the same source set"
        );
        assert!(
            sorted_by_relevance_then_source(&payload_a)
                && sorted_by_relevance_then_source(&payload_b),
            "recall smoke payloads should stay sorted by relevance/source"
        );
        assert!(
            excerpts_non_empty(&payload_a) && excerpts_non_empty(&payload_b),
            "recall smoke results should always include non-empty excerpts"
        );
    }

    #[test]
    fn test_round4() {
        assert_eq!(round4(0.12345), 0.1235);
        assert_eq!(round4(1.0), 1.0);
        assert_eq!(round4(f64::NAN), 0.0);
        assert_eq!(round4(f64::INFINITY), 0.0);
    }

    #[test]
    fn bm25_weights_from_resolver_uses_defaults_when_env_missing() {
        let weights = bm25_weights_from_resolver(|_| None);
        assert!((weights.memories_text - MEMORIES_BM25_TEXT_WEIGHT).abs() < f64::EPSILON);
        assert!((weights.memories_source - MEMORIES_BM25_SOURCE_WEIGHT).abs() < f64::EPSILON);
        assert!((weights.memories_tags - MEMORIES_BM25_TAGS_WEIGHT).abs() < f64::EPSILON);
        assert!((weights.decisions_text - DECISIONS_BM25_DECISION_WEIGHT).abs() < f64::EPSILON);
        assert!((weights.decisions_context - DECISIONS_BM25_CONTEXT_WEIGHT).abs() < f64::EPSILON);
    }

    #[test]
    fn bm25_weights_from_resolver_applies_overrides_and_clamps() {
        let env = HashMap::from([
            ("CORTEX_BM25_MEM_TEXT_WEIGHT".to_string(), "4.9".to_string()),
            (
                "CORTEX_BM25_MEM_SOURCE_WEIGHT".to_string(),
                "-3".to_string(),
            ),
            ("CORTEX_BM25_MEM_TAGS_WEIGHT".to_string(), "999".to_string()),
            ("CORTEX_BM25_DECISION_WEIGHT".to_string(), "abc".to_string()),
            ("CORTEX_BM25_CONTEXT_WEIGHT".to_string(), "0.01".to_string()),
        ]);
        let weights = bm25_weights_from_resolver(|name| env.get(name).cloned());
        assert!((weights.memories_text - 4.9).abs() < 0.0001);
        assert!((weights.memories_source - MEMORIES_BM25_SOURCE_WEIGHT).abs() < 0.0001);
        assert!((weights.memories_tags - BM25_WEIGHT_MAX).abs() < 0.0001);
        assert!((weights.decisions_text - DECISIONS_BM25_DECISION_WEIGHT).abs() < 0.0001);
        assert!((weights.decisions_context - BM25_WEIGHT_MIN).abs() < 0.0001);
    }

    #[test]
    fn shadow_error_to_unavailable_reason_detects_missing_vec_module() {
        assert_eq!(
            shadow_error_to_unavailable_reason(
                "sqlite-vec shadow create failed: no such module: vec0"
            ),
            Some("sqlite_vec_not_available")
        );
        assert_eq!(
            shadow_error_to_unavailable_reason(
                "sqlite-vec shadow row decode failed: malformed row"
            ),
            None
        );
    }

    #[test]
    fn shadow_semantic_telemetry_summary_compacts_ok_payload() {
        let summary = shadow_semantic_telemetry_summary(&json!({
            "enabled": true,
            "status": "ok",
            "topK": 6,
            "vectorDimension": 5,
            "baselineCandidateCount": 11,
            "shadowCandidateCount": 9,
            "baselineTopSources": ["memory::a", "memory::b"],
            "shadowTopSources": ["memory::b", "memory::c"],
            "overlapCount": 1,
            "overlapRatio": 0.1667,
            "jaccard": 0.3333,
            "matchedRankPairs": 1,
            "meanAbsRankDelta": 1.0,
            "top1Match": false
        }));

        assert_eq!(summary["status"].as_str(), Some("ok"));
        assert_eq!(summary["topK"].as_u64(), Some(6));
        assert_eq!(summary["vectorDimension"].as_u64(), Some(5));
        assert_eq!(summary["baselineCandidateCount"].as_u64(), Some(11));
        assert_eq!(summary["shadowCandidateCount"].as_u64(), Some(9));
        assert_eq!(summary["overlapCount"].as_u64(), Some(1));
        assert_eq!(summary["overlapRatio"].as_f64(), Some(0.1667));
        assert_eq!(summary["jaccard"].as_f64(), Some(0.3333));
        assert_eq!(summary["matchedRankPairs"].as_u64(), Some(1));
        assert_eq!(summary["meanAbsRankDelta"].as_f64(), Some(1.0));
        assert_eq!(summary["top1Match"].as_bool(), Some(false));
        assert!(
            summary["baselineTopSources"].is_null(),
            "telemetry summary should omit baseline source arrays"
        );
        assert!(
            summary["shadowTopSources"].is_null(),
            "telemetry summary should omit shadow source arrays"
        );
    }

    #[test]
    fn sqlite_vec_trial_sampled_is_deterministic_for_same_inputs() {
        let first = sqlite_vec_trial_sampled("daemon lock arbitration", &solo_ctx(), None, 25);
        let second = sqlite_vec_trial_sampled("daemon lock arbitration", &solo_ctx(), None, 25);
        assert_eq!(first, second, "canary sampling must be deterministic");
    }

    #[test]
    fn shadow_guard_failure_reason_enforces_trial_gates() {
        let low_overlap = json!({
            "status": "ok",
            "overlapRatio": 0.2,
            "jaccard": 0.8,
            "meanAbsRankDelta": 0.2,
            "top1Match": true
        });
        assert_eq!(
            shadow_guard_failure_reason(&low_overlap),
            Some("overlap_ratio_below_gate")
        );

        let guard_pass = json!({
            "status": "ok",
            "overlapRatio": 0.9,
            "jaccard": 0.9,
            "meanAbsRankDelta": 0.4,
            "top1Match": true
        });
        assert_eq!(shadow_guard_failure_reason(&guard_pass), None);
    }

    #[test]
    fn maybe_apply_sqlite_vec_trial_force_off_keeps_baseline() {
        let conn = test_conn();
        let query_vector = [1.0_f32, 0.0_f32, 0.0_f32];
        let baseline = vec![SemanticCandidate {
            source: "memory::daemon-lock".to_string(),
            excerpt: "daemon ownership lock protects startup arbitration".to_string(),
            relevance: 0.81,
            importance: 0.9,
            ts: 0,
        }];
        let (routed, route) = maybe_apply_sqlite_vec_trial(
            &conn,
            "daemon ownership lock",
            Some(&query_vector),
            baseline.clone(),
            &solo_ctx(),
            None,
            4,
            Some(&crate::state::SqliteVecCanaryConfig {
                trial_percent: 100,
                force_off: true,
                route_mode: crate::state::SqliteVecRouteMode::Trial,
            }),
        );
        assert_eq!(routed.len(), baseline.len());
        assert_eq!(routed[0].source, baseline[0].source);
        assert_eq!(route["mode"].as_str(), Some("baseline"));
        assert_eq!(route["reason"].as_str(), Some("trial_force_off"));
    }

    #[test]
    fn maybe_apply_sqlite_vec_trial_unsampled_keeps_baseline() {
        let conn = test_conn();
        let query_vector = [1.0_f32, 0.0_f32, 0.0_f32];
        let baseline = vec![SemanticCandidate {
            source: "memory::daemon-lock".to_string(),
            excerpt: "daemon ownership lock protects startup arbitration".to_string(),
            relevance: 0.81,
            importance: 0.9,
            ts: 0,
        }];

        let mut query = "daemon ownership lock".to_string();
        for _ in 0..256 {
            if !sqlite_vec_trial_sampled(&query, &solo_ctx(), None, 1) {
                break;
            }
            query.push('x');
        }
        assert!(
            !sqlite_vec_trial_sampled(&query, &solo_ctx(), None, 1),
            "test should locate an unsampled query for 1% trial buckets"
        );

        let (routed, route) = maybe_apply_sqlite_vec_trial(
            &conn,
            &query,
            Some(&query_vector),
            baseline.clone(),
            &solo_ctx(),
            None,
            4,
            Some(&crate::state::SqliteVecCanaryConfig {
                trial_percent: 1,
                force_off: false,
                route_mode: crate::state::SqliteVecRouteMode::Trial,
            }),
        );
        assert_eq!(routed.len(), baseline.len());
        assert_eq!(routed[0].source, baseline[0].source);
        assert_eq!(route["mode"].as_str(), Some("baseline"));
        assert_eq!(route["reason"].as_str(), Some("not_sampled"));
        assert_eq!(route["sampled"].as_bool(), Some(false));
    }

    #[test]
    fn maybe_apply_sqlite_vec_primary_includes_shadow_only_sources() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("cortex-sqlite-vec-primary-{unique}.db"));
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");

        let conn = crate::db::open(&db_path).expect("db open should register sqlite-vec");
        crate::db::configure(&conn).expect("db configure should succeed");
        crate::db::initialize_schema(&conn).expect("schema init should succeed");
        crate::db::run_pending_migrations(&conn);

        insert_memory_with_embedding(
            &conn,
            "daemon ownership lock protects startup arbitration",
            "memory::daemon-lock",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "token budget windows tune startup telemetry visibility",
            "memory::token-budget",
            &[0.7, 0.3, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "sqlite vec rollout adds deterministic fallback hydration",
            "memory::vector-upgrade",
            &[0.2, 0.8, 0.0, 0.0, 0.0],
        );

        let baseline = vec![
            SemanticCandidate {
                source: "memory::daemon-lock".to_string(),
                excerpt: "daemon ownership lock protects startup arbitration".to_string(),
                relevance: 0.91,
                importance: 0.9,
                ts: 0,
            },
            SemanticCandidate {
                source: "memory::token-budget".to_string(),
                excerpt: "token budget windows tune startup telemetry visibility".to_string(),
                relevance: 0.73,
                importance: 0.8,
                ts: 0,
            },
            SemanticCandidate {
                source: "memory::legacy-route".to_string(),
                excerpt: "legacy semantic route for startup diagnostics".to_string(),
                relevance: 0.62,
                importance: 0.7,
                ts: 0,
            },
        ];

        let query_vector = [0.95_f32, 0.05_f32, 0.0_f32, 0.0_f32, 0.0_f32];
        let (routed, route) = maybe_apply_sqlite_vec_trial(
            &conn,
            "daemon ownership lock",
            Some(&query_vector),
            baseline.clone(),
            &solo_ctx(),
            None,
            3,
            Some(&crate::state::SqliteVecCanaryConfig {
                trial_percent: 20,
                force_off: false,
                route_mode: crate::state::SqliteVecRouteMode::Primary,
            }),
        );

        assert_eq!(route["mode"].as_str(), Some("vec0_primary"));
        assert_eq!(route["reason"].as_str(), Some("route_mode_primary"));
        assert_eq!(routed.len(), baseline.len());
        assert_eq!(routed[0].source, "memory::daemon-lock");
        assert_eq!(routed[1].source, "memory::token-budget");
        assert_eq!(routed[2].source, "memory::vector-upgrade");
        assert!(
            routed[2].excerpt.contains("sqlite vec rollout"),
            "shadow-only source should hydrate excerpt from persisted memory text"
        );

        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);
    }

    #[tokio::test]
    async fn execute_unified_recall_logs_shadow_semantic_summary_when_uncached() {
        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            insert_memory_with_embedding(
                &conn,
                "daemon ownership lock protects startup arbitration",
                "memory::daemon-lock",
                &[1.0, 0.0, 0.0, 0.0, 0.0],
            );
        }

        let _response = execute_unified_recall(
            &state,
            "daemon ownership lock",
            240,
            6,
            "codex",
            &solo_ctx(),
            None,
        )
        .await
        .expect("unified recall should succeed");

        let conn = state.db.lock().await;
        let event = latest_recall_query_event(&conn);
        let shadow_semantic = &event["shadow_semantic"];
        assert_eq!(event["semantic_route"]["mode"].as_str(), Some("baseline"));
        assert_eq!(shadow_semantic["status"].as_str(), Some("unavailable"));
        assert_eq!(
            shadow_semantic["reason"].as_str(),
            Some("query_embedding_unavailable")
        );
        assert!(
            shadow_semantic["baselineTopSources"].is_null(),
            "telemetry event payload should not contain baseline source arrays"
        );
        assert!(
            shadow_semantic["shadowTopSources"].is_null(),
            "telemetry event payload should not contain shadow source arrays"
        );
    }

    #[tokio::test]
    async fn execute_unified_recall_logs_shadow_semantic_skip_on_cache_hit() {
        let state = shared_test_state();
        let query = "daemon ownership lock";
        let scope_key = recall_scope_key("codex", &solo_ctx());
        let cached_item = RecallItem {
            source: "memory::daemon-lock".to_string(),
            relevance: 0.91,
            excerpt: "daemon ownership lock protects startup arbitration".to_string(),
            method: "semantic".to_string(),
            tokens: Some(16),
            entropy: None,
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
        };
        {
            let mut pre_cache = state.pre_cache.lock().await;
            pre_cache.insert(
                scope_key,
                crate::state::PreCacheEntry {
                    query: query.to_string(),
                    results: json!([recall_to_json(cached_item)]),
                    expires_at: chrono::Utc::now().timestamp_millis() + 60_000,
                },
            );
        }

        let response = execute_unified_recall(&state, query, 240, 4, "codex", &solo_ctx(), None)
            .await
            .expect("cached unified recall should succeed");
        assert_eq!(response["cached"].as_bool(), Some(true));

        let conn = state.db.lock().await;
        let event = latest_recall_query_event(&conn);
        assert_eq!(event["cached"].as_bool(), Some(true));
        assert_eq!(event["semantic_route"]["mode"].as_str(), Some("baseline"));
        assert_eq!(event["shadow_semantic"]["status"].as_str(), Some("skipped"));
        assert_eq!(
            event["shadow_semantic"]["reason"].as_str(),
            Some("cache_hit")
        );
    }

    #[tokio::test]
    async fn execute_unified_recall_logs_shadow_semantic_summary_in_headlines_mode() {
        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            insert_memory_with_embedding(
                &conn,
                "daemon ownership lock protects startup arbitration",
                "memory::daemon-lock",
                &[1.0, 0.0, 0.0, 0.0, 0.0],
            );
        }

        let response = execute_unified_recall(
            &state,
            "daemon ownership lock",
            0,
            6,
            "codex",
            &solo_ctx(),
            None,
        )
        .await
        .expect("headlines unified recall should succeed");
        assert_eq!(response["mode"].as_str(), Some("headlines"));

        let conn = state.db.lock().await;
        let event = latest_recall_query_event(&conn);
        assert_eq!(event["cached"].as_bool(), Some(false));
        assert_eq!(event["mode"].as_str(), Some("headlines"));
        assert_eq!(event["semantic_route"]["mode"].as_str(), Some("baseline"));
        let shadow_semantic = &event["shadow_semantic"];
        assert_eq!(shadow_semantic["status"].as_str(), Some("unavailable"));
        assert_eq!(
            shadow_semantic["reason"].as_str(),
            Some("query_embedding_unavailable")
        );
        assert!(
            shadow_semantic["baselineTopSources"].is_null(),
            "telemetry event payload should not contain baseline source arrays"
        );
        assert!(
            shadow_semantic["shadowTopSources"].is_null(),
            "telemetry event payload should not contain shadow source arrays"
        );
    }

    #[tokio::test]
    async fn execute_unified_recall_headlines_reports_token_usage_and_savings() {
        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
                 VALUES (?1, 'memory::headlines-token-usage', 'note', 'active', 0.92, datetime('now'), datetime('now'))",
                params!["daemon ownership lock arbitration details and heartbeat cadence"],
            )
            .unwrap();
        }

        let response = execute_unified_recall(
            &state,
            "daemon ownership lock heartbeat",
            0,
            5,
            "codex",
            &solo_ctx(),
            None,
        )
        .await
        .expect("headlines unified recall should succeed");
        let spent = response["spent"]
            .as_u64()
            .expect("headlines response should include spent");
        let saved = response["saved"]
            .as_i64()
            .expect("headlines response should include saved");
        assert!(
            spent > 0,
            "headlines usage should report non-zero source-token usage"
        );
        assert!(saved >= 0, "headlines savings should be non-negative");
        assert!(
            response["tokenUsageLine"]
                .as_str()
                .unwrap_or_default()
                .contains("headlines mode"),
            "headlines response should include headlines-mode usage line"
        );

        let conn = state.db.lock().await;
        let event = latest_recall_query_event(&conn);
        assert_eq!(event["spent"].as_u64(), Some(spent));
        assert_eq!(event["saved"].as_i64(), Some(saved));
    }

    // ── RRF fusion tests ───────────────────────────────────────────

    #[test]
    fn test_rrf_fuse_single_list() {
        // Single list: ranks 0,1,2 with k=60
        let list = vec![(10, 0.9), (20, 0.7), (30, 0.5)];
        let result = rrf_fuse(&[list], 60.0);
        assert_eq!(result.len(), 3);
        // Item at rank 0 should be first (highest fused score)
        assert_eq!(result[0].0, 10);
        assert_eq!(result[1].0, 20);
        assert_eq!(result[2].0, 30);
        // Score for rank-0 item: 1/(60+0+1) = 1/61
        let expected = 1.0 / 61.0;
        assert!(
            (result[0].1 - expected).abs() < 1e-10,
            "expected {expected}, got {}",
            result[0].1
        );
    }

    #[test]
    fn test_rrf_fuse_two_lists_agreement() {
        // Item 10 is rank-0 in both lists -- should score highest
        let list_a = vec![(10, 0.9), (20, 0.5)];
        let list_b = vec![(10, 0.8), (30, 0.4)];
        let result = rrf_fuse(&[list_a, list_b], 60.0);
        assert_eq!(result[0].0, 10);
        // Score = 1/(60+0+1) + 1/(60+0+1) = 2/61
        let expected = 2.0 / 61.0;
        assert!((result[0].1 - expected).abs() < 1e-10);
    }

    #[test]
    fn test_rrf_fuse_promotes_consistent_middle() {
        // Verify RRF correctly weights cross-list agreement vs single-list high rank.
        //
        // list_a = [(10,_), (20,_), (30,_)]: rank0=10, rank1=20, rank2=30
        // list_b = [(30,_), (20,_)]:          rank0=30, rank1=20
        //
        // RRF scores (k=60):
        //   item10: 1/(60+0+1)           = 1/61  ≈ 0.016393
        //   item20: 1/(60+1+1)+1/(60+1+1) = 2/62  ≈ 0.032258
        //   item30: 1/(60+2+1)+1/(60+0+1) = 1/63+1/61 ≈ 0.032266
        //
        // item30 beats item20 by 0.000008 (rank-0 bonus in list_b outweighs
        // rank-2 penalty in list_a vs rank-1 in both for item20).
        // Both item20 and item30 score ~2x item10 (cross-list agreement crushes lone rank-0).
        let list_a = vec![(10, 0.9), (20, 0.6), (30, 0.2)];
        let list_b = vec![(30, 0.8), (20, 0.5)];
        let result = rrf_fuse(&[list_a, list_b], 60.0);
        assert_eq!(result.len(), 3);

        // item 10 (only in list_a at rank 0) should be last -- single-list penalty
        let pos_10 = result.iter().position(|(id, _)| *id == 10).unwrap();
        let pos_20 = result.iter().position(|(id, _)| *id == 20).unwrap();
        let pos_30 = result.iter().position(|(id, _)| *id == 30).unwrap();
        assert!(
            pos_10 > pos_20,
            "item10 (rank-0 in one list) should lose to item20 (rank-1 in both)"
        );
        assert!(
            pos_10 > pos_30,
            "item10 (rank-0 in one list) should lose to item30 (rank-0 + rank-2)"
        );

        // Both multi-list items score well above single-list item10
        let score_10 = result[pos_10].1;
        let score_20 = result[pos_20].1;
        assert!(
            score_20 > score_10 * 1.9,
            "item20 cross-list score should be ~2x item10"
        );
    }

    #[test]
    fn test_rrf_fuse_empty_lists() {
        let result = rrf_fuse(&[], 60.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rrf_fuse_single_empty_list() {
        let result = rrf_fuse(&[vec![]], 60.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rrf_fuse_weighted_prefers_heavier_ranker() {
        let keyword_list = vec![(1, 0.99)];
        let semantic_list = vec![(2, 0.99)];

        let result = rrf_fuse_weighted(&[keyword_list, semantic_list], &[1.4, 0.6], 60.0);
        assert_eq!(result[0].0, 1);
        assert!(result[0].1 > result[1].1);
    }

    #[test]
    fn test_rrf_fuse_weighted_ignores_non_finite_weights() {
        let keyword_list = vec![(1, 0.99)];
        let semantic_list = vec![(2, 0.99)];

        let result = rrf_fuse_weighted(&[keyword_list, semantic_list], &[f64::NAN, 1.0], 60.0);
        assert_eq!(result, vec![(2, 1.0 / 61.0)]);
    }

    #[test]
    fn test_rrf_fuse_weighted_falls_back_for_non_finite_k() {
        let result = rrf_fuse_weighted(&[vec![(1, 0.99)]], &[1.0], f64::NAN);

        assert_eq!(result, vec![(1, 1.0 / 61.0)]);
        assert!(result[0].1.is_finite());
    }

    #[test]
    fn test_adaptive_rrf_weights_bias_short_exact_queries_toward_keyword() {
        let weights = adaptive_rrf_weights("auth.rs", None, true);
        assert!(weights.keyword > weights.semantic);
    }

    #[test]
    fn test_adaptive_rrf_weights_bias_long_natural_queries_toward_semantic() {
        let weights = adaptive_rrf_weights(
            "How does Cortex preserve session truth after a daemon restart and reconnect?",
            None,
            true,
        );
        assert!(weights.semantic > weights.keyword);
    }

    #[test]
    fn test_adaptive_rrf_weights_disable_semantic_when_unavailable() {
        let weights = adaptive_rrf_weights("codex recall", None, false);
        assert_eq!(
            weights,
            FusionWeights {
                keyword: 1.0,
                semantic: 0.0,
            }
        );
    }

    #[test]
    fn test_adaptive_fallback_weights_bias_short_exact_queries_toward_keyword() {
        let weights = adaptive_fallback_ranking_weights("auth.rs", 2);
        assert!(weights.keyword > weights.score);
        assert!(weights.keyword > weights.recency);
        assert!(weights.keyword > weights.retrieval);
    }

    #[test]
    fn test_adaptive_fallback_weights_bias_natural_queries_toward_non_keyword_signals() {
        let weights = adaptive_fallback_ranking_weights(
            "How does Cortex preserve session truth after a daemon restart and reconnect?",
            6,
        );
        assert!(weights.keyword < 0.40);
        let total = weights.keyword + weights.score + weights.recency + weights.retrieval;
        assert!((total - 1.0).abs() < 1e-9);
    }

    // ── compound scoring tests (Task 1.4) ──────────────────────────

    #[test]
    fn test_days_since() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let days_today = days_since(&today);

        // Today should be very close to 0 (within 1 minute tolerance)
        assert!(
            days_today < 0.001,
            "days_since(today) should be ~0, got {}",
            days_today
        );

        //Yesterday (approximately)
        let yesterday = (now - chrono::Duration::days(1)).to_rfc3339();
        let days_yesterday = days_since(&yesterday);
        assert!(
            (days_yesterday - 1.0).abs() < 0.02,
            "days_since(yesterday) should be ~1.0, got {}",
            days_yesterday
        );

        // Invalid timestamp should return MAX
        let days_invalid = days_since("invalid-date");
        assert_eq!(
            days_invalid,
            f64::MAX,
            "days_since(invalid) should return MAX"
        );
    }

    #[test]
    fn test_normalize() {
        // Typical range: 0-100
        assert!((normalize(0.0) - 0.0).abs() < f64::EPSILON);
        assert!((normalize(50.0) - 0.5).abs() < f64::EPSILON);
        assert!((normalize(100.0) - 1.0).abs() < f64::EPSILON);
        assert!((normalize(0.6) - 0.6).abs() < f64::EPSILON);

        // Clamp above 100
        assert_eq!(normalize(150.0), 1.0);

        // Clamp below 0
        assert_eq!(normalize(-10.0), 0.0);

        // Reject non-finite values before clamping.
        assert_eq!(normalize(f64::NAN), 0.0);
        assert_eq!(normalize(f64::INFINITY), 0.0);
    }

    #[test]
    fn test_blend_importance_uses_trust_when_available() {
        let low_trust = blend_importance(Some(0.6), Some(0.2));
        let high_trust = blend_importance(Some(0.6), Some(0.9));
        assert!(
            high_trust > low_trust,
            "higher trust should raise effective importance"
        );
        assert_eq!(
            blend_importance(Some(0.42), None),
            blend_importance(Some(0.42), Some(0.42))
        );
    }

    #[test]
    fn test_blend_importance_rejects_non_finite_values() {
        assert_eq!(blend_importance(Some(f64::NAN), None), 0.0);
        assert_eq!(
            blend_importance(Some(0.42), Some(f64::INFINITY)),
            blend_importance(Some(0.42), None)
        );
    }

    #[test]
    fn test_compound_score() {
        let now = chrono::Utc::now();
        let today = now.to_rfc3339();
        let week_ago = (now - chrono::Duration::weeks(1)).to_rfc3339();
        let month_ago = (now - chrono::Duration::days(30)).to_rfc3339();

        // High RRF, high importance, recent: should score well
        let score_high = compound_score(0.1, 100.0, &today);
        assert!(
            score_high > 0.06,
            "high RRF + high importance + recent should score well, got {}",
            score_high
        );

        // Low RRF, low importance, old: should score poorly (recency factor dominates but is low for old items)
        let score_low = compound_score(0.001, 0.0, &month_ago);
        assert!(
            score_low < 0.08,
            "low RRF + low importance + old should score poorly, got {}",
            score_low
        );

        // Recency decay: same RRF/imp, older date = lower score
        let score_today = compound_score(0.05, 50.0, &today);
        let score_week = compound_score(0.05, 50.0, &week_ago);
        assert!(
            score_today > score_week,
            "same RRF/imp, today should score > week ago"
        );
    }

    // ── synonym expansion tests ────────────────────────────────────

    #[test]
    fn test_synonym_expansion_func() {
        let kw = extract_search_keywords_with_synonyms("func error db");
        assert!(kw.contains(&"function".to_string()), "func -> function");
        assert!(kw.contains(&"error".to_string()));
        assert!(kw.contains(&"database".to_string()), "db -> database");
    }

    #[test]
    fn test_synonym_expansion_personal_memory_aliases() {
        let kw = extract_search_keywords_with_synonyms("lastname repainted walls color gray");
        assert!(
            kw.contains(&"surname".to_string()),
            "lastname should expand to surname"
        );
        assert!(
            kw.contains(&"paint".to_string()),
            "repainted should expand to paint"
        );
        assert!(
            kw.contains(&"wall".to_string()),
            "walls should expand to wall"
        );
        assert!(
            kw.contains(&"colour".to_string()),
            "color should expand to colour"
        );
        assert!(
            kw.contains(&"grey".to_string()),
            "gray should expand to grey"
        );
    }

    #[test]
    fn test_synonym_expansion_no_duplicates() {
        // "function" is already full form -- should not duplicate
        let kw = extract_search_keywords_with_synonyms("function");
        let count = kw.iter().filter(|w| *w == "function").count();
        assert_eq!(count, 1, "no duplicate expansions");
    }

    #[test]
    fn test_fts_query_joins_groups_with_and() {
        let groups = build_search_term_groups("func db timeout");
        let query = build_fts_query(&groups);
        assert!(query.contains(" AND "), "fts groups should be AND-joined");
        assert!(
            query.contains("(\"function\" OR \"func\")"),
            "func should expand to function alternates"
        );
        assert!(
            query.contains("(\"database\" OR \"db\")"),
            "db should expand to database alternates"
        );
    }

    #[test]
    fn test_build_search_term_groups_filters_low_signal_terms_for_natural_queries() {
        let groups = build_search_term_groups("Where did I attend for my study abroad program?");
        let flattened: Vec<String> = groups.into_iter().flatten().collect();
        assert!(
            !flattened
                .iter()
                .any(|token| token == "where" || token == "did" || token == "my"),
            "natural query token groups should trim low-signal filler terms"
        );
        assert!(
            flattened
                .iter()
                .any(|token| token == "study" || token == "abroad"),
            "natural query token groups should retain intent-bearing terms"
        );
        assert!(
            flattened
                .iter()
                .any(|token| token == "attend" || token == "attended"),
            "study-abroad intent should include attendance aliases"
        );
    }

    #[test]
    fn test_query_focused_excerpt_finds_late_match() {
        let prefix = "x".repeat(260);
        let text = format!("{prefix} I graduated with a degree in Business Administration.");
        let excerpt = query_focused_excerpt(&text, "What degree did I graduate with?", 120);
        assert!(
            excerpt.to_lowercase().contains("graduated"),
            "excerpt should contain matched term"
        );
        assert!(
            excerpt.contains("Business Administration"),
            "excerpt should preserve local factual span"
        );
    }

    #[test]
    fn test_query_focused_excerpt_matches_synonym_expansion() {
        let prefix = "x".repeat(240);
        let text =
            format!("{prefix} database timeout recovery keeps the daemon stable during reconnect.");
        let excerpt = query_focused_excerpt(&text, "db timeout", 110);
        let lower = excerpt.to_ascii_lowercase();
        assert!(
            lower.contains("database") && lower.contains("timeout"),
            "excerpt should center on the synonym-expanded span, got {excerpt:?}"
        );
    }

    #[test]
    fn test_query_focused_excerpt_prefers_user_answer_block_for_qa_memory() {
        let text = "[assistant-question] What did I buy for my sister's birthday gift? [user-answer] A yellow dress with silver buttons from Nordstrom downtown.";
        let excerpt =
            query_focused_excerpt(text, "What did I buy for my sister's birthday gift?", 96);
        let lower = excerpt.to_ascii_lowercase();
        assert!(
            lower.contains("[user-answer]"),
            "excerpt should prioritize user-answer span, got {excerpt:?}"
        );
        assert!(
            lower.contains("yellow dress"),
            "excerpt should preserve the concrete answer detail, got {excerpt:?}"
        );
    }

    #[test]
    fn test_apply_semantic_budget_skips_redundant_candidates_without_new_coverage() {
        let build_item = |source: &str, relevance: f64, excerpt: &str| RecallItem {
            source: source.to_string(),
            relevance,
            excerpt: excerpt.to_string(),
            method: "hybrid".to_string(),
            tokens: None,
            entropy: None,
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
        };

        let raw = vec![
            build_item(
                "memory::daemon-policy-a",
                0.93,
                "daemon startup ownership lock lease arbitration prevents duplicate startup and keeps one active owner during plugin reconnect",
            ),
            build_item(
                "memory::daemon-policy-b",
                0.91,
                "daemon startup ownership lock lease arbitration prevents duplicate startup and keeps one active owner during plugin reconnect loop",
            ),
            build_item(
                "memory::heartbeat",
                0.84,
                "plugin heartbeat monitor reports attach state and reconnect health for claude daemon sessions",
            ),
        ];

        let results =
            apply_semantic_budget(raw, 320, "daemon startup ownership lock lease heartbeat");
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();
        assert!(
            sources.contains(&"memory::daemon-policy-a"),
            "top policy candidate should remain selected: {sources:?}"
        );
        assert!(
            !sources.contains(&"memory::daemon-policy-b"),
            "near-duplicate candidate without new coverage should be dropped: {sources:?}"
        );
        assert!(
            sources.contains(&"memory::heartbeat"),
            "distinct candidate covering remaining query intent should be retained: {sources:?}"
        );
    }

    #[test]
    fn test_apply_semantic_budget_keeps_similar_candidate_with_new_query_term_coverage() {
        let build_item = |source: &str, relevance: f64, excerpt: &str| RecallItem {
            source: source.to_string(),
            relevance,
            excerpt: excerpt.to_string(),
            method: "hybrid".to_string(),
            tokens: None,
            entropy: None,
            family_members: Vec::new(),
            collapsed_sources: Vec::new(),
            collapsed_source_scores: Vec::new(),
        };

        let raw = vec![
            build_item(
                "memory::daemon-policy-a",
                0.93,
                "daemon startup ownership lock lease arbitration keeps one active owner during reconnect",
            ),
            build_item(
                "memory::daemon-policy-b",
                0.90,
                "daemon startup ownership lock lease arbitration keeps one active owner during reconnect and heartbeat checks",
            ),
        ];

        let results =
            apply_semantic_budget(raw, 320, "daemon startup ownership lock lease heartbeat");
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();
        assert!(
            sources.contains(&"memory::daemon-policy-a"),
            "primary candidate should remain selected"
        );
        assert!(
            sources.contains(&"memory::daemon-policy-b"),
            "candidate adding new query-term coverage should not be dropped as redundant"
        );
    }

    #[test]
    fn test_search_memories_fallback_matches_synonym_term_groups() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, 'memory::db-timeout', 'note', 'active', 0.9, datetime('now'), datetime('now'))",
            params!["database timeout recovery keeps reconnect stable"],
        )
        .unwrap();

        let results = search_memories_fallback(&conn, "db timeout", 5, None)
            .expect("memory fallback should succeed");
        assert!(
            results
                .iter()
                .any(|item| item.source == "memory::db-timeout"),
            "fallback should match synonym-expanded memory text"
        );
        assert_eq!(results[0].matched_keywords, 2);
    }

    #[test]
    fn test_search_memories_fallback_study_abroad_query_finds_attended_location_memory() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, 'memory::study-abroad-location', 'note', 'active', 0.88, datetime('now'), datetime('now'))",
            params!["I attended it in Australia."],
        )
        .unwrap();

        let results = search_memories_fallback(
            &conn,
            "Where did I attend for my study abroad program?",
            5,
            None,
        )
        .expect("study-abroad fallback query should succeed");
        assert!(
            results
                .iter()
                .any(|item| item.source == "memory::study-abroad-location"),
            "study-abroad fallback should retrieve attended-location memory"
        );
    }

    #[test]
    fn test_search_memories_fallback_empty_term_groups_prefers_score_signal() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES ('Older high-signal recovery policy', 'memory::older-high-score', 'note', 'active', 1.0, 1.0, datetime('now', '-2 day'), datetime('now', '-2 day'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES ('Newer low-signal note', 'memory::new-low-score', 'note', 'active', 0.1, 0.1, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results =
            search_memories_fallback(&conn, "a", 5, None).expect("memory fallback should succeed");
        assert_eq!(
            results[0].source, "memory::older-high-score",
            "empty-term fallback should rank by retained score signal before recency"
        );
    }

    #[test]
    fn test_search_memories_fallback_breaks_ties_by_source() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES ('daemon lock ownership flow', 'memory::b-source', 'note', 'active', 0.7, 0.7, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES ('daemon lock ownership flow', 'memory::a-source', 'note', 'active', 0.7, 0.7, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_memories_fallback(&conn, "daemon lock", 5, None)
            .expect("memory fallback should succeed");
        assert_eq!(results[0].source, "memory::a-source");
        assert_eq!(results[1].source, "memory::b-source");
    }

    #[test]
    fn test_search_memories_fallback_honors_source_prefix_scope() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'scope::memory::hit', 'note', 'active', 0.9, 0.9, datetime('now'), datetime('now'))",
            params!["daemon lock ownership flow"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'other::memory::noise', 'note', 'active', 0.9, 0.9, datetime('now'), datetime('now'))",
            params!["daemon lock ownership flow"],
        )
        .unwrap();

        let results = search_memories_fallback(&conn, "daemon lock", 5, Some("scope::"))
            .expect("memory fallback source-prefix query should succeed");
        assert!(
            results
                .iter()
                .all(|item| item.source.starts_with("scope::")),
            "fallback memory search should keep only scoped sources"
        );
        assert!(
            results
                .iter()
                .any(|item| item.source == "scope::memory::hit"),
            "scoped memory should remain in results"
        );
    }

    #[test]
    fn test_search_memories_fts_scoring_counts_synonym_term_groups() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, 'memory::db-timeout-fts', 'note', 'active', 0.9, datetime('now'), datetime('now'))",
            params!["database timeout recovery keeps reconnect stable"],
        )
        .unwrap();

        let results =
            search_memories(&conn, "db timeout", 5, None).expect("memory search should succeed");
        assert!(
            results
                .iter()
                .any(|item| item.source == "memory::db-timeout-fts"),
            "fts search should match synonym-expanded memory text"
        );
        assert_eq!(results[0].matched_keywords, 2);
    }

    #[test]
    fn test_search_decisions_fallback_matches_synonym_term_groups() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, created_at, updated_at)
             VALUES (?1, 'decision::authz-policy', 'active', 0.85, datetime('now'), datetime('now'))",
            params!["authorization policy should reject unknown callers by default"],
        )
        .unwrap();

        let results = search_decisions_fallback(&conn, "authz policy", 5, None)
            .expect("decision fallback should succeed");
        assert!(
            results
                .iter()
                .any(|item| item.source == "decision::authz-policy"),
            "fallback should match synonym-expanded decision text"
        );
        assert_eq!(results[0].matched_keywords, 2);
    }

    #[test]
    fn test_search_decisions_fallback_empty_term_groups_prefers_score_signal() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES ('Older high-confidence daemon rule', 'decision::older-high-score', 'active', 1.0, 1.0, datetime('now', '-2 day'), datetime('now', '-2 day'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES ('Newer low-confidence rule', 'decision::new-low-score', 'active', 0.1, 0.1, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_decisions_fallback(&conn, "a", 5, None)
            .expect("decision fallback should succeed");
        assert_eq!(
            results[0].source, "decision::older-high-score",
            "empty-term fallback should rank by retained score signal before recency"
        );
    }

    #[test]
    fn test_search_decisions_fallback_honors_source_prefix_scope() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'scope::decision::hit', 'active', 0.9, 0.9, datetime('now'), datetime('now'))",
            params!["daemon lock ownership rule"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'other::decision::noise', 'active', 0.9, 0.9, datetime('now'), datetime('now'))",
            params!["daemon lock ownership rule"],
        )
        .unwrap();

        let results = search_decisions_fallback(&conn, "daemon lock", 5, Some("scope::"))
            .expect("decision fallback source-prefix query should succeed");
        assert!(
            results
                .iter()
                .all(|item| item.source.starts_with("scope::")),
            "fallback decision search should keep only scoped sources"
        );
        assert!(
            results
                .iter()
                .any(|item| item.source == "scope::decision::hit"),
            "scoped decision should remain in results"
        );
    }

    #[test]
    fn test_search_decisions_fts_scoring_counts_synonym_term_groups() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, created_at, updated_at)
             VALUES (?1, 'decision::authz-policy-fts', 'active', 0.85, datetime('now'), datetime('now'))",
            params!["authorization policy should reject unknown callers by default"],
        )
        .unwrap();

        let results = search_decisions(&conn, "authz policy", 5, None)
            .expect("decision search should succeed");
        assert!(
            results
                .iter()
                .any(|item| item.source == "decision::authz-policy-fts"),
            "fts search should match synonym-expanded decision text"
        );
        assert_eq!(results[0].matched_keywords, 2);
    }

    #[test]
    fn test_search_memories_bm25_prefers_text_signal_over_source_metadata() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::text-rank', 'note', 'active', 1.0, 1.0, datetime('now'), datetime('now'))",
            params!["daemon ownership lock handoff policy keeps restarts stable"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::daemon-ownership-lock-path', 'note', 'active', 1.0, 1.0, datetime('now'), datetime('now'))",
            params!["unrelated metadata only"],
        )
        .unwrap();

        let results = search_memories(&conn, "daemon ownership lock", 1, None)
            .expect("memory search should succeed");
        assert_eq!(
            results[0].source, "memory::text-rank",
            "bm25 tuning should favor text-heavy matches when limit truncates candidates"
        );
    }

    #[test]
    fn test_search_decisions_bm25_prefers_decision_text_over_context_metadata() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'decision::text-rank', 'active', 1.0, 1.0, datetime('now'), datetime('now'))",
            params!["daemon ownership lock handoff policy remains strict"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'decision::daemon-ownership-lock-context', 'active', 1.0, 1.0, datetime('now'), datetime('now'))",
            params!["unrelated context metadata only"],
        )
        .unwrap();

        let results = search_decisions(&conn, "daemon ownership lock", 1, None)
            .expect("decision search should succeed");
        assert_eq!(
            results[0].source, "decision::text-rank",
            "bm25 tuning should prioritize decision-text evidence over context-only matches"
        );
    }

    #[test]
    fn test_fit_excerpt_to_remaining_budget_keeps_query_focus() {
        let prefix = "x".repeat(220);
        let text = format!(
            "{prefix} daemon ownership lock arbitration prevents split-brain after parent death."
        );
        let (excerpt, tokens) = fit_excerpt_to_remaining_budget(
            "memory::daemon-lock",
            &text,
            "daemon ownership lock",
            220,
            40,
        )
        .expect("expected source + excerpt to fit");
        assert!(tokens <= 40, "tokens should fit remaining budget");
        assert!(
            excerpt.to_ascii_lowercase().contains("daemon")
                || excerpt.to_ascii_lowercase().contains("ownership"),
            "budgeted excerpt should preserve query-bearing span"
        );
    }

    #[test]
    fn test_run_budget_recall_enforces_total_token_cap() {
        let mut conn = test_conn();
        for idx in 0..8 {
            let source = format!("memory::daemon-lock-{idx}");
            let text = format!(
                "{} daemon ownership lock handoff requires pid start-time checks and stale lock recovery.",
                "warmup ".repeat(18)
            );
            conn.execute(
                "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
                 VALUES (?1, ?2, 'note', 'active', 1.0, datetime('now'), datetime('now'))",
                params![text, source],
            )
            .unwrap();
        }

        let results = run_budget_recall(
            &mut conn,
            "daemon ownership lock",
            200,
            10,
            &solo_ctx(),
            None,
        )
        .expect("budget recall should succeed");
        let spent: usize = results
            .iter()
            .map(|item| {
                item.tokens
                    .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
            })
            .sum();

        assert!(!results.is_empty(), "expected at least one recall result");
        assert!(
            spent <= 200,
            "total tokens should not exceed budget: {spent}"
        );
    }

    #[test]
    fn test_run_budget_recall_keeps_late_query_span_when_clipped() {
        let mut conn = test_conn();
        let text = format!(
            "{} ownership lock handoff after sleep wake requires parent liveness gating.",
            "prefix ".repeat(40)
        );
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at)
             VALUES (?1, 'memory::sleep-wake-lock', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            params![text],
        )
        .unwrap();

        let results = run_budget_recall(
            &mut conn,
            "ownership lock handoff",
            90,
            5,
            &solo_ctx(),
            None,
        )
        .expect("budget recall should succeed");
        assert!(!results.is_empty(), "expected low-budget result");
        assert!(
            results[0]
                .excerpt
                .to_ascii_lowercase()
                .contains("ownership")
                || results[0].excerpt.to_ascii_lowercase().contains("lock"),
            "top result should keep query-bearing span under clipping"
        );
    }

    #[test]
    fn test_budget_recall_adds_associative_source_when_co_occurrence_is_strong() {
        let mut conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::daemon-lock', 'note', 'active', 0.9, 0.92, datetime('now'), datetime('now'))",
            params!["daemon ownership lock lease protects startup arbitration and stale pid recovery"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::service-ensure', 'note', 'active', 0.85, 0.88, datetime('now'), datetime('now'))",
            params!["service ensure keeps one daemon active before mcp attach"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::recovery-playbook', 'note', 'active', 0.25, 0.25, datetime('now'), datetime('now'))",
            params!["snapshot pruning and wal checkpoint cadence for cold-start recovery"],
        )
        .unwrap();

        for _ in 0..6 {
            crate::co_occurrence::record(
                &conn,
                &[
                    "memory::daemon-lock".to_string(),
                    "memory::recovery-playbook".to_string(),
                ],
            )
            .unwrap();
        }

        let results = run_budget_recall(
            &mut conn,
            "daemon ownership lock",
            700,
            8,
            &solo_ctx(),
            None,
        )
        .expect("budget recall should succeed");

        let assoc = results
            .iter()
            .find(|item| item.source == "memory::recovery-playbook");
        assert!(
            assoc.is_some(),
            "expected co-occurrence linked source to be included; got results={results:?}"
        );
        assert_eq!(
            assoc.unwrap().method,
            "associative",
            "linked source should be explicitly tagged as associative"
        );
    }

    #[test]
    fn budget_recall_trace_reports_family_compaction_after_associative_merge() {
        let mut conn = test_conn();
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];
        let (_crystal_id, crystal_key, _member_sources) = insert_crystal_with_memory_members(
            &conn,
            "daemon lifecycle",
            "Daemon lifecycle summary covers lease renewal and recovery.",
            &query_vector,
            &[
                (
                    "Daemon lifecycle lease renewal keeps ownership stable.",
                    "memory::daemon-lifecycle",
                    &query_vector,
                ),
                (
                    "Plugin reconnect heartbeat stops duplicate daemon startup.",
                    "memory::plugin-heartbeat",
                    &[0.97, 0.03, 0.0, 0.0, 0.0],
                ),
            ],
        );
        insert_memory_with_embedding(
            &conn,
            "Recovery dashboard shows lock state and daemon readiness.",
            "memory::recovery-dashboard",
            &[0.94, 0.06, 0.0, 0.0, 0.0],
        );

        for _ in 0..6 {
            crate::co_occurrence::record(
                &conn,
                &[
                    crystal_key.clone(),
                    "memory::plugin-heartbeat".to_string(),
                    "memory::recovery-dashboard".to_string(),
                ],
            )
            .unwrap();
        }

        let trace = run_budget_recall_trace_with_query_vector(
            &mut conn,
            "daemon lifecycle recovery",
            320,
            8,
            Some(&query_vector),
            &solo_ctx(),
            None,
            None,
        )
        .expect("budget trace should succeed");

        assert_eq!(
            trace.pre_compaction_candidate_count,
            trace.candidate_pool.len() + 1,
            "one associative family sibling should be compacted before packing"
        );
        assert_eq!(trace.family_compactions.len(), 1);
        assert_eq!(trace.family_compactions[0].family_key, crystal_key);
        assert_eq!(trace.family_compactions[0].kept_source, crystal_key);
        assert!(
            trace.family_compactions[0]
                .dropped_sources
                .contains(&"memory::plugin-heartbeat".to_string()),
            "associative family sibling should be reported as compacted"
        );
        assert!(
            trace
                .candidate_pool
                .iter()
                .all(|item| item.source != "memory::plugin-heartbeat"),
            "compacted sibling should not survive in candidate pool"
        );
        assert!(
            trace
                .candidate_pool
                .iter()
                .any(|item| item.source == "memory::recovery-dashboard"),
            "unrelated high-signal context should remain after compaction"
        );
    }

    #[tokio::test]
    async fn execute_recall_policy_explain_reports_family_compaction_after_associative_merge() {
        let state = shared_test_state();
        let query_vector = [1.0, 0.0, 0.0, 0.0, 0.0];
        let crystal_key = {
            let conn = state.db.lock().await;
            let (_crystal_id, crystal_key, _member_sources) = insert_crystal_with_memory_members(
                &conn,
                "daemon lifecycle",
                "Daemon lifecycle summary covers lease renewal and recovery.",
                &query_vector,
                &[
                    (
                        "Daemon lifecycle lease renewal keeps ownership stable.",
                        "memory::daemon-lifecycle",
                        &query_vector,
                    ),
                    (
                        "Plugin reconnect heartbeat stops duplicate daemon startup.",
                        "memory::plugin-heartbeat",
                        &[0.97, 0.03, 0.0, 0.0, 0.0],
                    ),
                ],
            );
            insert_memory_with_embedding(
                &conn,
                "Recovery dashboard shows lock state and daemon readiness.",
                "memory::recovery-dashboard",
                &[0.94, 0.06, 0.0, 0.0, 0.0],
            );

            for _ in 0..6 {
                crate::co_occurrence::record(
                    &conn,
                    &[
                        crystal_key.clone(),
                        "memory::plugin-heartbeat".to_string(),
                        "memory::recovery-dashboard".to_string(),
                    ],
                )
                .unwrap();
            }

            crystal_key
        };

        let explain = execute_recall_policy_explain_inner(
            &state,
            "daemon lifecycle recovery",
            320,
            8,
            "codex",
            &solo_ctx(),
            None,
            8,
            Some(&query_vector),
        )
        .await
        .expect("policy explain should succeed");

        let family_compactions = explain["explain"]["familyCompactions"]
            .as_array()
            .expect("family compactions should be present");
        assert_eq!(family_compactions.len(), 1);
        assert_eq!(
            family_compactions[0]["familyKey"].as_str(),
            Some(crystal_key.as_str())
        );
        assert_eq!(
            family_compactions[0]["keptSource"].as_str(),
            Some(crystal_key.as_str())
        );
        assert!(
            family_compactions[0]["droppedSources"]
                .as_array()
                .expect("dropped sources should be present")
                .iter()
                .any(|value| value.as_str() == Some("memory::plugin-heartbeat")),
            "compacted sibling should be reported in explain output"
        );

        assert_eq!(
            explain["policy"]["budgetReasoning"]["familyCompactedCount"].as_u64(),
            Some(1)
        );
        let post_budget_dropped = explain["policy"]["budgetReasoning"]["droppedCount"]
            .as_u64()
            .expect("post-budget dropped count should be numeric");
        assert_eq!(
            explain["policy"]["budgetReasoning"]["totalPreBudgetDrops"].as_u64(),
            Some(post_budget_dropped + 1)
        );

        let dropped_candidates = explain["explain"]["droppedCandidates"]
            .as_array()
            .expect("dropped candidates should be present");
        assert!(
            dropped_candidates
                .iter()
                .all(|candidate| candidate["source"].as_str() != Some("memory::plugin-heartbeat")),
            "family compaction should not misreport the sibling as a post-budget drop"
        );

        let returned = explain["explain"]["returned"]
            .as_array()
            .expect("returned explain entries should be present");
        assert!(
            !returned.is_empty(),
            "policy explain should include at least one returned result"
        );
        let ranking_factors = &returned[0]["rankingFactors"];
        assert!(
            ranking_factors.get("entityMatches").is_some(),
            "ranking factors should expose entity match signal"
        );
        assert!(
            ranking_factors.get("entityOverlap").is_some(),
            "ranking factors should expose entity overlap signal"
        );
        assert!(
            ranking_factors.get("entityBoost").is_some(),
            "ranking factors should expose entity boost contribution"
        );

        let shadow_semantic = &explain["explain"]["shadowSemantic"];
        assert_eq!(shadow_semantic["enabled"].as_bool(), Some(true));
        let status = shadow_semantic["status"]
            .as_str()
            .expect("shadow semantic status should be present");
        assert!(
            matches!(status, "ok" | "unavailable" | "error"),
            "unexpected shadow semantic status: {}",
            shadow_semantic
        );
        if status == "ok" {
            let overlap_count = shadow_semantic["overlapCount"]
                .as_u64()
                .expect("shadow overlap count should be numeric");
            let baseline_sources = shadow_semantic["baselineTopSources"]
                .as_array()
                .expect("baseline top sources should be present");
            assert!(
                baseline_sources.is_empty() || overlap_count >= 1,
                "shadow semantic probe should overlap baseline candidates when baseline exists: {}",
                shadow_semantic
            );
            let shadow_sources = shadow_semantic["shadowTopSources"]
                .as_array()
                .expect("shadow top sources should be present");
            assert!(
                !shadow_sources.is_empty(),
                "shadow top sources should not be empty: {}",
                shadow_semantic
            );
        } else {
            assert!(
                shadow_semantic["reason"].as_str().is_some(),
                "non-ok shadow semantic status should include reason: {}",
                shadow_semantic
            );
        }
    }

    #[tokio::test]
    async fn execute_recall_policy_explain_marks_shadow_semantic_unavailable_without_query_vector()
    {
        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            insert_memory_with_embedding(
                &conn,
                "daemon ownership lock protects recovery startup paths",
                "memory::daemon-lock",
                &[1.0, 0.0, 0.0, 0.0, 0.0],
            );
        }

        let explain = execute_recall_policy_explain_inner(
            &state,
            "daemon ownership lock",
            220,
            4,
            "codex",
            &solo_ctx(),
            None,
            6,
            None,
        )
        .await
        .expect("policy explain should succeed");

        let shadow_semantic = &explain["explain"]["shadowSemantic"];
        assert_eq!(
            explain["policy"]["semanticRoute"]["mode"].as_str(),
            Some("baseline")
        );
        assert_eq!(shadow_semantic["enabled"].as_bool(), Some(true));
        assert_eq!(shadow_semantic["status"].as_str(), Some("unavailable"));
        assert_eq!(
            shadow_semantic["reason"].as_str(),
            Some("query_embedding_unavailable")
        );
    }

    #[test]
    fn shadow_semantic_explain_uses_provided_baseline_override() {
        let conn = test_conn();
        let query_vector = [0.9_f32, 0.1_f32, 0.0_f32];
        let baseline = ShadowSemanticBaseline {
            candidate_count: 3,
            ranked_sources: vec![
                "memory::lock-heartbeat".to_string(),
                "memory::token-budget".to_string(),
                "decision::daemon-policy".to_string(),
            ],
        };

        let explain = build_shadow_semantic_explain(
            &conn,
            Some(&query_vector),
            "daemon ownership lock",
            &solo_ctx(),
            None,
            2,
            Some(&baseline),
        );
        assert_eq!(explain["status"].as_str(), Some("unavailable"));
        assert_eq!(explain["reason"].as_str(), Some("no_shadow_candidates"));
        assert_eq!(explain["baselineCandidateCount"].as_u64(), Some(3));
        assert_eq!(
            explain["baselineTopSources"]
                .as_array()
                .map(|items| items.len()),
            Some(2)
        );
        assert_eq!(
            explain["baselineTopSources"][0].as_str(),
            Some("memory::lock-heartbeat")
        );
        assert_eq!(
            explain["baselineTopSources"][1].as_str(),
            Some("memory::token-budget")
        );
    }

    #[test]
    fn sqlite_vec_shadow_knn_returns_ranked_sources_on_registered_connections() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("cortex-shadow-knn-{unique}.db"));
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");

        let conn = crate::db::open(&db_path).expect("db open should register sqlite-vec");
        crate::db::configure(&conn).expect("db configure should succeed");
        crate::db::initialize_schema(&conn).expect("schema init should succeed");
        crate::db::run_pending_migrations(&conn);
        insert_memory_with_embedding(
            &conn,
            "daemon ownership lock lease heartbeat",
            "memory::lock-heartbeat",
            &[1.0, 0.0, 0.0, 0.0, 0.0],
        );
        insert_memory_with_embedding(
            &conn,
            "token budgeting and ranking factors",
            "memory::token-budget",
            &[0.1, 0.9, 0.0, 0.0, 0.0],
        );

        let query_vector = [0.98, 0.02, 0.0, 0.0, 0.0];
        let rows = collect_shadow_semantic_rows(&conn, &solo_ctx(), None, query_vector.len());
        assert!(
            rows.len() >= 2,
            "shadow row collection should include inserted vectors"
        );
        assert!(
            rows.iter()
                .all(|row| row.vector.len() == query_vector.len()),
            "shadow rows should keep expected vector dimensionality"
        );
        let ranked_sources = run_sqlite_vec_shadow_knn_sources(&conn, &query_vector, &rows, 2)
            .expect("shadow knn should succeed");
        assert!(
            !ranked_sources.is_empty(),
            "shadow knn should return ranked sources"
        );
        assert_eq!(
            ranked_sources[0], "memory::lock-heartbeat",
            "nearest vector should rank first"
        );

        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);
    }

    #[test]
    fn test_budget_recall_skips_associative_expansion_for_tight_budgets() {
        let mut conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::daemon-lock', 'note', 'active', 0.9, 0.92, datetime('now'), datetime('now'))",
            params!["daemon ownership lock lease protects startup arbitration and stale pid recovery"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, trust_score, created_at, updated_at)
             VALUES (?1, 'memory::recovery-playbook', 'note', 'active', 0.25, 0.25, datetime('now'), datetime('now'))",
            params!["snapshot pruning and wal checkpoint cadence for cold-start recovery"],
        )
        .unwrap();
        for _ in 0..6 {
            crate::co_occurrence::record(
                &conn,
                &[
                    "memory::daemon-lock".to_string(),
                    "memory::recovery-playbook".to_string(),
                ],
            )
            .unwrap();
        }

        let results = run_budget_recall(
            &mut conn,
            "daemon ownership lock",
            180,
            8,
            &solo_ctx(),
            None,
        )
        .expect("budget recall should succeed");

        assert!(
            results.iter().all(|item| item.method != "associative"),
            "tight token budgets should skip associative expansion"
        );
    }

    #[test]
    fn budget_rank_char_cap_remains_rank_monotonic_and_query_adaptive() {
        let top = budget_rank_char_cap(200, 0, "daemon ownership lock policy");
        let second = budget_rank_char_cap(200, 1, "daemon ownership lock policy");
        let third = budget_rank_char_cap(200, 2, "daemon ownership lock policy");
        assert!(top > second && second > third);

        let exact = budget_rank_char_cap(200, 0, "auth.rs");
        let natural = budget_rank_char_cap(
            200,
            0,
            "How does Cortex preserve session truth after a daemon restart and reconnect?",
        );
        assert!(
            exact > natural,
            "exact identifier-like queries should receive more excerpt budget than broad natural queries"
        );
    }

    #[test]
    fn semantic_budget_max_items_adjusts_by_query_shape() {
        assert_eq!(
            semantic_budget_max_items(180, "auth.rs", 10),
            3,
            "tight exact query should prefer fewer, denser items"
        );
        assert_eq!(
            semantic_budget_max_items(
                180,
                "How does Cortex preserve session truth after a daemon restart and reconnect?",
                10,
            ),
            5,
            "tight broad query should keep one extra item for coverage"
        );
    }

    #[test]
    fn should_early_stop_budget_selection_requires_coverage_and_pressure() {
        let query_terms = HashSet::from(["daemon".to_string(), "heartbeat".to_string()]);
        let covered_all = HashSet::from(["daemon".to_string(), "heartbeat".to_string()]);
        let covered_partial = HashSet::from(["daemon".to_string()]);

        assert!(
            should_early_stop_budget_selection(300, 252, 2, &query_terms, &covered_all),
            "high budget pressure + full coverage should stop tail expansion"
        );
        assert!(
            !should_early_stop_budget_selection(300, 200, 2, &query_terms, &covered_all),
            "insufficient budget pressure should keep scanning candidates"
        );
        assert!(
            !should_early_stop_budget_selection(300, 252, 1, &query_terms, &covered_all),
            "need at least two selected results before early stop"
        );
        assert!(
            !should_early_stop_budget_selection(300, 252, 2, &query_terms, &covered_partial),
            "missing query-term coverage should keep searching"
        );
    }

    #[test]
    fn enforce_budget_token_invariant_trims_tail_when_spent_exceeds_budget() {
        let results = vec![
            RecallItem {
                source: "memory::a".to_string(),
                relevance: 0.9,
                excerpt: "daemon lock ownership and startup arbitration details".to_string(),
                method: "keyword".to_string(),
                tokens: Some(170),
                entropy: None,
                family_members: Vec::new(),
                collapsed_sources: Vec::new(),
                collapsed_source_scores: Vec::new(),
            },
            RecallItem {
                source: "memory::b".to_string(),
                relevance: 0.84,
                excerpt: "plugin heartbeat ownership reconciliation and reconnect sequence"
                    .to_string(),
                method: "semantic".to_string(),
                tokens: Some(160),
                entropy: None,
                family_members: Vec::new(),
                collapsed_sources: Vec::new(),
                collapsed_source_scores: Vec::new(),
            },
        ];

        let adjusted =
            enforce_budget_token_invariant(results, 300, "daemon ownership lock heartbeat");
        let usage = compute_recall_budget_usage(&adjusted, 300);
        assert!(
            !usage.over_budget,
            "invariant pass should guarantee spent <= budget"
        );
        assert!(
            usage.spent <= 300,
            "expected usage to fit the budget, got {}",
            usage.spent
        );
        assert!(
            !adjusted.is_empty(),
            "invariant pass should keep at least one high-rank result when possible"
        );
    }

    #[test]
    fn apply_semantic_budget_compacts_same_family_candidates_for_tight_budgets() {
        let results = apply_semantic_budget(
            vec![
                RecallItem {
                    source: "crystal::1::daemon lifecycle".to_string(),
                    relevance: 0.92,
                    excerpt:
                        "Daemon recovery policy covers lease renewal and safe restart behavior."
                            .to_string(),
                    method: "crystal".to_string(),
                    tokens: None,
                    entropy: None,
                    family_members: vec!["memory::family-child".to_string()],
                    collapsed_sources: vec!["memory::family-child".to_string()],
                    collapsed_source_scores: vec![("memory::family-child".to_string(), 0.88)],
                },
                RecallItem {
                    source: "memory::family-child".to_string(),
                    relevance: 0.89,
                    excerpt: "Child detail about plugin reconnect heartbeat.".to_string(),
                    method: "associative".to_string(),
                    tokens: None,
                    entropy: None,
                    family_members: Vec::new(),
                    collapsed_sources: Vec::new(),
                    collapsed_source_scores: Vec::new(),
                },
                RecallItem {
                    source: "memory::other-family".to_string(),
                    relevance: 0.83,
                    excerpt: "Unrelated recovery guardrail detail.".to_string(),
                    method: "keyword".to_string(),
                    tokens: None,
                    entropy: None,
                    family_members: Vec::new(),
                    collapsed_sources: Vec::new(),
                    collapsed_source_scores: Vec::new(),
                },
            ],
            180,
            "daemon recovery policy",
        );

        assert!(
            results
                .iter()
                .any(|item| item.source == "crystal::1::daemon lifecycle"),
            "tight budget should keep one family representative"
        );
        assert!(
            results
                .iter()
                .any(|item| item.source == "memory::other-family"),
            "tight budget should still keep unrelated high-signal context"
        );
        assert!(
            results
                .iter()
                .all(|item| item.source != "memory::family-child"),
            "tight budget should not spend a second slot on the same crystal family"
        );
    }

    #[test]
    fn recall_policy_mode_parser_accepts_supported_values() {
        assert_eq!(
            parse_recall_policy_mode(Some("fast")).expect("fast should parse"),
            Some(RecallPolicyMode::Fast)
        );
        assert_eq!(
            parse_recall_policy_mode(Some("balanced")).expect("balanced should parse"),
            Some(RecallPolicyMode::Balanced)
        );
        assert_eq!(
            parse_recall_policy_mode(Some("deep")).expect("deep should parse"),
            Some(RecallPolicyMode::Deep)
        );
        assert_eq!(
            parse_recall_policy_mode(Some("headlines")).expect("headlines should parse"),
            Some(RecallPolicyMode::Headlines)
        );
        assert!(parse_recall_policy_mode(Some("unknown-mode")).is_err());
    }

    #[test]
    fn resolve_recall_budget_k_uses_policy_defaults_when_budget_missing() {
        let (budget, k, mode) = resolve_recall_budget_k(Some(RecallPolicyMode::Fast), None, None);
        assert_eq!(mode, RecallPolicyMode::Fast);
        assert_eq!(
            budget,
            recall_default_budget_for_mode(RecallPolicyMode::Fast)
        );
        assert_eq!(k, recall_default_k_for_mode(RecallPolicyMode::Fast));

        let (budget, k, mode) = resolve_recall_budget_k(None, Some(640), None);
        assert_eq!(mode, RecallPolicyMode::Deep);
        assert_eq!(budget, 640);
        assert_eq!(k, recall_default_k_for_mode(RecallPolicyMode::Deep));
    }

    #[test]
    fn maybe_apply_adaptive_default_budget_reduces_short_exact_queries() {
        let (resolved_budget, resolved_k, _mode) = resolve_recall_budget_k(None, None, None);
        assert_eq!(resolved_budget, DEFAULT_RECALL_BUDGET_BALANCED);
        let adaptive =
            maybe_apply_adaptive_default_budget("auth.rs", None, None, resolved_budget, resolved_k);
        assert!(
            adaptive < resolved_budget,
            "short exact default queries should use less than balanced default budget"
        );
        assert!(
            adaptive >= 140,
            "adaptive budget should preserve baseline floor for recall quality"
        );
    }

    #[test]
    fn maybe_apply_adaptive_default_budget_preserves_long_natural_query_headroom() {
        let (resolved_budget, resolved_k, _mode) = resolve_recall_budget_k(None, None, Some(12));
        let adaptive = maybe_apply_adaptive_default_budget(
            "How does Cortex preserve session truth after a daemon restart and reconnect when plugin heartbeat ownership drifts?",
            None,
            None,
            resolved_budget,
            resolved_k,
        );
        assert!(
            adaptive >= 280,
            "broader natural queries should retain substantial budget headroom"
        );
        assert!(adaptive <= resolved_budget);
    }

    #[test]
    fn maybe_apply_adaptive_default_budget_does_not_override_explicit_settings() {
        let explicit_budget =
            maybe_apply_adaptive_default_budget("auth.rs", None, Some(512), 512, 6);
        assert_eq!(explicit_budget, 512);

        let explicit_mode_budget = maybe_apply_adaptive_default_budget(
            "auth.rs",
            Some(RecallPolicyMode::Deep),
            None,
            DEFAULT_RECALL_BUDGET_DEEP,
            recall_default_k_for_mode(RecallPolicyMode::Deep),
        );
        assert_eq!(explicit_mode_budget, DEFAULT_RECALL_BUDGET_DEEP);
    }

    #[tokio::test]
    async fn execute_unified_recall_fail_closes_when_latency_budget_is_zero() {
        let _env_lock = crate::test_env::lock_async().await;
        let _latency_budget =
            crate::test_env::ScopedEnvVar::set("CORTEX_RECALL_FAST_MAX_LATENCY_MS", "0");

        let state = shared_test_state();
        {
            let conn = state.db.lock().await;
            insert_memory_with_embedding(
                &conn,
                "daemon ownership lock heartbeat policy",
                "memory::lock-policy",
                &[1.0, 0.0, 0.0, 0.0, 0.0],
            );
        }

        let payload = execute_unified_recall(
            &state,
            "daemon ownership lock",
            180,
            8,
            "codex",
            &solo_ctx(),
            None,
        )
        .await
        .expect("recall should succeed");

        assert_eq!(payload["policyMode"].as_str(), Some("fast"));
        assert_eq!(payload["failClosed"]["triggered"].as_bool(), Some(true));
        assert_eq!(
            payload["semanticRoute"]["reason"].as_str(),
            Some("latency_budget_fail_closed")
        );
        assert_eq!(payload["latencyBudgetMs"].as_u64(), Some(0));
    }

    // ── query cache tests ──────────────────────────────────────────

    #[test]
    fn test_jaccard_similarity_identical() {
        let score = jaccard_similarity("rust error handling", "rust error handling");
        assert!((score - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_disjoint() {
        let score = jaccard_similarity("apple orange", "banana grape");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_jaccard_similarity_partial() {
        // "rust error" vs "rust warning" -- 1 shared ("rust"), 3 total -> 1/3
        let score = jaccard_similarity("rust error", "rust warning");
        assert!((score - 1.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_jaccard_similarity_above_threshold() {
        // "recall pipeline rrf fusion" vs "recall rrf pipeline" -- 3 shared, 4 total -> 0.75 >= 0.6
        let score = jaccard_similarity("recall pipeline rrf fusion", "recall rrf pipeline");
        assert!(score >= 0.6, "expected >= 0.6, got {score}");
    }
}
