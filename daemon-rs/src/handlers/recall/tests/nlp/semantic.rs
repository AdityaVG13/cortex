// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::super::support::*;
    use super::super::super::*;
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
}
}
