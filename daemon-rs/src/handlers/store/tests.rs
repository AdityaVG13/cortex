// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn test_state() -> RuntimeState {
        let write_conn = test_conn();
        let read_conn = test_conn();
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
            db_path: PathBuf::from(":memory:"),
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

    fn unit_vector_for_similarity(similarity: f32) -> Vec<f32> {
        vec![similarity, (1.0 - similarity * similarity).sqrt()]
    }

    #[test]
    fn provenance_normalizes_client_model_and_depth() {
        let provenance = DecisionProvenance::from_fields(
            "Codex (GPT-5.4)",
            Some("claude-opus-4.1"),
            Some("multi_step"),
        );
        assert_eq!(provenance.source_client, "codex");
        assert_eq!(provenance.source_model.as_deref(), Some("claude-opus-4.1"));
        assert_eq!(provenance.reasoning_depth, "multi-step");
    }

    #[test]
    fn trust_score_prefers_stronger_models() {
        let weak = compute_trust_score(0.9, Some("qwen-30b"));
        let strong = compute_trust_score(0.9, Some("claude-opus-4.1"));
        assert!(strong > weak);
        assert_eq!(strong, 0.9);
    }

    fn insert_existing_decision(
        conn: &Connection,
        decision: &str,
        context: Option<&str>,
        vector: &[f32],
    ) -> i64 {
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES (?1, ?2, 'tester', 'active', 1.0, 0, 50, datetime('now'), datetime('now'))",
            params![decision, context],
        )
        .unwrap();
        let id = conn.last_insert_rowid();
        persist_decision_embedding(conn, id, vector, crate::embeddings::selected_model_key())
            .unwrap();
        id
    }

    fn insert_legacy_decision(
        conn: &Connection,
        decision: &str,
        source_agent: &str,
        trust_score: f64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, confidence, trust_score, status, quality, created_at, updated_at) \
             VALUES (?1, ?2, 'decision', ?3, ?4, ?4, 'active', 70, datetime('now'), datetime('now'))",
            params![decision, "seed", source_agent, trust_score],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn persist_decision_embedding_uses_explicit_model_key() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('model-tag check', 'ctx', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let id = conn.last_insert_rowid();

        persist_decision_embedding(&conn, id, &[0.3, 0.4, 0.5], "unit-test-model").unwrap();

        let stored_model: String = conn
            .query_row(
                "SELECT model FROM embeddings WHERE target_type = 'decision' AND target_id = ?1",
                params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_model, "unit-test-model");
    }

    #[test]
    fn store_with_provenance_persists_trust_fields() {
        let mut conn = test_conn();
        let provenance = DecisionProvenance::from_fields(
            "codex",
            Some("claude-opus-4.1"),
            Some("tool-assisted"),
        );

        let (_, new_id) = store_decision_with_input_embedding_and_provenance(
            &mut conn,
            "persist provenance for memory trust",
            Some("unit test".to_string()),
            Some("decision".to_string()),
            "codex".to_string(),
            provenance,
            Some(0.9),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let (source_client, source_model, reasoning_depth, trust_score): (
            String,
            Option<String>,
            String,
            f64,
        ) = conn
            .query_row(
                "SELECT source_client, source_model, reasoning_depth, trust_score FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(source_client, "codex");
        assert_eq!(source_model.as_deref(), Some("claude-opus-4.1"));
        assert_eq!(reasoning_depth, "tool-assisted");
        assert_eq!(trust_score, 0.9);
    }

    #[test]
    fn semantic_dedup_threshold_boundaries() {
        assert!(!should_merge_candidate(0.89, 0.95));
        assert!(!should_merge_candidate(0.90, 0.70));
        assert!(should_merge_candidate(0.90, 0.71));
        assert!(should_merge_candidate(0.91, 0.71));
        assert!(should_merge_candidate(0.92, 0.71));
        assert!(should_merge_candidate(0.93, 0.00));
    }

    #[test]
    fn benchmark_entries_bypass_semantic_merge() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "store benchmark messages without dedup collapsing",
            Some("seed"),
            &[1.0, 0.0],
        );

        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "store benchmark messages without dedup collapsing",
            Some("bench-doc".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.99)),
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn benchmark_entries_allow_vague_payloads() {
        let mut conn = test_conn();
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "?",
            Some("bench-doc".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
    }

    #[test]
    fn non_benchmark_decisions_are_length_capped() {
        let mut conn = test_conn();
        let long_text = "x".repeat(MAX_DECISION_CHARS + 1800);
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            &long_text,
            Some("long decision body".to_string()),
            Some("decision".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let id = new_id.expect("decision id");
        let stored_chars: i64 = conn
            .query_row(
                "SELECT LENGTH(decision) FROM decisions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_chars as usize, MAX_DECISION_CHARS);

        let truncation_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_truncated'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(truncation_events, 1);
    }

    #[test]
    fn benchmark_decisions_keep_full_length() {
        let mut conn = test_conn();
        let long_text = "x".repeat(MAX_DECISION_CHARS + 1800);
        let (_entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            &long_text,
            Some("benchmark payload".to_string()),
            Some("benchmark".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let id = new_id.expect("decision id");
        let stored_chars: i64 = conn
            .query_row(
                "SELECT LENGTH(decision) FROM decisions WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_chars as usize, long_text.len());
    }

    #[test]
    fn merge_behavior_increments_count_and_appends_context() {
        let mut conn = test_conn();
        insert_existing_decision(&conn, "use early returns in Go code", None, &[1.0, 0.0]);

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "always use early returns",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.93)),
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        assert_eq!(entry["action"], "merged");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (merged_count, score, context): (i64, f64, Option<String>) = conn
            .query_row(
                "SELECT merged_count, score, context FROM decisions LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(merged_count, 1);
        assert_eq!(score, 6.0);
        assert!(context.unwrap().contains("always use early returns"));

        let merge_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'merge'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(merge_events, 1);
    }

    #[test]
    fn jaccard_review_band_merges_when_tokens_match() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "use early returns in go code",
            Some("initial"),
            &[1.0, 0.0],
        );

        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "use early returns in go code today",
            Some("follow-up".to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.91)),
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        let merged_count: i64 = conn
            .query_row("SELECT merged_count FROM decisions LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(merged_count, 1);
    }

    #[test]
    fn jaccard_review_band_inserts_when_tokens_do_not_match() {
        let mut conn = test_conn();
        insert_existing_decision(
            &conn,
            "database migrations need backups",
            Some("initial"),
            &[1.0, 0.0],
        );

        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "always use early returns",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            Some(&unit_vector_for_similarity(0.91)),
            None,
        )
        .unwrap();

        assert!(new_id.is_some());
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn contradiction_policy_keeps_higher_trust_decision_active() {
        let mut conn = test_conn();
        let existing_id =
            insert_legacy_decision(&conn, "always run migrations before deploy", "claude", 0.95);

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "never run migrations before deploy",
            Some("contradiction".to_string()),
            None,
            "codex".to_string(),
            Some(0.6),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        assert_eq!(entry["classification"], "CONTRADICTS");
        assert_eq!(entry["status"], "disputed");
        assert_eq!(entry["conflict"]["status"], "auto_resolved");

        let existing_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(existing_status, "active");

        let inserted_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(inserted_status, "disputed");

        let conflict_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_conflicts WHERE source_decision_id = ?1 AND target_decision_id = ?2 AND classification = 'CONTRADICTS'",
                params![new_id, existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_rows, 1);
    }

    #[test]
    fn refinement_policy_supersedes_same_agent_decision() {
        let mut conn = test_conn();
        let existing_id = insert_legacy_decision(
            &conn,
            "use structured logging for daemon requests",
            "codex",
            0.6,
        );

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "use structured logging with request ids for daemon requests",
            Some("refinement".to_string()),
            None,
            "codex".to_string(),
            Some(0.7),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        assert_eq!(entry["classification"], "REFINES");
        assert_eq!(entry["status"], "superseded_old");
        assert_eq!(entry["conflict"]["status"], "auto_resolved");
        assert_eq!(entry["supersedes"], existing_id);

        let old_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![existing_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_status, "superseded");

        let new_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                params![new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_status, "active");
    }

    #[test]
    fn agreement_policy_merges_duplicate_without_inserting_new_decision() {
        let mut conn = test_conn();
        let target_id = insert_legacy_decision(
            &conn,
            "enable recall cache warming at startup",
            "claude",
            0.8,
        );

        let (entry, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "enable recall cache warming at startup",
            Some("same intent".to_string()),
            None,
            "codex".to_string(),
            Some(0.9),
            None,
            None,
            None,
        )
        .unwrap();

        assert!(new_id.is_none());
        assert_eq!(entry["action"], "merged");
        assert_eq!(entry["classification"], "AGREES");
        assert_eq!(entry["target_id"], target_id);
        assert_eq!(entry["conflict"]["status"], "auto_resolved");

        let decision_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(decision_count, 1);

        let conflict_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decision_conflicts WHERE source_decision_id IS NULL AND target_decision_id = ?1 AND classification = 'AGREES'",
                params![target_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_rows, 1);
    }

    #[test]
    fn quality_scoring_edge_cases() {
        let empty = assess_quality("");
        assert_eq!(empty.score, 0);

        let question = assess_quality("?");
        assert_eq!(question.score, 0);

        let long_specific = assess_quality(
            "Update daemon-rs/src/handlers/store.rs so handle_store() appends merge context and keeps the score bump when semantic dedup hits the review band threshold for near-duplicate decision text.",
        );
        assert_eq!(long_specific.score, 90);

        let code_snippet = assess_quality("fn handle_store() { return Ok(()); }");
        assert_eq!(code_snippet.score, 50);
    }

    #[test]
    fn detailed_store_persists_quality_score() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_input_embedding(
            &mut conn,
            "Always use rtk prefix for shell commands in Cortex repo",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let quality: i64 = conn
            .query_row(
                "SELECT quality FROM decisions WHERE id = ?1",
                [new_id.unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(quality >= 70);
    }

    #[test]
    fn rejection_at_quality_below_twenty() {
        let mut conn = test_conn();
        let err = store_decision_with_input_embedding(
            &mut conn,
            "?",
            None,
            None,
            "tester".to_string(),
            None,
            None,
            None,
            None,
        )
        .unwrap_err();

        match err {
            StoreError::Validation {
                message, quality, ..
            } => {
                assert_eq!(message, "Memory too vague");
                assert_eq!(quality, 0);
            }
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_store_returns_http_400_for_vague_input() {
        let state = test_state();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-token".parse().unwrap());
        headers.insert("x-cortex-request", "true".parse().unwrap());

        let response = handle_store(
            State(state),
            headers,
            Json(StoreRequest {
                decision: Some("?".to_string()),
                ..StoreRequest::default()
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn store_decision_with_ttl_sets_expires_at() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "temporary decision with enough detail to persist",
            Some("ttl-test".to_string()),
            None,
            "tester".to_string(),
            None,
            Some(3600),
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let expires_at: Option<String> = conn
            .query_row(
                "SELECT expires_at FROM decisions WHERE id = ?1",
                [new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(expires_at.is_some());

        let expires_in_future: i64 = conn
            .query_row(
                "SELECT expires_at > datetime('now') FROM decisions WHERE id = ?1",
                [new_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expires_in_future, 1);
    }

    #[test]
    fn store_decision_rejects_invalid_explicit_ttl() {
        for ttl_seconds in [0, -1, MAX_EXPLICIT_TTL_SECONDS + 1] {
            let mut conn = test_conn();
            let err = store_decision_with_ttl(
                &mut conn,
                "temporary decision with enough detail to persist",
                Some("ttl-test".to_string()),
                None,
                "tester".to_string(),
                None,
                Some(ttl_seconds),
                None,
            )
            .unwrap_err();

            if ttl_seconds <= 0 {
                assert_eq!(err, "ttl_seconds must be > 0");
            } else {
                assert_eq!(err, "ttl_seconds must be <= 31536000 (365 days)");
            }
        }
    }

    #[test]
    fn store_decision_explicit_retention_class_overrides_entry_type() {
        let mut conn = test_conn();
        let provenance = DecisionProvenance::from_fields("tester", None, None);
        let (entry, new_id) = store_decision_with_input_embedding_and_provenance_retention(
            &mut conn,
            "audit record with enough detail to persist",
            Some("permission audit".to_string()),
            Some("decision".to_string()),
            "tester".to_string(),
            provenance,
            None,
            None,
            Some(RetentionClass::Audit),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            entry.get("retention_class").and_then(Value::as_str),
            Some("audit")
        );
        let new_id = new_id.unwrap();
        let row: (String, Option<String>, i64) = conn
            .query_row(
                "SELECT retention_class, expires_at,
                        expires_at > datetime('now', '+364 days')
                        AND expires_at < datetime('now', '+366 days')
                 FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "audit");
        assert!(row.1.is_some());
        assert_eq!(row.2, 1);
    }

    #[test]
    fn store_decision_entry_type_sets_default_retention_ttl() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "observation with enough detail to persist",
            Some("ops note".to_string()),
            Some("observation".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let row: (String, Option<String>, i64) = conn
            .query_row(
                "SELECT retention_class, expires_at,
                        expires_at > datetime('now', '+89 days')
                        AND expires_at < datetime('now', '+91 days')
                 FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "operational");
        assert!(row.1.is_some());
        assert_eq!(row.2, 1);
    }

    #[test]
    fn store_decision_text_heuristic_marks_unknown_type_durable() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "Always preserve this architecture convention with enough detail",
            Some("api contract".to_string()),
            Some("misc".to_string()),
            "tester".to_string(),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT retention_class, expires_at FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "durable");
        assert!(row.1.is_none());
    }

    #[test]
    fn store_decision_without_ttl_leaves_expires_at_null() {
        let mut conn = test_conn();
        let (_, new_id) = store_decision_with_ttl(
            &mut conn,
            "persistent decision with enough detail to persist",
            Some("ttl-test".to_string()),
            None,
            "tester".to_string(),
            None,
            None,
            None,
        )
        .unwrap();

        let new_id = new_id.unwrap();
        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT retention_class, expires_at FROM decisions WHERE id = ?1",
                [new_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "durable");
        assert!(row.1.is_none());
    }

