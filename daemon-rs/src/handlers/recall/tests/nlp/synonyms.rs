// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::super::support::*;
    use super::super::super::*;
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
}
