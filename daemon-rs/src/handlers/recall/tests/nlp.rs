// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
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
}
