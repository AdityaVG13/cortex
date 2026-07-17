// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_health_{name}_{unique}"))
    }

    #[test]
    fn public_health_payload_redacts_private_runtime_paths() {
        let mut payload = json!({
            "runtime": {
                "version": "0.6.0",
                "mode": "team",
                "port": 7437,
                "db_path": "C:/Users/example/.cortex/cortex.db",
                "token_path": "C:/Users/example/.cortex/cortex.token",
                "pid_path": "C:/Users/example/.cortex/cortex.pid",
                "ipc_endpoint": r"\\.\pipe\cortex-daemon-7437",
                "ipc_kind": "named-pipe",
                "executable": "C:/Users/example/cortex.exe",
                "owner": "control-center"
            },
            "stats": {
                "home": "C:/Users/example/.cortex",
                "memories": 3
            }
        });

        redact_private_runtime_details(&mut payload);

        let runtime = payload["runtime"].as_object().unwrap();
        assert_eq!(runtime["version"], "0.6.0");
        assert_eq!(runtime["mode"], "team");
        assert_eq!(runtime["port"], 7437);
        for key in [
            "db_path",
            "token_path",
            "pid_path",
            "ipc_endpoint",
            "ipc_kind",
            "executable",
            "owner",
        ] {
            assert!(
                !runtime.contains_key(key),
                "public health payload should redact {key}"
            );
        }
        assert!(!payload["stats"].as_object().unwrap().contains_key("home"));
        assert_eq!(payload["stats"]["memories"], 3);
    }

    #[test]
    fn private_runtime_details_require_cortex_header_and_loopback_peer() {
        let mut headers = HeaderMap::new();
        assert!(!include_private_runtime_details(&headers));

        headers.insert("x-cortex-request", HeaderValue::from_static("true"));
        assert!(include_private_runtime_details(&headers));

        headers.insert(
            crate::handlers::CORTEX_PEER_IP_HEADER,
            HeaderValue::from_static("203.0.113.9"),
        );
        assert!(!include_private_runtime_details(&headers));
    }

    #[test]
    fn collect_storage_metrics_reports_storage_backup_count_and_log_bytes() {
        let home_dir = temp_test_dir("storage_metrics");
        let backup_dir = home_dir.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        fs::write(backup_dir.join("cortex-a.db"), b"1234").unwrap();
        fs::write(backup_dir.join("cortex-b.db"), b"56").unwrap();
        fs::write(backup_dir.join("ignore.txt"), b"zzz").unwrap();
        fs::write(home_dir.join("daemon.log"), b"abcd").unwrap();
        fs::write(home_dir.join("daemon.log.1"), b"ef").unwrap();

        let (storage_bytes, backup_count, log_bytes) = collect_storage_metrics(&home_dir);

        assert_eq!(backup_count, 2);
        assert_eq!(log_bytes, 6);
        assert_eq!(storage_bytes, 15);

        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn build_digest_excludes_benchmark_boot_savings_rows() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now'))",
            params![json!({
                "agent": "amb-cortex::run-a",
                "saved": 500,
                "served": 100,
                "baseline": 600
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now'))",
            params![json!({
                "agent": "codex",
                "saved": 50,
                "served": 20,
                "baseline": 70
            })
            .to_string()],
        )
        .unwrap();

        let digest = build_digest(&conn).expect("digest payload should build");
        assert_eq!(
            digest["tokenSavings"]["allTime"]["saved"].as_i64(),
            Some(50)
        );
        assert_eq!(
            digest["tokenSavings"]["allTime"]["served"].as_i64(),
            Some(20)
        );
        assert_eq!(digest["tokenSavings"]["allTime"]["boots"].as_i64(), Some(1));
        assert_eq!(digest["tokenSavings"]["today"]["saved"].as_i64(), Some(50));
    }

    #[test]
    fn collect_embedding_inventory_reports_model_mix_and_backlog() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('m-active-current', 'memory::current', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let memory_current_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('m-active-other', 'memory::other', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let memory_other_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('d-unknown-model', 'ctx::unknown', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let decision_unknown_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, score, merged_count, quality, created_at, updated_at) \
             VALUES ('d-missing-embedding', 'ctx::missing', 'tester', 'active', 1.0, 0, 70, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let decision_missing_id = conn.last_insert_rowid();

        let blob = crate::embeddings::vector_to_blob(&[0.2, 0.4, 0.6]);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'all-MiniLM-L6-v2')",
            params![memory_current_id, blob.clone()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('memory', ?1, ?2, 'other-model')",
            params![memory_other_id, blob.clone()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', ?1, ?2, NULL)",
            params![decision_unknown_id, blob],
        )
        .unwrap();

        let metrics = collect_embedding_inventory(&conn, "all-minilm-l6-v2");
        assert_eq!(metrics.active_model_embeddings, 1);
        assert_eq!(metrics.other_model_embeddings, 1);
        assert_eq!(metrics.unknown_model_embeddings, 1);
        assert_eq!(metrics.backlog_memories, 1);
        assert_eq!(metrics.backlog_decisions, 2);
        assert!(
            decision_missing_id > 0,
            "decision without embedding should contribute to backlog"
        );
    }

    #[test]
    fn cache_snapshot_if_fresh_enforces_ttl() {
        let now = Utc::now().timestamp();
        let fresh = HealthHeavyMetricsSnapshot {
            computed_at_unix_secs: now - 2,
            embedding_inventory: EmbeddingInventoryMetrics::default(),
            storage_bytes: 10,
            backup_count: 1,
            log_bytes: 2,
        };
        let stale = HealthHeavyMetricsSnapshot {
            computed_at_unix_secs: now - (HEALTH_HEAVY_CACHE_TTL_SECS + 5),
            embedding_inventory: EmbeddingInventoryMetrics::default(),
            storage_bytes: 10,
            backup_count: 1,
            log_bytes: 2,
        };

        assert!(cache_snapshot_if_fresh(Some(fresh), now).is_some());
        assert!(cache_snapshot_if_fresh(Some(stale), now).is_none());
        assert!(cache_snapshot_if_fresh(None, now).is_none());
    }

    #[test]
    fn savings_payload_cache_if_fresh_enforces_ttl() {
        let now = Utc::now().timestamp();
        let fresh = SavingsPayloadSnapshot {
            computed_at_unix_secs: now - 1,
            payload: json!({ "ok": true }),
        };
        let stale = SavingsPayloadSnapshot {
            computed_at_unix_secs: now - (SAVINGS_CACHE_TTL_SECS + 5),
            payload: json!({ "ok": false }),
        };

        assert!(savings_payload_cache_if_fresh(Some(fresh), now).is_some());
        assert!(savings_payload_cache_if_fresh(Some(stale), now).is_none());
        assert!(savings_payload_cache_if_fresh(None, now).is_none());
    }

    #[test]
    fn is_control_center_owner_is_case_insensitive() {
        assert!(is_control_center_owner(Some("control-center")));
        assert!(is_control_center_owner(Some("CoNtRoL-CeNtEr")));
        assert!(!is_control_center_owner(Some("plugin-codex")));
        assert!(!is_control_center_owner(None));
    }

    #[test]
    fn build_recall_stats_payload_summarizes_tiers_and_latency() {
        let rows = vec![
            (
                json!({
                    "mode": "balanced",
                    "budget": 200,
                    "spent": 60,
                    "saved": 140,
                    "hits": 2,
                    "cached": false,
                    "method_breakdown": { "keyword": 2 },
                    "tier": "keyword_only",
                    "latency_ms": 5,
                    "shadow_semantic": {
                        "status": "unavailable",
                        "reason": "query_embedding_unavailable"
                    }
                })
                .to_string(),
                "2026-04-14T10:00:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "full",
                    "budget": 300,
                    "spent": 220,
                    "saved": 80,
                    "hits": 3,
                    "cached": false,
                    "method_breakdown": { "hybrid": 2, "semantic": 1 },
                    "tier": "hybrid_fusion",
                    "latency_ms": 28,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.5,
                        "jaccard": 0.4,
                        "meanAbsRankDelta": 0.75,
                        "top1Match": true
                    }
                })
                .to_string(),
                "2026-04-14T10:01:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 180,
                    "spent": 0,
                    "saved": 180,
                    "hits": 1,
                    "cached": true,
                    "tier": "cache_hit",
                    "latency_ms": 1,
                    "shadow_semantic": {
                        "status": "skipped",
                        "reason": "cache_hit"
                    }
                })
                .to_string(),
                "2026-04-14T10:02:00Z".to_string(),
            ),
        ];

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["summary"]["totalRecalls"], 3);
        assert_eq!(payload["summary"]["totalBudget"], 680);
        assert_eq!(payload["summary"]["totalSpent"], 280);
        assert_eq!(payload["summary"]["totalSaved"], 400);
        assert_eq!(payload["tier_distribution"]["cache_hit"], 1);
        assert_eq!(payload["tier_distribution"]["keyword_only"], 1);
        assert_eq!(payload["tier_distribution"]["hybrid_fusion"], 1);
        assert_eq!(payload["avg_latency_ms"]["overall"], 11.3);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["unavailable"],
            1
        );
        assert_eq!(payload["shadow_semantic"]["status_counts"]["ok"], 1);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["skipped"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_rank_delta_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_samples"], 1);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_ratio_avg"], 0.5);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_avg"], 0.4);
        assert_eq!(
            payload["shadow_semantic"]["ok_mean_abs_rank_delta_avg"],
            0.75
        );
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_rate"], 1.0);
        assert_eq!(payload["shadowSemantic"]["statusCounts"]["ok"], 1);
        assert_eq!(payload["shadowSemantic"]["okOverlapRatioAvg"], 0.5);
        assert_eq!(payload["shadowSemantic"]["okMeanAbsRankDeltaAvg"], 0.75);
        assert_eq!(payload["shadowSemantic"]["okTop1MatchRate"], 1.0);
        assert_eq!(payload["shadowSemantic"]["okOverlapSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okJaccardSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okRankDeltaSamples"], 1);
        assert_eq!(payload["shadowSemantic"]["okTop1MatchSamples"], 1);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert!(payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be an array")
            .iter()
            .any(|value| value.as_str() == Some("insufficient_shadow_samples")));
        assert_eq!(payload["shadowSemanticGate"]["decision"], "hold");
        assert_eq!(
            payload["estimated_savings"]["vs_always_full_pipeline_pct"],
            58.8
        );
    }

    #[test]
    fn build_recall_stats_payload_reports_ready_shadow_semantic_gate() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.72,
                        "jaccard": 0.61,
                        "meanAbsRankDelta": 0.42,
                        "top1Match": true
                    }
                })
                .to_string(),
                format!("2026-04-14T10:{idx:02}:00Z"),
            ));
        }
        rows.push((
            json!({
                "mode": "balanced",
                "budget": 220,
                "spent": 110,
                "saved": 110,
                "hits": 2,
                "cached": false,
                "tier": "hybrid_fusion",
                "latency_ms": 9,
                "shadow_semantic": {
                    "status": "unavailable",
                    "reason": "query_embedding_unavailable"
                }
            })
            .to_string(),
            "2026-04-14T11:00:00Z".to_string(),
        ));
        rows.push((
            json!({
                "mode": "balanced",
                "budget": 220,
                "spent": 100,
                "saved": 120,
                "hits": 2,
                "cached": false,
                "tier": "hybrid_fusion",
                "latency_ms": 8,
                "shadow_semantic": {
                    "status": "error",
                    "reason": "transient_probe_failure"
                }
            })
            .to_string(),
            "2026-04-14T11:01:00Z".to_string(),
        ));

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["ok"], 30,
            "ok status count should include all successful probes"
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["decision"],
            "ready_for_vec0_trial"
        );
        assert_eq!(payload["shadow_semantic_gate"]["ready"], true);
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["probed_events"],
            32
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["unavailable_rate"],
            0.0313
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["error_rate"],
            0.0313
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_ratio_avg"],
            0.72
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_jaccard_avg"],
            0.61
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_mean_abs_rank_delta_avg"],
            0.42
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_rate"],
            1.0
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_jaccard_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_rank_delta_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_samples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["decision"],
            "ready_for_vec0_trial"
        );
        assert_eq!(payload["shadowSemanticGate"]["metrics"]["probedEvents"], 32);
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okMeanAbsRankDeltaAvg"],
            0.42
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okTop1MatchRate"],
            1.0
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okOverlapSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okJaccardSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okRankDeltaSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okTop1MatchSamples"],
            30
        );
    }

    #[test]
    fn build_recall_stats_payload_holds_shadow_gate_for_rank_drift() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": {
                        "status": "ok",
                        "overlapRatio": 0.78,
                        "jaccard": 0.68,
                        "meanAbsRankDelta": 2.2,
                        "top1Match": false
                    }
                })
                .to_string(),
                format!("2026-04-14T12:{idx:02}:00Z"),
            ));
        }

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        let blockers = payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be present");
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("mean_abs_rank_delta_above_gate")),
            "rank-delta blocker should be present"
        );
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("top1_match_rate_below_gate")),
            "top1-match blocker should be present"
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_mean_abs_rank_delta_avg"],
            2.2
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_top1_match_rate"],
            0.0
        );
    }

    #[test]
    fn build_recall_stats_payload_holds_shadow_gate_for_under_sampled_ok_metrics() {
        let mut rows: Vec<(String, String)> = Vec::new();
        for idx in 0..30 {
            let include_rank_signals = idx < 10;
            let mut shadow = json!({
                "status": "ok",
                "overlapRatio": 0.74,
                "jaccard": 0.62
            });
            if include_rank_signals {
                shadow["meanAbsRankDelta"] = json!(0.33);
                shadow["top1Match"] = json!(true);
            }
            rows.push((
                json!({
                    "mode": "balanced",
                    "budget": 220,
                    "spent": 120,
                    "saved": 100,
                    "hits": 3,
                    "cached": false,
                    "tier": "hybrid_fusion",
                    "latency_ms": 12,
                    "shadow_semantic": shadow
                })
                .to_string(),
                format!("2026-04-14T13:{idx:02}:00Z"),
            ));
        }

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic_gate"]["ready"], false);
        assert_eq!(payload["shadow_semantic_gate"]["decision"], "hold");
        let blockers = payload["shadow_semantic_gate"]["blockers"]
            .as_array()
            .expect("gate blockers should be present");
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("insufficient_rank_delta_samples")),
            "rank-delta sample blocker should be present"
        );
        assert!(
            blockers
                .iter()
                .any(|value| value.as_str() == Some("insufficient_top1_match_samples")),
            "top1-match sample blocker should be present"
        );
        assert_eq!(payload["shadow_semantic"]["ok_samples"], 10);
        assert_eq!(payload["shadow_semantic"]["ok_overlap_samples"], 30);
        assert_eq!(payload["shadow_semantic"]["ok_jaccard_samples"], 30);
        assert_eq!(payload["shadow_semantic"]["ok_rank_delta_samples"], 10);
        assert_eq!(payload["shadow_semantic"]["ok_top1_match_samples"], 10);
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_overlap_samples"],
            30
        );
        assert_eq!(
            payload["shadow_semantic_gate"]["metrics"]["ok_rank_delta_samples"],
            10
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okOverlapSamples"],
            30
        );
        assert_eq!(
            payload["shadowSemanticGate"]["metrics"]["okRankDeltaSamples"],
            10
        );
    }

    #[test]
    fn build_recall_stats_payload_normalizes_shadow_status_buckets() {
        let rows = vec![
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "OK", "overlapRatio": 0.7, "jaccard": 0.6, "meanAbsRankDelta": 0.5, "top1Match": true }
                })
                .to_string(),
                "2026-04-14T14:00:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": " UnAvailable " }
                })
                .to_string(),
                "2026-04-14T14:01:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "SKIPPED" }
                })
                .to_string(),
                "2026-04-14T14:02:00Z".to_string(),
            ),
            (
                json!({
                    "mode": "balanced",
                    "budget": 100,
                    "spent": 60,
                    "saved": 40,
                    "hits": 1,
                    "shadow_semantic": { "status": "mystery" }
                })
                .to_string(),
                "2026-04-14T14:03:00Z".to_string(),
            ),
        ];

        let payload = build_recall_stats_payload_from_rows(&rows);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["ok"], 1);
        assert_eq!(
            payload["shadow_semantic"]["status_counts"]["unavailable"],
            1
        );
        assert_eq!(payload["shadow_semantic"]["status_counts"]["skipped"], 1);
        assert_eq!(payload["shadow_semantic"]["status_counts"]["unknown"], 1);
    }

