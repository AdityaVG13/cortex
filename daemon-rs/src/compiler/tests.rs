// SPDX-License-Identifier: MIT

use super::*;
    use super::*;

    fn repeated_text(label: &str, repeats: usize) -> String {
        let mut text = format!("## {label}\n");
        for idx in 0..repeats {
            text.push_str(&format!("{label} detail {idx}. "));
        }
        text
    }

    fn admitted_tokens(admitted: &[Value], name: &str, key: &str) -> usize {
        admitted
            .iter()
            .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
            .and_then(|item| item.get(key))
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(0)
    }

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 5, 12, 0, 0)
            .single()
            .expect("valid fixed time")
    }

    fn candidate(
        source_id: i64,
        retention_class: &str,
        updated_at: &str,
        retrievals: i64,
        last_accessed: Option<&str>,
        relevance: f64,
    ) -> RankedCandidate {
        RankedCandidate {
            source_kind: "memory",
            source_id,
            retention_class: retention_class.to_string(),
            title: format!("candidate-{source_id}"),
            body: format!("candidate body {source_id}"),
            updated_at: Some(updated_at.to_string()),
            created_at: Some(updated_at.to_string()),
            last_accessed: last_accessed.map(str::to_string),
            retrievals,
            relevance,
            components: empty_rank_components(),
        }
    }

    #[test]
    fn ranking_component_scores_cover_class_recency_relevance_and_activity() {
        let now = fixed_now();

        assert_eq!(retention_class_score("durable"), 1.0);
        assert_eq!(retention_class_score("operational"), 0.8);
        assert_eq!(retention_class_score("audit"), 0.4);
        assert_eq!(retention_class_score("ephemeral"), 0.2);
        assert!(recency_score(Some("2026-05-05T11:30:00Z"), now) > 0.9);
        assert!(recency_score(Some("2025-01-01T00:00:00Z"), now) < 0.1);
        assert!(
            activity_score(10, Some("2026-05-05T11:30:00Z"), now) > activity_score(0, None, now)
        );

        let ranked = rank_components_for(
            &candidate(
                1,
                "operational",
                "2026-05-05T11:30:00Z",
                6,
                Some("2026-05-05T11:45:00Z"),
                1.4,
            ),
            now,
        );
        assert_eq!(ranked.relevance_score, 1.0);
        assert!(ranked.total_score > 0.75);
    }

    #[test]
    fn active_operational_context_ranks_above_stale_durable_context() {
        let now = fixed_now();
        let stale_durable = candidate(1, "durable", "2025-01-01T00:00:00Z", 0, None, 0.85);
        let active_operational = candidate(
            2,
            "operational",
            "2026-05-05T11:50:00Z",
            9,
            Some("2026-05-05T11:55:00Z"),
            0.70,
        );

        let ranked = rank_candidates(vec![stale_durable, active_operational], 2, now);

        assert_eq!(ranked[0].source_id, 2);
        assert!(ranked[0].components.total_score > ranked[1].components.total_score);
    }

    #[test]
    fn ranked_context_items_emit_boot_audit_components() {
        let now = fixed_now();
        let ranked = rank_candidates(
            vec![candidate(
                7,
                "operational",
                "2026-05-05T11:50:00Z",
                4,
                Some("2026-05-05T11:55:00Z"),
                0.90,
            )],
            1,
            now,
        );
        let item = ContextItem::from_ranked_candidate(ranked[0].clone());
        let packed = pack_context_items(&[item], 200, SourceTokenBounds::new(20, 200));
        let capsule = &packed.admitted[0];

        assert_eq!(capsule["sourceKind"], "memory");
        assert_eq!(capsule["sourceId"], 7);
        assert_eq!(capsule["retentionClass"], "operational");
        assert!(capsule["rankComponents"]["class"].as_f64().unwrap() > 0.0);
        assert!(capsule["rankComponents"]["total"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn flat_score_fallback_matches_legacy_greedy_packing() {
        let items = vec![
            ContextItem::new("alpha", repeated_text("alpha", 40), 0.5),
            ContextItem::new("beta", repeated_text("beta", 40), 0.5),
            ContextItem::new("gamma", repeated_text("gamma", 40), 0.5),
        ];

        let legacy = pack_context_items_greedy(&items, 120);
        let packed = pack_context_items(&items, 120, SourceTokenBounds::new(20, 200));

        assert_eq!(packed.assembled_parts, legacy.assembled_parts);
        assert_eq!(packed.admitted, legacy.admitted);
        assert_eq!(packed.rejected, legacy.rejected);
    }

    #[test]
    fn forced_legacy_mode_uses_greedy_packer_for_variance_fixture() {
        let items = vec![
            ContextItem::new("high", repeated_text("high", 90), 0.95),
            ContextItem::new("low", repeated_text("low", 10), 0.20),
            ContextItem::new("medium", repeated_text("medium", 40), 0.60),
        ];

        let legacy = pack_context_items_greedy(&items, 120);
        let packed = pack_context_items_with_mode(
            &items,
            120,
            SourceTokenBounds::new(24, 120),
            BootPackingMode::LegacyGreedy,
        );

        assert_eq!(packed.assembled_parts, legacy.assembled_parts);
        assert_eq!(packed.admitted, legacy.admitted);
        assert_eq!(packed.rejected, legacy.rejected);
    }

    #[test]
    fn score_adaptive_packing_gives_high_score_sources_more_tokens() {
        let items = vec![
            ContextItem::new("high", repeated_text("high", 140), 0.95),
            ContextItem::new("low", repeated_text("low", 140), 0.15),
        ];

        let packed = pack_context_items(&items, 140, SourceTokenBounds::new(24, 120));
        let high_allocated = admitted_tokens(&packed.admitted, "high", "allocatedTokens");
        let low_allocated = admitted_tokens(&packed.admitted, "low", "allocatedTokens");
        let high_tokens = admitted_tokens(&packed.admitted, "high", "tokens");
        let low_tokens = admitted_tokens(&packed.admitted, "low", "tokens");

        assert!(
            high_allocated > low_allocated,
            "expected high-score allocation > low-score allocation: {packed:?}",
            packed = packed.admitted
        );
        assert!(
            high_tokens > low_tokens,
            "expected high-score source to receive more emitted tokens: {packed:?}",
            packed = packed.admitted
        );
    }

    #[test]
    fn source_token_bounds_keeps_max_at_or_above_min() {
        assert_eq!(
            SourceTokenBounds::new(80, 40),
            SourceTokenBounds { min: 80, max: 80 }
        );
    }

    #[test]
    fn score_adaptive_allocation_respects_budget_below_floor() {
        let items = vec![ContextItem::new("tiny", repeated_text("tiny", 80), 1.0)];
        let allocations = score_adaptive_allocations(&items, 10, SourceTokenBounds::new(40, 120));

        assert_eq!(allocations, vec![10]);
    }

