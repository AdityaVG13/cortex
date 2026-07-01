// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);
        conn
    }

    #[test]
    fn test_prune_old_events() {
        let conn = setup();
        // Insert an old event
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('test', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('agent_boot', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', '{}', 'test', datetime('now', '-60 days'))",
            [],
        )
        .unwrap();
        // Insert a recent event
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('test', '{}', 'test')",
            [],
        )
        .unwrap();

        let pruned = prune_old_events(&conn);
        assert_eq!(
            pruned, 2,
            "Should prune old non-savings events, including stale agent_boot rows"
        );

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
    }

    #[test]
    fn test_rollup_old_boot_savings_compacts_history_and_keeps_recent_rows() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'test', datetime('now', '-60 days'))",
            params![r#"{"saved":100,"served":50,"baseline":150}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'test', datetime('now'))",
            params![r#"{"saved":20,"served":10,"baseline":30}"#],
        )
        .unwrap();

        let pruned = rollup_old_boot_savings_with_retention(&conn, 30);
        assert_eq!(pruned, 1);

        let raw_boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            raw_boot_rows, 1,
            "recent raw boot_savings row should remain"
        );

        let rollup: (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT \
                    COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.served') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.baseline') AS INTEGER), 0), \
                    COALESCE(CAST(json_extract(data, '$.boots') AS INTEGER), 0) \
                 FROM events WHERE type = 'boot_savings_rollup' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(rollup, (100, 50, 150, 1));
    }

    #[test]
    fn test_rollup_old_boot_savings_excludes_benchmark_agent_payloads() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-60 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 999,
                "served": 1,
                "baseline": 1000
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-60 days'))",
            params![serde_json::json!({
                "agent": "codex",
                "saved": 50,
                "served": 10,
                "baseline": 60
            })
            .to_string()],
        )
        .unwrap();

        let pruned = rollup_old_boot_savings_with_retention(&conn, 30);
        assert_eq!(
            pruned, 1,
            "only non-benchmark boot_savings rows should roll up"
        );

        let rollup_saved: i64 = conn
            .query_row(
                "SELECT COALESCE(CAST(json_extract(data, '$.saved') AS INTEGER), 0) \
                 FROM events WHERE type = 'boot_savings_rollup' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rollup_saved, 50);

        let benchmark_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE type = 'boot_savings' \
                   AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(benchmark_rows, 1);
    }

    #[test]
    fn test_prune_old_events_keeps_boot_rollups() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings_rollup', ?1, 'compaction', datetime('now', '-90 days'))",
            params![r#"{"saved":1000,"served":500,"baseline":1500,"boots":10}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('decision_stored', '{}', 'test', datetime('now', '-90 days'))",
            [],
        )
        .unwrap();

        let pruned = prune_old_events_with_retention(&conn, 30);
        assert_eq!(
            pruned, 1,
            "only non-rollup historical events should be pruned"
        );

        let rollup_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings_rollup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rollup_rows, 1, "boot_savings_rollup must be retained");
    }

    #[test]
    fn test_strip_archived_text() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memories (text, source, status, updated_at) \
             VALUES ('important data', 'test', 'archived', datetime('now', '-120 days'))",
            [],
        )
        .unwrap();

        let stripped = strip_archived_text(&conn);
        assert_eq!(stripped, 1);

        let text: String = conn
            .query_row("SELECT text FROM memories WHERE source = 'test'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(text, "[compacted]");
    }

    #[test]
    fn test_strip_archived_text_compacts_superseded_decisions() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, updated_at) \
             VALUES ('superseded text', 'old context', 'superseded', datetime('now', '-120 days'))",
            [],
        )
        .unwrap();

        let stripped = strip_archived_text(&conn);
        assert_eq!(stripped, 1);

        let row: (String, Option<String>) = conn
            .query_row(
                "SELECT decision, context FROM decisions WHERE status = 'superseded' LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "[compacted]");
        assert!(row.1.is_none());
    }

    #[test]
    fn test_compaction_full_pass() {
        let conn = setup();
        let result = run_compaction(&conn);
        // Empty DB should compact cleanly
        assert_eq!(result.events_pruned, 0);
        assert_eq!(result.archived_text_stripped, 0);
        assert_eq!(result.expired_pruned, 0);
    }

    #[test]
    fn test_storage_breakdown() {
        let conn = setup();
        let breakdown = storage_breakdown(&conn);
        assert!(!breakdown.is_empty());
        // All counts should be 0 for empty DB
        assert!(breakdown.iter().all(|(_, count)| *count == 0));
    }

    #[test]
    fn test_storage_pressure_classification() {
        assert_eq!(
            classify_storage_pressure(STORAGE_SOFT_LIMIT_BYTES - 1),
            "normal"
        );
        assert_eq!(
            classify_storage_pressure(STORAGE_SOFT_LIMIT_BYTES),
            "elevated"
        );
        assert_eq!(
            classify_storage_pressure(STORAGE_HARD_LIMIT_BYTES),
            "critical"
        );
    }

    #[test]
    fn test_event_pressure_classification() {
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_SOFT_LIMIT_ROWS - 1),
            "normal"
        );
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_SOFT_LIMIT_ROWS),
            "elevated"
        );
        assert_eq!(
            classify_event_pressure(EVENT_NONBOOT_HARD_LIMIT_ROWS),
            "critical"
        );
    }

    #[test]
    fn test_storage_governor_thresholds() {
        assert!(!should_run_compaction_governor(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES
        ));
        assert!(should_run_compaction_governor(STORAGE_SOFT_LIMIT_BYTES, 0));
        assert!(should_run_compaction_governor(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES + 1
        ));
        assert!(should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            EVENT_NONBOOT_SOFT_LIMIT_ROWS + 1,
            0,
        ));
    }

    #[test]
    fn test_governor_triggers_on_fts_segment_pressure() {
        // No size pressure, no freelist pressure, no event pressure — but FTS
        // segment count above the soft limit MUST still trigger the governor,
        // because that is the bloat dimension a healthy DB can hide.
        assert!(!should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            0,
            FTS_SEGMENT_ROW_SOFT_LIMIT,
        ));
        assert!(should_run_compaction_governor_with_pressure(
            STORAGE_SOFT_LIMIT_BYTES - 1,
            VACUUM_FREELIST_THRESHOLD_PAGES,
            0,
            FTS_SEGMENT_ROW_SOFT_LIMIT + 1,
        ));
    }

    #[test]
    fn test_fts_segment_row_total_counts_known_tables() {
        let conn = setup();
        // Fresh schema: FTS shadow tables exist but should be near-empty.
        let baseline = fts_segment_row_total(&conn);
        // Force several inserts + updates to grow segments.
        for i in 0..50 {
            conn.execute(
                "INSERT INTO decisions (decision, context, type, source_agent, status) \
                 VALUES (?1, 'ctx', 'decision', 'test', 'active')",
                params![format!("decision-{i} alpha beta gamma delta epsilon")],
            )
            .unwrap();
        }
        let after_inserts = fts_segment_row_total(&conn);
        assert!(
            after_inserts > baseline,
            "decisions inserts should bump fts_segment_row_total ({baseline} -> {after_inserts})"
        );
    }

    #[test]
    fn test_pq8_migration_handles_crystal_centroid_blobs() {
        let conn = setup();
        // Seed two clusters: one legacy LE-f32 centroid, one already-PQ8.
        let legacy_centroid = crate::embeddings::vector_to_legacy_f32_blob(&[
            0.10, -0.20, 0.30, -0.40, 0.50, -0.60, 0.70, -0.80,
        ]);
        let already_pq8 = crate::embeddings::vector_to_pq8_blob(&[
            0.11, -0.21, 0.31, -0.41, 0.51, -0.61, 0.71, -0.81,
        ]);
        conn.execute(
            "INSERT INTO memory_clusters (label, centroid, consolidated_text) \
             VALUES ('legacy', ?1, 'legacy crystal'), ('pq8', ?2, 'pq8 crystal')",
            params![legacy_centroid, already_pq8],
        )
        .unwrap();

        let migrated = migrate_legacy_embeddings_to_pq8(&conn);
        assert_eq!(
            migrated, 1,
            "exactly one centroid (the legacy one) should migrate"
        );

        // Both centroids must now carry the PQ8 signature.
        let mut stmt = conn
            .prepare("SELECT label, centroid FROM memory_clusters ORDER BY label")
            .unwrap();
        let rows: Vec<(String, Vec<u8>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(rows.len(), 2);
        for (label, blob) in &rows {
            assert!(blob.len() >= 2, "{label} centroid too short to carry magic");
            assert_eq!(
                blob[0],
                crate::embeddings::PQ8_MAGIC_BYTE,
                "{label} centroid missing PQ8 magic"
            );
            assert_eq!(
                blob[1],
                crate::embeddings::PQ8_FORMAT_VERSION,
                "{label} centroid wrong PQ8 version"
            );
        }

        // Idempotent: re-running finds nothing new to migrate.
        assert_eq!(migrate_legacy_embeddings_to_pq8(&conn), 0);
    }

    #[test]
    fn test_pq8_migration_catches_legacy_blobs_with_collision_byte() {
        // Regression: legacy LE-f32 blobs whose first byte coincidentally
        // equals PQ8_MAGIC_BYTE (0xC8) used to escape migration. The
        // signature is now 2 bytes (magic + version) plus a length-mod-4
        // gate, which legacy blobs always pass and PQ8 blobs never do.
        let conn = setup();
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('legacy-collision', 'memory::collision', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let mid: i64 = conn.last_insert_rowid();

        // Construct a legacy LE-f32 blob whose first byte is 0xC8 by
        // choosing an f32 whose LE encoding starts with 0xC8.
        let collide_f32 = f32::from_le_bytes([0xC8, 0xAB, 0x12, 0x34]);
        let legacy_vec = vec![collide_f32; 16];
        let legacy_blob = crate::embeddings::vector_to_legacy_f32_blob(&legacy_vec);
        assert_eq!(legacy_blob[0], 0xC8, "first byte should collide with magic");
        assert_ne!(
            legacy_blob[1],
            crate::embeddings::PQ8_FORMAT_VERSION,
            "second byte should NOT be the version byte"
        );
        assert_eq!(legacy_blob.len() % 4, 0);

        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) \
             VALUES ('memory', ?1, ?2, 'bge-base-en-v1.5')",
            params![mid, legacy_blob],
        )
        .unwrap();

        let migrated = migrate_legacy_embeddings_to_pq8(&conn);
        assert_eq!(
            migrated, 1,
            "legacy blob with 0xC8 leading byte must still be detected"
        );

        // Migrated row carries the full 2-byte signature.
        let new_blob: Vec<u8> = conn
            .query_row(
                "SELECT vector FROM embeddings WHERE target_id = ?1",
                params![mid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(new_blob[0], crate::embeddings::PQ8_MAGIC_BYTE);
        assert_eq!(new_blob[1], crate::embeddings::PQ8_FORMAT_VERSION);
    }

    #[test]
    fn test_pq8_migration_reencodes_legacy_blobs() {
        let conn = setup();
        // Insert a memory row and a paired legacy LE-f32 embedding directly
        // — this is what pre-v0.6.0 storage produced.
        conn.execute(
            "INSERT INTO memories (text, source, type, status, score, created_at, updated_at) \
             VALUES ('legacy', 'memory::legacy', 'note', 'active', 1.0, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        let mid: i64 = conn.last_insert_rowid();
        let legacy_vec: Vec<f32> = (0..16).map(|i| (i as f32) * 0.05).collect();
        let legacy_blob = crate::embeddings::vector_to_legacy_f32_blob(&legacy_vec);
        assert_eq!(legacy_blob.len(), legacy_vec.len() * 4);
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) \
             VALUES ('memory', ?1, ?2, 'bge-base-en-v1.5')",
            params![mid, legacy_blob],
        )
        .unwrap();

        let migrated = migrate_legacy_embeddings_to_pq8(&conn);
        assert_eq!(migrated, 1, "exactly one legacy blob should be migrated");

        // The stored blob is now PQ8 — magic byte and length both shrink.
        let (new_blob, len): (Vec<u8>, i64) = conn
            .query_row(
                "SELECT vector, LENGTH(vector) FROM embeddings WHERE target_id = ?1",
                params![mid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(new_blob[0], crate::embeddings::PQ8_MAGIC_BYTE);
        assert_eq!(
            len as usize,
            crate::embeddings::PQ8_HEADER_BYTES + legacy_vec.len()
        );

        // A second migration pass becomes a no-op once every row is PQ8.
        let migrated_again = migrate_legacy_embeddings_to_pq8(&conn);
        assert_eq!(migrated_again, 0, "idempotent: nothing left to migrate");

        // Recall fidelity: re-reading via blob_to_vector should match the
        // original within the per-vector quantization scale.
        let recovered = crate::embeddings::blob_to_vector(&new_blob);
        let scale = f32::from_le_bytes([new_blob[2], new_blob[3], new_blob[4], new_blob[5]]);
        let max_err = legacy_vec
            .iter()
            .zip(recovered.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_err <= scale,
            "max err {max_err} should be within scale {scale}"
        );
    }

    #[test]
    fn test_fts_optimize_drops_segment_rows() {
        let conn = setup();
        // Generate UPDATE churn so the FTS shadow accumulates segment rows.
        for i in 0..30 {
            conn.execute(
                "INSERT INTO decisions (decision, context, type, source_agent, status) \
                 VALUES (?1, 'ctx', 'decision', 'test', 'active')",
                params![format!("seed-{i} apple banana cherry")],
            )
            .unwrap();
        }
        for round in 0..5 {
            conn.execute(
                "UPDATE decisions SET decision = decision || ?1 WHERE source_agent = 'test'",
                params![format!(" round{round}")],
            )
            .unwrap();
        }
        let pre = fts_segment_row_total(&conn);
        assert!(pre > 0, "fixture should produce FTS segment rows");
        let optimized = optimize_fts_indexes(&conn);
        assert!(optimized, "optimize should report success on populated FTS");
        let post = fts_segment_row_total(&conn);
        assert!(
            post < pre,
            "optimize should reduce segment row count: {pre} -> {post}"
        );
    }

    #[test]
    fn test_startup_governor_relieves_pressure_without_vacuum() {
        let conn = setup();
        let payload = "x".repeat(4096);
        for i in 0..600 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{payload}{i}")],
            )
            .unwrap();
        }
        conn.execute("DELETE FROM events WHERE type = 'decision_stored'", [])
            .unwrap();
        let freelist_before = freelist_count(&conn);
        assert!(
            freelist_before > VACUUM_FREELIST_THRESHOLD_PAGES,
            "fixture should create enough reclaimable pages to trigger governor"
        );

        let result = run_compaction_governor_startup(&conn);
        assert!(
            result.is_some(),
            "startup governor should run when freelist pressure is high"
        );
        let freelist_after = freelist_count(&conn);
        assert!(
            freelist_after > 0,
            "startup governor should skip VACUUM to keep early lock windows shorter"
        );
    }

    #[test]
    fn test_event_type_caps_prune_oldest_rows() {
        let conn = setup();
        for i in 0..10 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent, created_at)
                 VALUES ('decision_stored', ?1, 'test', datetime('now', ?2))",
                params![format!("{{\"i\":{i}}}"), format!("-{} minutes", 10 - i)],
            )
            .unwrap();
        }

        let pruned = prune_event_type_caps(&conn, &[("decision_stored", 3)]);
        assert_eq!(pruned, 7);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 3);
    }

    #[test]
    fn test_nonboot_event_overflow_prunes_only_nonboot_rows() {
        let conn = setup();
        for i in 0..8 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }
        for _ in 0..3 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('agent_boot', '{}', 'test')",
                [],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('boot_savings', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('boot_savings_rollup', '{}', 'test')",
            [],
        )
        .unwrap();

        let pruned = prune_nonboot_event_overflow(&conn, 2);
        assert_eq!(pruned, 6);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let agent_boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'agent_boot'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rollup_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings_rollup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 2);
        assert_eq!(agent_boot_rows, 3);
        assert_eq!(boot_rows, 1);
        assert_eq!(boot_rollup_rows, 1);
    }

    #[test]
    fn test_nonboot_event_overflow_preserves_savings_analytics_rows() {
        let conn = setup();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('recall_query', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('store_savings', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('tool_call_savings', '{}', 'test')",
            [],
        )
        .unwrap();

        // keep_rows=3 equals the number of protected analytics rows, so all
        // non-analytics non-boot rows should be pruned.
        let pruned = prune_nonboot_event_overflow(&conn, 3);
        assert_eq!(pruned, 5);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let protected_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type IN ('recall_query', 'store_savings', 'tool_call_savings')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 0);
        assert_eq!(protected_rows, 3);
    }

    #[test]
    fn test_nonboot_event_overflow_limit_batches_deletes() {
        let conn = setup();
        for i in 0..8 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', ?1, 'test')",
                params![format!("{{\"i\":{i}}}")],
            )
            .unwrap();
        }

        let pruned = prune_nonboot_event_overflow_with_limit(&conn, 2, Some(3));
        assert_eq!(pruned, 3, "startup mode should batch overflow pruning");

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 5);
    }

    #[test]
    fn test_prune_old_events_treats_missing_created_at_as_old() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) VALUES ('decision_stored', '{}', 'test', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) VALUES ('boot_savings', '{}', 'test', NULL)",
            [],
        )
        .unwrap();

        let pruned = prune_old_events_with_retention(&conn, 14);
        assert_eq!(pruned, 1);

        let decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let boot_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'boot_savings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(decision_rows, 0);
        assert_eq!(boot_rows, 1);
    }

    #[test]
    fn test_prune_old_events_limit_batches_deletes() {
        let conn = setup();
        for i in 0..6 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent, created_at) \
                 VALUES ('decision_stored', ?1, 'test', datetime('now', '-40 days', ?2))",
                params![format!("{{\"i\":{i}}}"), format!("+{} minutes", i)],
            )
            .unwrap();
        }

        let pruned = prune_old_events_with_retention_limit(&conn, 14, Some(2));
        assert_eq!(pruned, 2);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'decision_stored'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 4);
    }

    #[test]
    fn test_prune_expired_entries() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memories (text, source, status, expires_at) VALUES ('expired memory', 'ttl::mem', 'active', datetime('now', '-1 second'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at) VALUES ('expired decision', 'ttl::dec', 'active', datetime('now', '-1 second'))",
            [],
        )
        .unwrap();

        let deleted = prune_expired_entries(&conn);
        assert_eq!(deleted, 2);

        let mem_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source = 'ttl::mem'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let dec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context = 'ttl::dec'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mem_count, 0);
        assert_eq!(dec_count, 0);

        let event: (String, String) = conn
            .query_row(
                "SELECT type, data FROM events WHERE source_agent = 'compaction' ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(event.0, "expired_entries_pruned");
        assert!(event.1.contains("\"memories_deleted\":1"));
        assert!(event.1.contains("\"decisions_deleted\":1"));
    }

    #[test]
    fn test_rollup_old_savings_events_compacts_rows() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'test', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "saved": 80,
                "spent": 20,
                "budget": 100,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('store_savings', ?1, 'test', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "saved": 50,
                "served": 25,
                "baseline": 75
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'test', datetime('now', '-1 days'))",
            params![serde_json::json!({
                "saved": 9,
                "spent": 1,
                "budget": 10,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();

        let rolled = rollup_old_savings_events(&conn, 7);
        assert_eq!(rolled, 2);

        let remaining_old: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE created_at < datetime('now', '-7 days') \
                   AND type IN ('recall_query', 'store_savings', 'tool_call_savings')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_old, 0);

        let remaining_recent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE created_at >= datetime('now', '-7 days') \
                   AND type = 'recall_query'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_recent, 1);

        let (saved, served, baseline, events, hits, misses): (i64, i64, i64, i64, i64, i64) = conn
            .query_row(
                "SELECT \
                     COALESCE(SUM(saved), 0), \
                     COALESCE(SUM(served), 0), \
                     COALESCE(SUM(baseline), 0), \
                     COALESCE(SUM(events), 0), \
                     COALESCE(SUM(hits), 0), \
                     COALESCE(SUM(misses), 0) \
                 FROM event_savings_rollups",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(saved, 130);
        assert_eq!(served, 45);
        assert_eq!(baseline, 175);
        assert_eq!(events, 2);
        assert_eq!(hits, 1);
        assert_eq!(misses, 0);
    }

    #[test]
    fn test_rollup_old_savings_events_ignores_benchmark_agent_payloads() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('store_savings', ?1, 'rust-daemon', datetime('now', '-10 days'))",
            params![serde_json::json!({
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
             VALUES ('store_savings', ?1, 'rust-daemon', datetime('now', '-10 days'))",
            params![serde_json::json!({
                "agent": "codex",
                "saved": 50,
                "served": 25,
                "baseline": 75
            })
            .to_string()],
        )
        .unwrap();

        let rolled = rollup_old_savings_events(&conn, 7);
        assert_eq!(
            rolled, 1,
            "benchmark rows should not be rolled into production rollups"
        );

        let rollup_saved: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(saved), 0) FROM event_savings_rollups WHERE operation = 'store'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rollup_saved, 50);

        let benchmark_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE type = 'store_savings' \
                   AND created_at < datetime('now', '-7 days') \
                   AND LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(benchmark_rows, 1);
    }

    #[test]
    fn test_prune_old_event_savings_rollups_respects_retention() {
        let conn = setup();
        conn.execute(
            "INSERT INTO event_savings_rollups (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) \
             VALUES (date('now', '-200 days'), 1, 'recall', 10, 5, 15, 1, 1, 0, datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO event_savings_rollups (day, hour, operation, saved, served, baseline, events, hits, misses, updated_at) \
             VALUES (date('now', '-1 days'), 2, 'store', 20, 10, 30, 2, 0, 0, datetime('now'))",
            [],
        )
        .unwrap();

        let deleted = prune_old_event_savings_rollups(&conn, 120);
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM event_savings_rollups", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_purge_benchmark_artifacts_removes_benchmark_rows_and_dependencies() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memory_clusters (label, consolidated_text, member_count) VALUES ('bench', 'x', 0)",
            [],
        )
        .unwrap();
        let cluster_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (id, decision, context, type, source_agent, status) VALUES (1, 'bench', 'ctx', 'benchmark', 'amb-cortex::run-a', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (id, decision, context, type, source_agent, status) VALUES (2, 'prod', 'ctx', 'decision', 'codex', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decision_conflicts (source_decision_id, target_decision_id, classification) VALUES (1, 2, 'REFINES')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', 1, X'0102', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings (target_type, target_id, vector, model) VALUES ('decision', 2, X'0304', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 1, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 2, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, result_id, signal, agent) VALUES ('q', 'decision::1', 'decision', 1, 1.0, 'amb-cortex::run-a')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_feedback (query_text, result_source, result_type, result_id, signal, agent) VALUES ('q', 'decision::2', 'decision', 2, 1.0, 'codex')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('decision::1', 'memory::x', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO co_occurrence (source_a, source_b, count) VALUES ('decision::2', 'memory::x', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{\"id\":1,\"source_agent\":\"amb-cortex::run-a\"}', 'rust-daemon')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('merge', '{\"source_agent\":\"amb-cortex::run-a\"}', 'rust-daemon')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('merge', '{\"source_agent\":\"codex\"}', 'rust-daemon')",
            [],
        )
        .unwrap();

        let result = purge_benchmark_artifacts(&conn);
        assert_eq!(result.decisions_deleted, 1);
        assert_eq!(result.embeddings_deleted, 1);
        assert!(result.events_deleted >= 2);

        let decisions_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(decisions_remaining, 1);
        let feedback_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM recall_feedback", [], |row| row.get(0))
            .unwrap();
        assert_eq!(feedback_remaining, 1);
        let cooccurrence_remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cooccurrence_remaining, 1);
    }

    #[test]
    fn test_prune_old_benchmark_artifacts_respects_retention_window() {
        let conn = setup();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, created_at, updated_at) VALUES ('old-bench', 'ctx', 'benchmark', 'amb-cortex::run-old', 'active', datetime('now', '-10 days'), datetime('now', '-10 days'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, status, created_at, updated_at) VALUES ('new-bench', 'ctx', 'benchmark', 'amb-cortex::run-new', 'active', datetime('now', '-1 days'), datetime('now', '-1 days'))",
            [],
        )
        .unwrap();

        let deleted = prune_old_benchmark_artifacts(&conn, 7, true);
        assert_eq!(deleted, 1);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE LOWER(type) = 'benchmark'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_prune_old_benchmark_artifacts_removes_data_agent_marked_events() {
        let conn = setup();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('boot_savings', ?1, 'rust-daemon', datetime('now', '-5 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 10,
                "served": 5,
                "baseline": 15
            })
            .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent, created_at) \
             VALUES ('recall_query', ?1, 'rust-daemon', datetime('now', '-5 days'))",
            params![serde_json::json!({
                "agent": "amb-cortex::run-a",
                "saved": 5,
                "spent": 2,
                "budget": 7,
                "hits": 1
            })
            .to_string()],
        )
        .unwrap();

        let deleted = prune_old_benchmark_artifacts(&conn, 2, true);
        assert!(deleted >= 2);

        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events \
                 WHERE LOWER(COALESCE(json_extract(data, '$.agent'), '')) LIKE 'amb-cortex%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_prune_orphan_cluster_members_removes_missing_targets() {
        let conn = setup();
        conn.execute(
            "INSERT INTO memory_clusters (label, consolidated_text, member_count) VALUES ('c1', 'x', 0)",
            [],
        )
        .unwrap();
        let cluster_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memories (text, source, status) VALUES ('m1', 'memory::1', 'active')",
            [],
        )
        .unwrap();
        let memory_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO decisions (decision, context, status) VALUES ('d1', 'ctx', 'active')",
            [],
        )
        .unwrap();
        let decision_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'memory', ?2, 1.0)",
            params![cluster_id, memory_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', ?2, 1.0)",
            params![cluster_id, decision_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'decision', 999999, 1.0)",
            params![cluster_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cluster_members (cluster_id, target_type, target_id, similarity) VALUES (?1, 'memory', 999999, 1.0)",
            params![cluster_id],
        )
        .unwrap();

        let pruned = prune_orphan_cluster_members(&conn);
        assert_eq!(pruned, 2);

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM cluster_members", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
    }

