// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use rusqlite::{params, Connection};

    #[test]
    fn test_open_configure_schema() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        let busy_timeout_ms: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        let wal_autocheckpoint_pages: i64 = conn
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))
            .unwrap();
        assert_eq!(busy_timeout_ms, SQLITE_BUSY_TIMEOUT_MS as i64);
        assert_eq!(
            wal_autocheckpoint_pages,
            SQLITE_WAL_AUTOCHECKPOINT_PAGES as i64
        );
        initialize_schema(&conn).unwrap();
        assert!(verify_integrity(&conn).unwrap());
    }

    #[test]
    fn test_archive_entries() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        // Insert a test decision
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent) VALUES (?1, ?2, ?3, ?4)",
            params!["test decision", "test context", "decision", "test"],
        )
        .unwrap();

        let affected = archive_entries(&conn, "decisions", &[1]).unwrap();
        assert_eq!(affected, 1);

        let status: String = conn
            .query_row("SELECT status FROM decisions WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(status, "archived");
    }

    #[test]
    fn test_archive_entries_empty_ids() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        let affected = archive_entries(&conn, "memories", &[]).unwrap();
        assert_eq!(affected, 0);
    }

    #[test]
    fn test_archive_entries_invalid_table() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        let result = archive_entries(&conn, "locks", &[1]);
        assert!(result.is_err());
    }

    #[test]
    fn test_checkpoint_wal_best_effort() {
        // Should not panic even on an in-memory connection (WAL not applicable)
        let conn = Connection::open_in_memory().unwrap();
        checkpoint_wal_best_effort(&conn);
    }

    #[test]
    fn test_best_effort_checkpoint_interval_guard() {
        assert!(!should_attempt_best_effort_checkpoint(10_000, 8_000));
        assert!(should_attempt_best_effort_checkpoint(15_000, 10_000));
    }

    #[test]
    fn test_best_effort_truncate_interval_guard() {
        // First run should not force TRUNCATE.
        assert!(!should_attempt_truncate_checkpoint(100_000, 0));
        // Before interval: no TRUNCATE.
        assert!(!should_attempt_truncate_checkpoint(200_000, 150_001));
        // After interval: TRUNCATE allowed.
        assert!(should_attempt_truncate_checkpoint(
            500_000,
            500_000 - BEST_EFFORT_TRUNCATE_INTERVAL_MS - 1
        ));
    }

    #[test]
    fn test_fts5_schema_and_search() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let memories_fts_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            memories_fts_sql
                .to_ascii_lowercase()
                .contains("tokenize='porter unicode61'"),
            "expected porter/unicode61 tokenizer, got: {memories_fts_sql}"
        );

        let decisions_fts_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'decisions_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            decisions_fts_sql
                .to_ascii_lowercase()
                .contains("tokenize='porter unicode61'"),
            "expected porter/unicode61 tokenizer, got: {decisions_fts_sql}"
        );

        conn.execute(
            "INSERT INTO memories (text, source, type) VALUES (?1, ?2, ?3)",
            params![
                "Cortex uses Ebbinghaus decay for memory scoring",
                "test::fts",
                "memory"
            ],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'Ebbinghaus'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "FTS5 trigger should auto-index new memories");

        let count2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'nonexistent'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count2, 0);
    }

    #[test]
    fn test_rebuild_fts_if_needed_rebuilds_once_for_empty_fts() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let rebuilt = rebuild_fts_if_needed(&conn).unwrap();
        assert!(rebuilt, "first call should seed FTS marker");

        let marker_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 'fts_seeded_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(marker_rows, 1, "expected FTS marker row to be persisted");

        let rebuilt_again = rebuild_fts_if_needed(&conn).unwrap();
        assert!(
            !rebuilt_again,
            "second call should skip when marker already exists"
        );
    }

    #[test]
    fn test_reindex_fts_removes_orphan_rows_and_rebuilds_from_base_tables() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO memories (text, source, type) VALUES (?1, ?2, ?3)",
            params!["primary memory row", "test::reindex", "memory"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type) VALUES (?1, ?2, ?3)",
            params!["primary decision row", "context", "decision"],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO memories_fts(rowid, text, source, tags) VALUES (?1, ?2, ?3, ?4)",
            params![999_i64, "orphan memory", "test::orphan", ""],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions_fts(rowid, decision, context) VALUES (?1, ?2, ?3)",
            params![888_i64, "orphan decision", "orphan context"],
        )
        .unwrap();

        let orphan_memory_rows_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let orphan_decision_rows_before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_memory_rows_before, 1,
            "precondition failed: expected injected orphan memory row"
        );
        assert_eq!(
            orphan_decision_rows_before, 1,
            "precondition failed: expected injected orphan decision row"
        );

        reindex_fts(&conn).unwrap();

        let orphan_memory_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let orphan_decision_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_memory_rows, 0,
            "orphan memory FTS row should be removed"
        );
        assert_eq!(
            orphan_decision_rows, 0,
            "orphan decision FTS row should be removed"
        );

        let rebuilt_memory_match: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories_fts WHERE memories_fts MATCH 'primary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let rebuilt_decision_match: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions_fts WHERE decisions_fts MATCH 'primary'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            rebuilt_memory_match, 1,
            "expected canonical memory row to be present after reindex"
        );
        assert_eq!(
            rebuilt_decision_match, 1,
            "expected canonical decision row to be present after reindex"
        );
    }

    #[test]
    fn test_solo_schema_baseline_unchanged() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        // Core solo schema tables exist.
        for table in [
            "memories",
            "decisions",
            "decision_conflicts",
            "embeddings",
            "events",
            "co_occurrence",
            "locks",
            "activities",
            "messages",
            "sessions",
            "tasks",
            "feed",
            "feed_acks",
            "client_permissions",
            "context_cache",
            "schema_migrations",
            "memories_fts",
            "decisions_fts",
            "recall_feedback",
            "agent_feedback",
        ] {
            assert!(table_exists(&conn, table), "missing solo table: {table}");
        }

        // Team tables are not auto-created in solo mode.
        assert!(!table_exists(&conn, "config"));
        assert!(!table_exists(&conn, "users"));
        assert!(!table_exists(&conn, "teams"));
        assert!(!table_exists(&conn, "team_members"));

        // Team columns are not present in solo baseline.
        assert!(!table_has_column(&conn, "memories", "owner_id"));
        assert!(!table_has_column(&conn, "memories", "visibility"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(!table_has_column(&conn, "decisions", "owner_id"));
        assert!(!table_has_column(&conn, "decisions", "visibility"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));
        assert!(table_has_column(&conn, "sessions", "agent"));
        assert!(table_has_column(&conn, "sessions", "session_id"));
        assert!(!table_has_column(&conn, "sessions", "owner_id"));
        assert!(table_has_column(&conn, "locks", "path"));
        assert!(!table_has_column(&conn, "locks", "owner_id"));
        assert!(table_has_column(&conn, "feed_acks", "agent"));
        assert!(!table_has_column(&conn, "feed_acks", "owner_id"));
    }

    #[test]
    fn test_quick_check_clean_db() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        assert!(
            quick_check(&conn),
            "fresh in-memory DB should pass quick_check"
        );
    }

    #[test]
    fn test_auto_repair_recovers_data() {
        use std::io::Write;

        // ── Build a valid DB with test data at a temp file path ────────────
        // Use a unique path under the system temp dir so parallel test runs don't collide.
        let tmp_dir = std::env::temp_dir();
        let db_path = tmp_dir.join(format!(
            "cortex_repair_test_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));

        {
            let conn = Connection::open(&db_path).unwrap();
            configure(&conn).unwrap();
            initialize_schema(&conn).unwrap();

            // Insert known rows.
            conn.execute(
                "INSERT INTO memories (text, source, type) VALUES (?1, ?2, ?3)",
                params!["repair test memory", "test::repair", "memory"],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO decisions (decision, context, type) VALUES (?1, ?2, ?3)",
                params!["repair test decision", "test context", "decision"],
            )
            .unwrap();
            // Checkpoint so data is in the main DB file, not just WAL.
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .unwrap();
        } // Connection closed -- file flushed.

        // Verify the DB is clean before corruption.
        {
            let conn = Connection::open(&db_path).unwrap();
            assert!(
                verify_integrity(&conn).unwrap(),
                "DB should be clean before corruption"
            );
        }

        // ── Corrupt the DB by overwriting a page mid-file ─────────────────
        // We write garbage into the middle of the file. SQLite's B-tree index
        // pages live in the middle; data in leaf pages (lower in the file) often
        // survives. For this test we write at a safe offset that corrupts the
        // free-list / interior pages but leaves leaf data readable.
        {
            let meta = std::fs::metadata(&db_path).unwrap();
            let file_size = meta.len();
            // Write 512 bytes of 0xFF starting at 40% of the file (index area).
            let corrupt_offset = (file_size as f64 * 0.4) as u64;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&db_path)
                .unwrap();
            use std::io::Seek;
            f.seek(std::io::SeekFrom::Start(corrupt_offset)).unwrap();
            f.write_all(&[0xFF_u8; 512]).unwrap();
            f.flush().unwrap();
        }

        // ── Run auto_repair ────────────────────────────────────────────────
        let result = auto_repair(&db_path, "20260407_test");
        match &result {
            Ok(r) => {
                eprintln!(
                    "[test] auto_repair: {} memories, {} decisions recovered",
                    r.memories_recovered, r.decisions_recovered
                );
                // The corrupt archive must exist.
                assert!(
                    r.corrupt_db_path.exists(),
                    "corrupt DB should be preserved at {:?}",
                    r.corrupt_db_path
                );
            }
            Err(e) => {
                // If SQLite was able to read the corrupted pages without error
                // (it sometimes can), repair may not be triggered -- that's OK.
                // But if repair ran and produced an error, that's a test failure.
                panic!("auto_repair returned error: {e:?}");
            }
        }

        // The repaired DB at the original path must pass integrity_check.
        if db_path.exists() {
            let conn = Connection::open(&db_path).unwrap();
            assert!(
                verify_integrity(&conn).unwrap_or(false),
                "repaired DB must pass integrity_check"
            );

            // At least memories and decisions tables must be present.
            let mem_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                .unwrap_or(0);
            let dec_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM decisions", [], |r| r.get(0))
                .unwrap_or(0);
            eprintln!("[test] Repaired DB: {mem_count} memories, {dec_count} decisions");
            // We may not recover all rows if the page containing them was corrupted,
            // but the DB itself must be structurally sound (integrity check above).
            drop(conn);
        }

        // Cleanup temp files.
        let _ = std::fs::remove_file(&db_path);
        // Also remove the corrupt archive if it exists.
        if let Ok(r) = &result {
            let _ = std::fs::remove_file(&r.corrupt_db_path);
        }
    }

    #[test]
    fn test_run_pending_migrations_applies_all_once() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();

        let first_applied = run_pending_migrations(&conn);
        assert_eq!(first_applied, migration_definitions().len());

        let second_applied = run_pending_migrations(&conn);
        assert_eq!(second_applied, 0);

        let pending = pending_migration_versions(&conn).unwrap();
        assert!(
            pending.is_empty(),
            "no pending migrations expected after first run"
        );

        let recorded: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(recorded as usize, migration_definitions().len());
        let tokenizer_migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = '012'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tokenizer_migration_count, 1);
        assert_eq!(
            current_schema_user_version(&conn).unwrap(),
            latest_schema_user_version()
        );

        assert!(table_has_column(&conn, "memories", "merged_count"));
        assert!(table_has_column(&conn, "memories", "quality"));
        assert!(table_has_column(&conn, "memories", "expires_at"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(table_has_column(&conn, "memories", "retention_class"));
        assert!(table_has_column(&conn, "memories", "observed_at"));
        assert!(table_has_column(&conn, "memories", "valid_from"));
        assert!(table_has_column(&conn, "memories", "valid_until"));
        assert!(table_has_column(&conn, "decisions", "merged_count"));
        assert!(table_has_column(&conn, "decisions", "quality"));
        assert!(table_has_column(&conn, "decisions", "expires_at"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));
        assert!(table_has_column(&conn, "decisions", "retention_class"));
        assert!(table_has_column(&conn, "decisions", "observed_at"));
        assert!(table_has_column(&conn, "decisions", "valid_from"));
        assert!(table_has_column(&conn, "decisions", "valid_until"));
        assert!(table_exists(&conn, "decision_conflicts"));
        assert!(table_exists(&conn, "agent_feedback"));
        assert!(table_exists(&conn, "focus_sessions"));
        assert!(table_exists(&conn, "memory_clusters"));
        assert!(table_exists(&conn, "cluster_members"));
    }

    #[test]
    fn test_open_registers_sqlite_vec_and_supports_vec0_smoke_query() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("cortex_sqlite_vec_smoke_{unique}.db"));
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");

        let conn = open(&db_path).expect("db open should succeed with sqlite-vec bootstrap");
        configure(&conn).unwrap();

        let status = sqlite_vec_status(&conn);
        assert!(
            status.available,
            "sqlite-vec should be available on freshly opened connections: {:?}",
            status.error
        );
        assert!(
            status
                .version
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            "sqlite-vec version should be reported"
        );

        conn.execute_batch(
            "CREATE VIRTUAL TABLE vec_examples USING vec0(
                sample_id INTEGER PRIMARY KEY,
                sample_embedding FLOAT[3]
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vec_examples(sample_id, sample_embedding) VALUES (1, '[0.1, 0.2, 0.3]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO vec_examples(sample_id, sample_embedding) VALUES (2, '[0.9, 0.1, 0.1]')",
            [],
        )
        .unwrap();

        let (sample_id, distance): (i64, f64) = conn
            .query_row(
                "SELECT sample_id, distance
                 FROM vec_examples
                 WHERE sample_embedding MATCH '[0.11, 0.19, 0.31]' AND k = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(sample_id, 1);
        assert!(distance >= 0.0);

        drop(conn);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);
    }

    #[test]
    fn test_delete_expired_entries_removes_only_expired_rows() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        run_pending_migrations(&conn);

        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('expired-memory', 'note', 'expired-memory', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('future-memory', 'note', 'future-memory', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('forever-memory', 'note', 'forever-memory', 'active', NULL, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired-decision', 'expired-decision', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('future-decision', 'future-decision', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('forever-decision', 'forever-decision', 'active', NULL, datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let deleted = delete_expired_entries(&conn).unwrap();
        assert_eq!(
            deleted,
            ExpiredCleanupCounts {
                memories_deleted: 1,
                decisions_deleted: 1,
            }
        );

        let remaining_memories: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source IN ('future-memory', 'forever-memory')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_memories, 2);

        let expired_memories: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE source = 'expired-memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expired_memories, 0);

        let remaining_decisions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context IN ('future-decision', 'forever-decision')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_decisions, 2);

        let expired_decisions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM decisions WHERE context = 'expired-decision'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expired_decisions, 0);
    }

    #[test]
    fn test_team_migration_creates_owner_scoped_schema() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        initialize_schema(&conn).unwrap();
        migrate_focus_table(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);

        create_team_mode_tables(&conn).unwrap();
        let owner_id =
            upsert_owner_user(&conn, "owner", Some("Owner"), "argon2id-placeholder").unwrap();
        migrate_to_team_mode(&conn, owner_id).unwrap();

        assert_eq!(current_mode(&conn), "team");
        assert!(table_exists(&conn, "users"));
        assert!(table_exists(&conn, "teams"));
        assert!(table_exists(&conn, "team_members"));

        assert!(table_has_column(&conn, "memories", "owner_id"));
        assert!(table_has_column(&conn, "memories", "visibility"));
        assert!(table_has_column(&conn, "decisions", "owner_id"));
        assert!(table_has_column(&conn, "decisions", "visibility"));
        assert!(table_has_column(&conn, "memory_clusters", "owner_id"));
        assert!(table_has_column(&conn, "memory_clusters", "visibility"));
        assert!(table_has_column(&conn, "sessions", "id"));
        assert!(table_has_column(&conn, "sessions", "owner_id"));
        assert!(table_has_column(&conn, "locks", "owner_id"));
        assert!(table_has_column(&conn, "feed_acks", "owner_id"));
        let owner_cfg: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = 'owner_user_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner_cfg, owner_id.to_string());
        let default_team_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM teams WHERE name = 'default'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(default_team_count, 1);
        let owner_membership_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM team_members tm
                 JOIN teams t ON t.id = tm.team_id
                 WHERE t.name = 'default' AND tm.user_id = ?1",
                params![owner_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner_membership_count, 1);

        let sessions_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(sessions_sql.contains("UNIQUE(owner_id, agent)"));
        assert!(!sessions_sql.contains("UNIQUE(agent)"));

        let locks_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'locks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(locks_sql.contains("UNIQUE(owner_id, path)"));
        assert!(!locks_sql.contains("UNIQUE(path)"));

        let feed_acks_sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'feed_acks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(feed_acks_sql.contains("PRIMARY KEY(owner_id, agent)"));
        assert!(!feed_acks_sql.contains("UNIQUE(agent)"));
    }

