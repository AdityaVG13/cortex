// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
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

}
