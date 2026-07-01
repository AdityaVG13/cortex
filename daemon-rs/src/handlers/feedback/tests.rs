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
        let boosts = compute_boosts(&conn, &sources, None);
        assert!(boosts["memory::a"] > 0.0);
        assert!(boosts["memory::b"] < 0.0);
        assert!(!boosts.contains_key("memory::c"), "No feedback = no entry");
    }

    #[test]
    fn test_compute_boosts_prefers_similar_query_embeddings() {
        let conn = setup_test_db();
        let source = "memory::ranked";
        let similar = vec![1.0_f32, 0.0, 0.0];
        let dissimilar = vec![0.0_f32, 1.0, 0.0];
        let similar_blob = embeddings::vector_to_blob(&similar);
        let dissimilar_blob = embeddings::vector_to_blob(&dissimilar);

        conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, signal, agent) \
             VALUES ('similar', ?1, ?2, 'memory', 0.1, 'test')",
            params![similar_blob, source],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, query_embedding, result_source, result_type, signal, agent) \
             VALUES ('dissimilar', ?1, ?2, 'memory', 0.1, 'test')",
            params![dissimilar_blob, source],
        )
        .unwrap();

        let scoped = compute_boosts(&conn, &[source.to_string()], Some(&similar));
        let baseline = compute_boosts(&conn, &[source.to_string()], None);

        assert!(
            scoped[source] < baseline[source],
            "dissimilar feedback should be down-weighted for current query"
        );
        assert!(
            scoped[source] > 0.0,
            "similar feedback should still contribute positive boost"
        );
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

    #[test]
    fn test_record_agent_feedback_from_value_rejects_invalid_outcome() {
        let conn = setup_test_db();
        let payload = json!({
            "agent": "codex",
            "taskClass": "recall",
            "outcome": "unknown"
        });
        let err = record_agent_feedback_from_value(&conn, 0, &payload, "mcp").unwrap_err();
        assert!(err.contains("invalid outcome"));
    }

    #[test]
    fn test_record_agent_feedback_from_value_persists_entry() {
        let conn = setup_test_db();
        let payload = json!({
            "agent": "codex",
            "taskClass": "recall",
            "outcome": "success",
            "qualityScore": 0.9,
            "latencyMs": 120,
            "tokensUsed": 280,
            "memorySources": ["decision::42"]
        });
        let result = record_agent_feedback_from_value(&conn, 0, &payload, "mcp").unwrap();
        assert_eq!(result["stored"].as_bool(), Some(true));
        assert_eq!(result["agent"].as_str(), Some("codex"));

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM agent_feedback WHERE owner_id = 0 AND agent = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_build_agent_feedback_stats_payload_aggregates_by_owner() {
        let conn = setup_test_db();
        record_agent_feedback_from_value(
            &conn,
            0,
            &json!({
                "agent": "codex",
                "taskClass": "recall",
                "outcome": "success",
                "qualityScore": 0.8,
                "memorySources": ["decision::1"]
            }),
            "mcp",
        )
        .unwrap();
        record_agent_feedback_from_value(
            &conn,
            9,
            &json!({
                "agent": "claude",
                "taskClass": "store",
                "outcome": "failure",
                "qualityScore": 0.2
            }),
            "mcp",
        )
        .unwrap();

        let stats = build_agent_feedback_stats_payload(&conn, 0, 30, 100, None, None).unwrap();
        assert_eq!(stats["sampled"].as_i64(), Some(1));
        assert_eq!(stats["outcomes"]["success"].as_i64(), Some(1));
        assert_eq!(stats["outcomes"]["failure"].as_i64(), Some(0));
        assert_eq!(
            stats["byAgent"][0]["name"].as_str(),
            Some("codex"),
            "owner-scoped stats should exclude other owners"
        );
    }

    #[test]
    fn test_recommend_recall_k_increases_depth_for_struggling_task_class() {
        let conn = setup_test_db();
        for _ in 0..10 {
            record_agent_feedback_from_value(
                &conn,
                0,
                &json!({
                    "agent": "codex",
                    "taskClass": "debug",
                    "outcome": "failure",
                    "qualityScore": 0.25
                }),
                "mcp",
            )
            .unwrap();
        }
        let policy = recommend_recall_k(&conn, 0, "codex", Some("debug"), 10)
            .unwrap()
            .expect("policy expected");
        assert_eq!(policy["recommendedK"].as_u64(), Some(14));
        assert_eq!(policy["reason"].as_str(), Some("raise_depth_for_recovery"));
    }

    #[test]
    fn test_recommend_recall_k_reduces_depth_for_stable_high_quality_runs() {
        let conn = setup_test_db();
        for _ in 0..12 {
            record_agent_feedback_from_value(
                &conn,
                0,
                &json!({
                    "agent": "codex",
                    "taskClass": "refactor",
                    "outcome": "success",
                    "qualityScore": 0.95
                }),
                "mcp",
            )
            .unwrap();
        }
        let policy = recommend_recall_k(&conn, 0, "codex", Some("refactor"), 12)
            .unwrap()
            .expect("policy expected");
        assert_eq!(policy["recommendedK"].as_u64(), Some(10));
        assert_eq!(
            policy["reason"].as_str(),
            Some("reduce_depth_for_efficiency")
        );
    }
}
