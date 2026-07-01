// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::super::support::*;
    use super::super::super::*;
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
}
