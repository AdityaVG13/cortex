// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use axum::body::to_bytes;
    use axum::http::{HeaderValue, StatusCode};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::run_pending_migrations(&conn);
        conn
    }

    fn test_state(team_mode: bool) -> RuntimeState {
        let write_conn = test_conn();
        let read_conn = test_conn();
        let (events, _) = broadcast::channel(8);
        let (brain_firing, _) = broadcast::channel(8);
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
            team_mode,
            default_owner_id: None,
            team_api_key_hashes: Arc::new(std::sync::RwLock::new(Vec::new())),
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
        }
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        headers.insert("x-cortex-request", HeaderValue::from_static("desktop"));
        headers
    }

    fn insert_disputed_pair(conn: &Connection) -> (i64, i64) {
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'claude', 'claude', 0.72, 0.74, 'active')",
            params!["Always use SQLite for local dev", "DB policy"],
        )
        .unwrap();
        let first = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'codex', 'codex', 0.91, 0.95, 'active')",
            params!["Use PostgreSQL for production workloads", "DB policy"],
        )
        .unwrap();
        let second = conn.last_insert_rowid();

        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            params![second, first],
        )
        .unwrap();
        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            params![first, second],
        )
        .unwrap();

        (first, second)
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[test]
    fn conflict_list_reports_open_and_resolved_with_metadata() {
        let mut conn = test_conn();
        let (first, second) = insert_disputed_pair(&conn);

        let open_payload = list_conflicts_payload(&conn, &ConflictListOptions::default()).unwrap();
        assert_eq!(open_payload["openCount"].as_u64(), Some(1));
        assert_eq!(open_payload["count"].as_u64(), Some(1));
        assert_eq!(open_payload["pairs"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            open_payload["conflicts"][0]["classification"].as_str(),
            Some("CONTRADICTS")
        );

        let resolution = resolve_decision_with_metadata(
            &mut conn,
            second,
            "keep",
            Some(first),
            ResolutionMetadata {
                conflict_id: Some(conflict_id_from_pair(first, second)),
                classification: Some("CONTRADICTS".to_string()),
                notes: Some("Prefer higher trust score".to_string()),
                resolved_by: Some("codex".to_string()),
                similarity: Some(0.67),
            },
        )
        .unwrap();

        assert_eq!(resolution["resolved"].as_bool(), Some(true));
        assert_eq!(resolution["winnerId"].as_i64(), Some(second));
        assert_eq!(resolution["supersededId"].as_i64(), Some(first));

        let resolved_payload = list_conflicts_payload(
            &conn,
            &ConflictListOptions {
                status: ConflictStatusFilter::Resolved,
                classification: Some("CONTRADICTS".to_string()),
                conflict_id: Some(conflict_id_from_pair(first, second)),
                limit: 100,
            },
        )
        .unwrap();
        assert_eq!(resolved_payload["resolvedCount"].as_u64(), Some(1));
        assert_eq!(
            resolved_payload["conflicts"][0]["resolution"]["resolvedBy"].as_str(),
            Some("codex")
        );
        assert_eq!(
            resolved_payload["conflicts"][0]["resolution"]["notes"].as_str(),
            Some("Prefer higher trust score")
        );
    }

    #[tokio::test]
    async fn conflicts_endpoint_requires_admin_in_team_mode() {
        let state = test_state(true);
        let response = handle_conflicts(
            State(state),
            auth_headers("test-token"),
            Query(ConflictListQuery::default()),
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Admin endpoints require team mode")
        );
    }

    #[test]
    fn permission_grant_list_and_revoke_round_trip() {
        let conn = test_conn();
        grant_permission(&conn, 0, "codex", "admin", "*", "control-center").unwrap();
        grant_permission(
            &conn,
            0,
            "claude",
            "read",
            "cortex_recall",
            "control-center",
        )
        .unwrap();

        let grants = list_permissions(&conn, 0).unwrap();
        assert_eq!(grants.len(), 2);
        assert_eq!(grants[0]["client"].as_str(), Some("claude"));
        assert_eq!(grants[1]["client"].as_str(), Some("codex"));

        let deleted = revoke_permission(&conn, 0, "claude", "read", "cortex_recall").unwrap();
        assert_eq!(deleted, 1);

        let grants = list_permissions(&conn, 0).unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["client"].as_str(), Some("codex"));
    }

    #[tokio::test]
    async fn permissions_endpoint_requires_admin_in_team_mode() {
        let state = test_state(true);
        let response = handle_permissions_list(State(state), auth_headers("test-token")).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Admin endpoints require team mode")
        );
    }

    #[test]
    fn conflict_filter_rejects_invalid_classification() {
        let err = ConflictListOptions::from_query(ConflictListQuery {
            status: Some("open".to_string()),
            classification: Some("contradictory".to_string()),
            conflict_id: None,
            limit: Some(20),
        })
        .expect_err("invalid classification should be rejected");
        assert!(err.contains("Invalid classification filter"));
    }

    #[tokio::test]
    async fn permissions_grant_rejects_invalid_client_shape() {
        let state = test_state(false);
        let response = handle_permissions_grant(
            State(state),
            auth_headers("test-token"),
            Json(PermissionGrantRequest {
                client: Some("!!!".to_string()),
                permission: Some("read".to_string()),
                scope: Some("*".to_string()),
                granted_by: Some("control-center".to_string()),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let payload = response_json(response).await;
        assert_eq!(
            payload["error"].as_str(),
            Some("Invalid client id. Use letters, numbers, '-', '_'.")
        );
    }

