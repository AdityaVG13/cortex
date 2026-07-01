// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
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

}
