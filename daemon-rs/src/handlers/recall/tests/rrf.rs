// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;
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

}
