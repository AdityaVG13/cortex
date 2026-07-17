mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_guard() -> tokio::sync::MutexGuard<'static, ()> {
        crate::test_env::lock()
    }

    const V041_SCHEMA_SQL: &str = r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS memories (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          text TEXT NOT NULL,
          source TEXT,
          type TEXT DEFAULT 'memory',
          tags TEXT,
          source_agent TEXT DEFAULT 'unknown',
          confidence REAL DEFAULT 0.8,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS decisions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          decision TEXT NOT NULL,
          context TEXT,
          type TEXT DEFAULT 'decision',
          source_agent TEXT DEFAULT 'unknown',
          confidence REAL DEFAULT 0.8,
          surprise REAL DEFAULT 1.0,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          parent_id INTEGER,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS embeddings (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          target_type TEXT NOT NULL,
          target_id INTEGER NOT NULL,
          vector BLOB NOT NULL,
          model TEXT DEFAULT 'nomic-embed-text',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS events (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          type TEXT NOT NULL,
          data TEXT,
          source_agent TEXT,
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS co_occurrence (
          source_a TEXT NOT NULL,
          source_b TEXT NOT NULL,
          count INTEGER DEFAULT 1,
          last_seen TEXT DEFAULT (datetime('now')),
          PRIMARY KEY (source_a, source_b)
        );

        CREATE TABLE IF NOT EXISTS locks (
          id TEXT PRIMARY KEY,
          path TEXT NOT NULL UNIQUE,
          agent TEXT NOT NULL,
          locked_at TEXT NOT NULL,
          expires_at TEXT
        );

        CREATE TABLE IF NOT EXISTS activities (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          description TEXT NOT NULL,
          files_json TEXT NOT NULL DEFAULT '[]',
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages (
          id TEXT PRIMARY KEY,
          sender TEXT NOT NULL,
          recipient TEXT NOT NULL,
          message TEXT NOT NULL,
          timestamp TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
          agent TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          description TEXT,
          started_at TEXT NOT NULL,
          last_heartbeat TEXT NOT NULL,
          expires_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tasks (
          task_id TEXT PRIMARY KEY,
          title TEXT NOT NULL,
          description TEXT,
          project TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          priority TEXT NOT NULL DEFAULT 'medium',
          required_capability TEXT NOT NULL DEFAULT 'any',
          status TEXT NOT NULL DEFAULT 'pending',
          claimed_by TEXT,
          created_at TEXT NOT NULL,
          claimed_at TEXT,
          completed_at TEXT,
          summary TEXT
        );

        CREATE TABLE IF NOT EXISTS feed (
          id TEXT PRIMARY KEY,
          agent TEXT NOT NULL,
          kind TEXT NOT NULL,
          summary TEXT NOT NULL,
          content TEXT,
          files_json TEXT NOT NULL DEFAULT '[]',
          task_id TEXT,
          trace_id TEXT,
          priority TEXT NOT NULL DEFAULT 'normal',
          timestamp TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS feed_acks (
          agent TEXT PRIMARY KEY,
          last_seen_id TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a);
        CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b);
        CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status);
        CREATE INDEX IF NOT EXISTS idx_memories_source_status ON memories(source, status);
        CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status);
        CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_events_type_created ON events(type, created_at);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);

        CREATE TABLE IF NOT EXISTS context_cache (
          cache_key TEXT PRIMARY KEY,
          content_hash TEXT NOT NULL,
          compressed TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0,
          created_at TEXT DEFAULT (datetime('now')),
          hits INTEGER DEFAULT 0
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
          text, source, tags,
          content=memories,
          content_rowid=id,
          tokenize='trigram'
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
          decision, context,
          content=decisions,
          content_rowid=id,
          tokenize='trigram'
        );

        CREATE TABLE IF NOT EXISTS recall_feedback (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          query_text TEXT NOT NULL,
          query_embedding BLOB,
          result_source TEXT NOT NULL,
          result_type TEXT NOT NULL DEFAULT 'unknown',
          result_id INTEGER,
          signal REAL NOT NULL DEFAULT 1.0,
          agent TEXT NOT NULL DEFAULT 'unknown',
          created_at TEXT DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_feedback_result ON recall_feedback(result_source);
        CREATE INDEX IF NOT EXISTS idx_feedback_created ON recall_feedback(created_at);

        CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_ai AFTER INSERT ON decisions BEGIN
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_ad AFTER DELETE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
        END;

        CREATE TRIGGER IF NOT EXISTS decisions_fts_au AFTER UPDATE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;
    "#;

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_state_{name}_{unique}"))
    }

    #[test]
    fn initialize_fails_when_rotated_token_cannot_be_persisted() {
        let root_dir = temp_test_dir("token_home_is_file");
        fs::create_dir_all(&root_dir).unwrap();
        let home_dir = root_dir.join("home-is-file");
        fs::write(&home_dir, "not a directory").unwrap();
        let db_path = root_dir.join("cortex.db");

        let home_str = home_dir.to_string_lossy().to_string();
        let db_str = db_path.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            Some(&db_str),
            Some(54968),
            Some("127.0.0.1"),
        );

        match initialize(&paths, true) {
            Ok(_) => panic!("startup should fail when rotated token cannot be persisted"),
            Err(err) => assert!(
                err.contains("Failed to generate shared auth token"),
                "unexpected error: {err}"
            ),
        }
        assert!(!paths.token.exists());

        let _ = fs::remove_dir_all(&root_dir);
    }

    fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
        let pragma = format!("PRAGMA table_info({table})");
        let mut stmt = match conn.prepare(&pragma) {
            Ok(stmt) => stmt,
            Err(_) => return false,
        };
        let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
            Ok(rows) => rows,
            Err(_) => return false,
        };
        for name in rows.flatten() {
            if name == column {
                return true;
            }
        }
        false
    }

    fn seed_v041_fixture(db_path: &Path) {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let conn = crate::db::open(db_path).unwrap();
        crate::db::configure(&conn).unwrap();
        conn.execute_batch(V041_SCHEMA_SQL).unwrap();
        conn.execute(
            "INSERT INTO memories (text, source, type, tags, source_agent, confidence, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active')",
            params![
                "legacy memory payload",
                "legacy::memory",
                "memory",
                "alpha,beta",
                "legacy-agent",
                0.72_f64
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, type, source_agent, confidence, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
            params![
                "legacy decision payload",
                "legacy context payload",
                "decision",
                "legacy-agent",
                0.91_f64
            ],
        )
        .unwrap();
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }

    #[test]
    fn initialize_and_migrate_v041_fixture_to_latest_schema() {
        let home_dir = temp_test_dir("upgrade_v041");
        fs::create_dir_all(&home_dir).unwrap();
        let home_str = home_dir.to_string_lossy().to_string();
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(54968),
            Some("127.0.0.1"),
        );
        seed_v041_fixture(&paths.db);

        let (state, shutdown_rx) =
            initialize(&paths, true).expect("current daemon boot should accept a v0.4.1 database");
        assert!(
            paths.token.exists(),
            "boot should materialize a shared auth token"
        );
        drop(shutdown_rx);
        drop(state);

        let conn = crate::db::open(&paths.db).unwrap();
        crate::db::configure(&conn).unwrap();

        let applied = crate::db::run_pending_migrations(&conn);
        assert_eq!(
            applied,
            crate::db::migration_definitions().len(),
            "legacy fixtures should apply every tracked migration exactly once"
        );
        assert!(crate::db::verify_integrity(&conn).unwrap());
        assert_eq!(
            crate::db::current_schema_user_version(&conn).unwrap(),
            crate::db::latest_schema_user_version()
        );

        let memory_text: String = conn
            .query_row(
                "SELECT text FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(memory_text, "legacy memory payload");

        let decision_text: String = conn
            .query_row(
                "SELECT decision FROM decisions WHERE context = 'legacy context payload'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decision_text, "legacy decision payload");

        let merged_count: i64 = conn
            .query_row(
                "SELECT merged_count FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(merged_count, 0);

        let quality: i64 = conn
            .query_row(
                "SELECT quality FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(quality, 50);

        let source_client: String = conn
            .query_row(
                "SELECT source_client FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_client, "unknown");

        let reasoning_depth: String = conn
            .query_row(
                "SELECT reasoning_depth FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reasoning_depth, "single-shot");

        let trust_score: f64 = conn
            .query_row(
                "SELECT trust_score FROM memories WHERE source = 'legacy::memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trust_score, 0.8_f64);

        assert!(crate::db::table_exists(&conn, "schema_migrations"));
        assert!(crate::db::table_exists(&conn, "focus_sessions"));
        assert!(crate::db::table_exists(&conn, "memory_clusters"));
        assert!(crate::db::table_exists(&conn, "cluster_members"));
        assert!(crate::db::table_exists(&conn, "client_permissions"));
        assert!(crate::db::table_exists(&conn, "decision_conflicts"));
        assert!(crate::db::table_exists(&conn, "agent_feedback"));

        assert!(table_has_column(&conn, "memories", "merged_count"));
        assert!(table_has_column(&conn, "memories", "quality"));
        assert!(table_has_column(&conn, "memories", "expires_at"));
        assert!(table_has_column(&conn, "memories", "source_client"));
        assert!(table_has_column(&conn, "memories", "source_model"));
        assert!(table_has_column(&conn, "memories", "reasoning_depth"));
        assert!(table_has_column(&conn, "memories", "trust_score"));
        assert!(table_has_column(&conn, "decisions", "merged_count"));
        assert!(table_has_column(&conn, "decisions", "quality"));
        assert!(table_has_column(&conn, "decisions", "expires_at"));
        assert!(table_has_column(&conn, "decisions", "source_client"));
        assert!(table_has_column(&conn, "decisions", "source_model"));
        assert!(table_has_column(&conn, "decisions", "reasoning_depth"));
        assert!(table_has_column(&conn, "decisions", "trust_score"));

        let migration_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            migration_count as usize,
            crate::db::migration_definitions().len() + 1,
            "boot should add the FTS seed marker before tracked migrations run"
        );
        let tokenizer_migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = '012'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tokenizer_migration_count, 1);

        let fts_seeded_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 'fts_seeded_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_seeded_count, 1);

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

        let memory_fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories_fts", [], |row| row.get(0))
            .unwrap();
        let decision_fts_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(memory_fts_rows, 1);
        assert_eq!(decision_fts_rows, 1);

        drop(conn);
        let _ = fs::remove_dir_all(&home_dir);
    }

    #[test]
    fn sqlite_vec_canary_config_defaults_to_primary_route() {
        let _guard = env_guard();
        let prev_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT").ok();
        let prev_force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF").ok();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");

        let config = SqliteVecCanaryConfig::from_env();
        assert_eq!(config.trial_percent, 0);
        assert!(!config.force_off);
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert!(matches!(
            config.effective_route_mode(),
            SqliteVecRouteMode::Primary
        ));

        if let Some(value) = prev_percent {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        }
        if let Some(value) = prev_force_off {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        }
        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn sqlite_vec_canary_config_parses_and_clamps_env_values() {
        let _guard = env_guard();
        let prev_percent = std::env::var("CORTEX_SQLITE_VEC_TRIAL_PERCENT").ok();
        let prev_force_off = std::env::var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF").ok();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", "145");
        std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", "yes");
        std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", "primary");

        let config = SqliteVecCanaryConfig::from_env();
        assert_eq!(config.trial_percent, 100);
        assert!(config.force_off);
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert!(matches!(
            config.effective_route_mode(),
            SqliteVecRouteMode::Baseline
        ));

        if let Some(value) = prev_percent {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_PERCENT");
        }
        if let Some(value) = prev_force_off {
            std::env::set_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_TRIAL_FORCE_OFF");
        }
        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn sqlite_vec_canary_config_route_mode_aliases_are_supported() {
        let _guard = env_guard();
        let prev_route = std::env::var("CORTEX_SQLITE_VEC_ROUTE").ok();
        std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", "vec0");

        let config = SqliteVecCanaryConfig::from_env();
        assert!(matches!(config.route_mode, SqliteVecRouteMode::Primary));
        assert_eq!(config.route_mode.as_str(), "primary");

        if let Some(value) = prev_route {
            std::env::set_var("CORTEX_SQLITE_VEC_ROUTE", value);
        } else {
            std::env::remove_var("CORTEX_SQLITE_VEC_ROUTE");
        }
    }

    #[test]
    fn derive_read_pool_size_uses_cpu_hint_when_env_unset() {
        assert_eq!(derive_read_pool_size(None, Some(2)), READ_POOL_DEFAULT_MIN);
        assert_eq!(derive_read_pool_size(None, Some(64)), READ_POOL_DEFAULT_MAX);
    }

    #[test]
    fn derive_read_pool_size_clamps_configured_values() {
        assert_eq!(derive_read_pool_size(Some(0), Some(8)), READ_POOL_HARD_MIN);
        assert_eq!(derive_read_pool_size(Some(1), Some(8)), READ_POOL_HARD_MIN);
        assert_eq!(derive_read_pool_size(Some(7), Some(8)), 7);
        assert_eq!(
            derive_read_pool_size(Some(READ_POOL_HARD_MAX + 100), Some(8)),
            READ_POOL_HARD_MAX
        );
    }

    #[test]
    fn read_connection_pool_rotates_connections() {
        let conn_a = Connection::open_in_memory().unwrap();
        conn_a.execute_batch("PRAGMA user_version = 101;").unwrap();
        let conn_b = Connection::open_in_memory().unwrap();
        conn_b.execute_batch("PRAGMA user_version = 202;").unwrap();
        let pool = ReadConnectionPool::new(vec![conn_a, conn_b]);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime");
        let seen_versions = rt.block_on(async {
            let mut versions = Vec::new();
            for _ in 0..4 {
                let conn = pool.lock().await;
                let version: i64 = conn
                    .query_row("PRAGMA user_version", [], |row| row.get(0))
                    .unwrap();
                versions.push(version);
            }
            versions
        });

        assert_eq!(seen_versions, vec![101, 202, 101, 202]);
    }
}
