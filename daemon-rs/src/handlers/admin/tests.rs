mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::extract::State;
    use axum::http::{HeaderValue, StatusCode};
    use rusqlite::params;
    use rusqlite::Connection;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open sqlite memory DB");
        crate::db::configure(&conn).expect("configure sqlite");
        crate::db::initialize_schema(&conn).expect("initialize schema");
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn auth_headers(api_key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {api_key}")).expect("valid bearer token header"),
        );
        headers.insert("x-cortex-request", HeaderValue::from_static("true"));
        headers
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("parse response json")
    }

    fn test_state_with_admin() -> (RuntimeState, String, i64) {
        let write_conn = test_conn();
        let read_conn = test_conn();
        let (events, _) = broadcast::channel(8);
        let (brain_firing, _) = broadcast::channel(8);
        let admin_api_key = crate::auth::generate_ctx_api_key();
        let admin_hash =
            crate::auth::hash_api_key_argon2id(&admin_api_key).expect("hash admin API key");
        crate::db::create_team_mode_tables(&write_conn).expect("create team-mode tables");
        let admin_user_id = crate::db::upsert_owner_user(
            &write_conn,
            "owner-admin",
            Some("Owner Admin"),
            &admin_hash,
        )
        .expect("upsert admin owner");
        crate::db::migrate_to_team_mode(&write_conn, admin_user_id)
            .expect("migrate write DB to team mode");
        crate::db::create_team_mode_tables(&read_conn).expect("create team-mode tables (read DB)");
        let _ = crate::db::upsert_owner_user(
            &read_conn,
            "owner-admin",
            Some("Owner Admin"),
            &admin_hash,
        )
        .expect("upsert owner in read DB");
        crate::db::migrate_to_team_mode(&read_conn, admin_user_id)
            .expect("migrate read DB to team mode");

        (
            RuntimeState {
                db: Arc::new(Mutex::new(write_conn)),
                db_read: Arc::new(Mutex::new(read_conn)),
                token: Arc::new("test-token".to_string()),
                events,
                brain_firing,
                mcp_calls: Arc::new(AtomicU64::new(0)),
                mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
                recall_history: Arc::new(Mutex::new(HashMap::new())),
                pre_cache: Arc::new(Mutex::new(HashMap::new())),
                served_content: Arc::new(Mutex::new(HashMap::new())),
                shutdown_tx: Arc::new(Mutex::new(None)),
                home: PathBuf::from("."),
                db_path: PathBuf::from(":memory:"),
                token_path: PathBuf::from("cortex.token"),
                pid_path: PathBuf::from("cortex.pid"),
                port: 7437,
                embedding_engine: None,
                rate_limiter: crate::rate_limit::RateLimiter::new(),
                team_mode: true,
                default_owner_id: Some(admin_user_id),
                team_api_key_hashes: Arc::new(std::sync::RwLock::new(vec![(
                    admin_user_id,
                    admin_hash,
                )])),
                degraded_mode: Arc::new(AtomicBool::new(false)),
                db_corrupted: Arc::new(AtomicBool::new(false)),
                readiness: Arc::new(AtomicBool::new(true)),
                last_activity_unix_secs: Arc::new(AtomicU64::new(0)),
                write_buffer_path: PathBuf::from("write_buffer.jsonl"),
                sqlite_vec_canary: crate::state::SqliteVecCanaryConfig {
                    trial_percent: 0,
                    force_off: false,
                    route_mode: crate::state::SqliteVecRouteMode::Trial,
                },
                rerank_config: crate::rerank::RerankConfig::off(),
                reranker: None,
            },
            admin_api_key,
            admin_user_id,
        )
    }

    #[tokio::test]
    async fn handle_unowned_reports_ownerless_rows() {
        let (state, admin_api_key, owner_id) = test_state_with_admin();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, NULL, 'private', 'note', 'active', 1.0)",
                params!["ownerless memory", "tests::admin"],
            )
            .expect("insert ownerless memory");
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, ?3, 'private', 'note', 'active', 1.0)",
                params!["owned memory", "tests::admin", owner_id],
            )
            .expect("insert owned memory");
            conn.execute(
                "INSERT INTO decisions (decision, context, source_agent, owner_id, visibility, status, score, merged_count, quality)
                 VALUES (?1, ?2, 'tester', NULL, 'private', 'active', 1.0, 0, 70)",
                params!["ownerless decision", "tests::admin"],
            )
            .expect("insert ownerless decision");
        }

        let response = handle_unowned(State(state), auth_headers(&admin_api_key)).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["unowned"]["memories"], 1);
        assert_eq!(body["unowned"]["decisions"], 1);
    }

    #[tokio::test]
    async fn handle_assign_owner_moves_target_rows_between_users() {
        let (state, admin_api_key, _) = test_state_with_admin();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, 'member')",
                params![
                    "from-user",
                    "From User",
                    crate::auth::hash_api_key_argon2id(&crate::auth::generate_ctx_api_key())
                        .expect("hash from-user key")
                ],
            )
            .expect("insert from user");
            let from_user_id = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, 'member')",
                params![
                    "to-user",
                    "To User",
                    crate::auth::hash_api_key_argon2id(&crate::auth::generate_ctx_api_key())
                        .expect("hash to-user key")
                ],
            )
            .expect("insert to user");
            let to_user_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, ?3, 'private', 'note', 'active', 1.0)",
                params!["owned by from", "tests::admin", from_user_id],
            )
            .expect("insert from-owner memory");
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, ?3, 'private', 'note', 'active', 1.0)",
                params!["already to", "tests::admin", to_user_id],
            )
            .expect("insert to-owner memory");
        }

        let response = handle_assign_owner(
            State(state.clone()),
            auth_headers(&admin_api_key),
            Json(AssignOwnerBody {
                from_user: Some("from-user".to_string()),
                to_user: "to-user".to_string(),
                table: Some("memories".to_string()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["assigned"]["memories"], 1);

        let conn = state.db.lock().await;
        let reassigned_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories m
                 JOIN users u ON u.id = m.owner_id
                 WHERE m.text = 'owned by from' AND u.username = 'to-user'",
                [],
                |row| row.get(0),
            )
            .expect("count reassigned memory rows");
        assert_eq!(reassigned_count, 1);
    }

    #[tokio::test]
    async fn handle_set_visibility_updates_allowed_tables() {
        let (state, admin_api_key, owner_id) = test_state_with_admin();
        let target_ids = {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, ?3, 'private', 'note', 'active', 1.0)",
                params!["visibility-a", "tests::admin", owner_id],
            )
            .expect("insert first memory");
            let first = conn.last_insert_rowid();
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, ?3, 'private', 'note', 'active', 1.0)",
                params!["visibility-b", "tests::admin", owner_id],
            )
            .expect("insert second memory");
            vec![first, conn.last_insert_rowid()]
        };

        let response = handle_set_visibility(
            State(state.clone()),
            auth_headers(&admin_api_key),
            Json(SetVisibilityBody {
                table: "memories".to_string(),
                ids: target_ids.clone(),
                visibility: "team".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["updated"], 2);

        let conn = state.db.lock().await;
        let updated_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE id IN (?1, ?2) AND visibility = 'team'",
                params![target_ids[0], target_ids[1]],
                |row| row.get(0),
            )
            .expect("count updated visibility rows");
        assert_eq!(updated_count, 2);
    }

    #[tokio::test]
    async fn handle_assign_owner_backfills_unowned_rows_across_default_tables() {
        let (state, admin_api_key, _) = test_state_with_admin();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, 'member')",
                params![
                    "to-owner",
                    "To Owner",
                    crate::auth::hash_api_key_argon2id(&crate::auth::generate_ctx_api_key())
                        .expect("hash to-owner key")
                ],
            )
            .expect("insert to-owner user");
            conn.execute(
                "INSERT INTO memories (text, source, owner_id, visibility, type, status, score)
                 VALUES (?1, ?2, NULL, 'private', 'note', 'active', 1.0)",
                params!["ownerless memory", "tests::admin"],
            )
            .expect("insert ownerless memory");
            conn.execute(
                "INSERT INTO decisions (decision, context, source_agent, owner_id, visibility, status, score, merged_count, quality)
                 VALUES (?1, ?2, 'tester', NULL, 'private', 'active', 1.0, 0, 70)",
                params!["ownerless decision", "tests::admin"],
            )
            .expect("insert ownerless decision");
        }

        let response = handle_assign_owner(
            State(state.clone()),
            auth_headers(&admin_api_key),
            Json(AssignOwnerBody {
                from_user: None,
                to_user: "to-owner".to_string(),
                table: None,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["assigned"]["memories"], 1);
        assert_eq!(body["assigned"]["decisions"], 1);

        let conn = state.db.lock().await;
        let memory_owner: Option<String> = conn
            .query_row(
                "SELECT u.username FROM memories m JOIN users u ON u.id = m.owner_id WHERE m.text = 'ownerless memory'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(memory_owner.as_deref(), Some("to-owner"));
        let decision_owner: Option<String> = conn
            .query_row(
                "SELECT u.username FROM decisions d JOIN users u ON u.id = d.owner_id WHERE d.decision = 'ownerless decision'",
                [],
                |row| row.get(0),
            )
            .ok();
        assert_eq!(decision_owner.as_deref(), Some("to-owner"));
    }

    #[tokio::test]
    async fn handle_assign_owner_rejects_non_allowlisted_table() {
        let (state, admin_api_key, _) = test_state_with_admin();
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, 'member')",
                params![
                    "to-owner",
                    "To Owner",
                    crate::auth::hash_api_key_argon2id(&crate::auth::generate_ctx_api_key())
                        .expect("hash to-owner key")
                ],
            )
            .expect("insert to-owner user");
        }

        let response = handle_assign_owner(
            State(state),
            auth_headers(&admin_api_key),
            Json(AssignOwnerBody {
                from_user: None,
                to_user: "to-owner".to_string(),
                table: Some("users".to_string()),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"], "table not in allowlist");
    }

    #[tokio::test]
    async fn handle_set_visibility_returns_zero_for_empty_target_set() {
        let (state, admin_api_key, _owner_id) = test_state_with_admin();
        let response = handle_set_visibility(
            State(state),
            auth_headers(&admin_api_key),
            Json(SetVisibilityBody {
                table: "memories".to_string(),
                ids: vec![],
                visibility: "shared".to_string(),
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["updated"], 0);
    }
}
