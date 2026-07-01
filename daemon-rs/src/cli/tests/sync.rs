// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    #[test]
    fn sync_changeset_filename_filter_is_strict() {
        assert!(is_sync_changeset_file_name(
            "changeset-abc-20260419T101112000Z.json"
        ));
        assert!(is_sync_changeset_file_name("changeset-node-1.json"));
        assert!(!is_sync_changeset_file_name("changeset-node-1.txt"));
        assert!(!is_sync_changeset_file_name("metrics.json"));
    }

    #[test]
    fn resolve_sync_since_prefers_override_then_cursor_file() {
        let home_dir = temp_test_dir("sync_cursor_resolution");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let cursor_file = home_dir.join("cursor.txt");

        let override_since = "2026-04-19T00:00:00Z";
        assert_eq!(
            resolve_sync_since(Some(override_since), Some(&cursor_file)),
            Some(override_since.to_string())
        );
        assert_eq!(resolve_sync_since(None, Some(&cursor_file)), None);

        write_sync_cursor_file(&cursor_file, "2026-04-20T00:00:00Z").expect("write cursor");
        assert_eq!(
            resolve_sync_since(None, Some(&cursor_file)),
            Some("2026-04-20T00:00:00Z".to_string())
        );

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn atomic_text_write_replaces_existing_file() {
        let home_dir = temp_test_dir("atomic_text_write");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let path = home_dir.join("changeset.json");

        write_atomic_text_file(&path, "{\"old\":true}\n").expect("write initial file");
        write_atomic_text_file(&path, "{\"new\":true}\n").expect("replace file");

        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"new\":true}\n");

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn general_import_metadata_allows_legacy_json_without_version() {
        let payload = json!({
            "memories": [{"text": "legacy memory"}],
            "decisions": []
        });

        validate_import_payload_metadata(&payload, ImportPayloadExpectation::GeneralJson)
            .expect("legacy JSON import remains compatible");
    }

    #[test]
    fn import_metadata_rejects_unsupported_version_marker() {
        let payload = json!({
            "version": 2,
            "memories": [],
            "decisions": []
        });

        let err = validate_import_payload_metadata(&payload, ImportPayloadExpectation::GeneralJson)
            .expect_err("unsupported version must fail");
        assert!(err.contains("unsupported version marker"));
    }

    #[test]
    fn sync_import_metadata_requires_changeset_mode_and_cursor() {
        let payload = json!({
            "version": 1,
            "memories": [],
            "decisions": [],
            "memories_count": 0,
            "decisions_count": 0
        });

        let err =
            validate_import_payload_metadata(&payload, ImportPayloadExpectation::SyncChangeset)
                .expect_err("sync import requires changeset metadata");
        assert!(err.contains("mode=\"changeset\""));

        let missing_cursor = json!({
            "version": 1,
            "mode": "changeset",
            "memories": [],
            "decisions": [],
            "memories_count": 0,
            "decisions_count": 0
        });
        let err = validate_import_payload_metadata(
            &missing_cursor,
            ImportPayloadExpectation::SyncChangeset,
        )
        .expect_err("sync import requires cursor marker");
        assert!(err.contains("missing cursor"));

        let missing_counts = json!({
            "version": 1,
            "mode": "changeset",
            "cursor": "2026-04-20T00:00:00Z",
            "memories": [],
            "decisions": []
        });
        let err = validate_import_payload_metadata(
            &missing_counts,
            ImportPayloadExpectation::SyncChangeset,
        )
        .expect_err("sync import requires count markers");
        assert!(err.contains("missing required memories_count marker"));
    }

    #[test]
    fn sync_import_metadata_rejects_count_marker_mismatch() {
        let payload = json!({
            "version": 1,
            "mode": "changeset",
            "cursor": "2026-04-20T00:00:00Z",
            "exported_at": "2026-04-20T00:00:00Z",
            "memories": [{"text": "one"}],
            "decisions": [],
            "memories_count": 2,
            "decisions_count": 0
        });

        let err =
            validate_import_payload_metadata(&payload, ImportPayloadExpectation::SyncChangeset)
                .expect_err("count marker mismatch must fail before import");
        assert!(err.contains("memories_count marker"));
    }

    #[test]
    fn cli_connection_applies_pending_migrations_for_fresh_db() {
        let home_dir = temp_test_dir("cli_connection_migrations");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let db_path = home_dir.join("cortex.db");

        let conn = open_cli_connection(&db_path).expect("open cli connection");
        let pending =
            db::pending_migration_versions(&conn).expect("read pending migration versions");
        assert!(
            pending.is_empty(),
            "fresh CLI DB should not have pending migrations: {pending:?}"
        );
        assert!(
            db::table_exists(&conn, "focus_sessions"),
            "fresh CLI DB should include migrated focus table"
        );

        let _ = std::fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn sync_watch_candidate_collection_skips_local_site_files() {
        let watch_dir = temp_test_dir("sync_watch_candidates");
        fs::create_dir_all(&watch_dir).expect("create watch dir");
        fs::write(
            watch_dir.join("changeset-local-site-20260419T100000000Z.json"),
            "{}",
        )
        .expect("write local file");
        fs::write(
            watch_dir.join("changeset-remote-site-20260419T100001000Z.json"),
            "{}",
        )
        .expect("write remote file");
        fs::write(watch_dir.join("notes.txt"), "ignore").expect("write noise file");

        let files = collect_sync_watch_import_candidates(&watch_dir, "local-site")
            .expect("collect sync watch files");
        assert_eq!(files.len(), 1);
        let name = files[0]
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        assert_eq!(name, "changeset-remote-site-20260419T100001000Z.json");

        let _ = std::fs::remove_dir_all(&watch_dir);
    }

    #[test]
    fn event_type_count_helpers_return_expected_rows() {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        conn.execute(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT,
                source_agent TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create events table");
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (type, data, source_agent) VALUES ('recall_query', '{}', 'test')",
            [],
        )
        .unwrap();

        assert_eq!(event_type_count(&conn, "decision_stored"), 2);
        assert_eq!(event_type_count(&conn, "recall_query"), 1);
        assert_eq!(event_type_count(&conn, "missing"), 0);

        let top = top_event_type_counts(&conn, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0], ("decision_stored".to_string(), 2));
        assert_eq!(top[1], ("recall_query".to_string(), 1));
    }

    #[test]
    fn run_event_compaction_cleanup_dry_run_reports_preview_lines() {
        let home_dir = temp_test_dir("event_cleanup_dry_run");
        fs::create_dir_all(&home_dir).expect("create temp home");
        let db_path = home_dir.join("cortex.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        db::configure(&conn).expect("configure db");
        conn.execute(
            "CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                data TEXT,
                source_agent TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        )
        .expect("create events table");
        for _ in 0..5 {
            conn.execute(
                "INSERT INTO events (type, data, source_agent) VALUES ('decision_stored', '{}', 'test')",
                [],
            )
            .expect("insert event");
        }
        drop(conn);

        let lines =
            run_event_compaction_cleanup(&db_path, true, 2).expect("event cleanup dry run lines");
        assert!(
            lines.iter().any(|line| line.starts_with("EVENTS before:")),
            "missing before summary: {lines:?}"
        );
        assert!(
            lines.iter().any(|line| line.contains("dry-run only")),
            "missing dry-run hint: {lines:?}"
        );

        let _ = fs::remove_dir_all(&home_dir);
    }

}
