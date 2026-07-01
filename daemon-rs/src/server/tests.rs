// SPDX-License-Identifier: MIT

use super::*;
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{HeaderMap, Method, Request, StatusCode};
    use tower::ServiceExt;

    async fn build_state(team_mode: bool) -> RuntimeState {
        build_state_with_budgets(team_mode, None).await
    }

    async fn build_state_with_budgets(team_mode: bool, budgets_toml: Option<&str>) -> RuntimeState {
        let mut home_dir = std::env::temp_dir();
        let mut db_path = std::env::temp_dir();
        let suffix = if team_mode { "team" } else { "solo" };
        home_dir.push(format!(
            "cortex-api-parity-home-{suffix}-{}",
            uuid::Uuid::new_v4()
        ));
        db_path.push(format!(
            "cortex-api-parity-{suffix}-{}.db",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&home_dir).unwrap();
        if let Some(contents) = budgets_toml {
            std::fs::write(home_dir.join("budgets.toml"), contents).unwrap();
        }

        let conn = crate::db::open(&db_path).unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        crate::db::migrate_focus_table(&conn);
        crate::crystallize::migrate_crystal_tables(&conn);
        if team_mode {
            crate::db::create_team_mode_tables(&conn).unwrap();
            let owner_id =
                crate::db::upsert_owner_user(&conn, "owner", Some("Owner"), "argon2id-placeholder")
                    .unwrap();
            crate::db::migrate_to_team_mode(&conn, owner_id).unwrap();
        }
        drop(conn);

        let home_str = home_dir.to_string_lossy().to_string();
        let db_str = db_path.to_string_lossy().to_string();
        let paths = crate::auth::CortexPaths::resolve_with_overrides(
            Some(&home_str),
            Some(&db_str),
            None,
            None,
        );
        let (state, _shutdown_rx) = crate::state::initialize(&paths, false).unwrap();
        let _ = std::fs::remove_file(db_path);
        state
    }

    async fn route_status(
        router: &Router,
        method: Method,
        path: &str,
        body: Option<&str>,
    ) -> StatusCode {
        let mut req = Request::builder().method(method).uri(path);
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let req = req
            .body(Body::from(body.unwrap_or_default().to_string()))
            .unwrap();
        router.clone().oneshot(req).await.unwrap().status()
    }

    fn budget_toml(endpoint: &str, limit: usize) -> String {
        format!(
            r#"
[defaults]
enabled = true

[endpoints.{endpoint}]
limit = {limit}
window_seconds = 60
"#
        )
    }

    fn authed_json_request(token: &str, method: Method, uri: &str, body: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("x-cortex-request", "true")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn send_json(
        router: &Router,
        token: &str,
        method: Method,
        uri: &str,
        body: &str,
    ) -> (StatusCode, HeaderMap, Value) {
        let resp = router
            .clone()
            .oneshot(authed_json_request(token, method, uri, body))
            .await
            .unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let payload = serde_json::from_slice(&body).unwrap();
        (status, headers, payload)
    }

    #[tokio::test]
    async fn test_non_admin_routes_preserved_across_team_migration() {
        let solo_router = build_router(build_state(false).await, 7437);
        let team_router = build_router(build_state(true).await, 7437);

        let cases: Vec<(Method, &str, Option<&str>)> = vec![
            (Method::GET, "/health", None),
            (Method::GET, "/readiness", None),
            (Method::GET, "/digest", None),
            (Method::GET, "/savings", None),
            (Method::GET, "/stats", None),
            (Method::GET, "/dump", None),
            (Method::POST, "/store", Some("{}")),
            (Method::GET, "/recall", None),
            (Method::POST, "/recall", Some("{}")),
            (Method::GET, "/recall/explain", None),
            (Method::GET, "/peek", None),
            (Method::GET, "/unfold", None),
            (Method::GET, "/boot", None),
            (Method::POST, "/diary", Some("{}")),
            (Method::GET, "/recall/budget", None),
            (Method::POST, "/feedback", Some("{}")),
            (Method::GET, "/feedback/stats", None),
            (Method::POST, "/agent-feedback", Some("{}")),
            (Method::GET, "/agent-feedback/stats", None),
            (Method::GET, "/crystals", None),
            (Method::POST, "/crystallize", Some("{}")),
            (Method::POST, "/compact", Some("{}")),
            (Method::POST, "/compact/benchmark", Some("{}")),
            (Method::GET, "/storage", None),
            (Method::POST, "/forget", Some("{}")),
            (Method::POST, "/resolve", Some("{}")),
            (Method::POST, "/conflicts/resolve", Some("{}")),
            (Method::GET, "/conflicts", None),
            (Method::POST, "/archive", Some("{}")),
            (Method::POST, "/focus/start", Some("{}")),
            (Method::POST, "/focus/end", Some("{}")),
            (Method::POST, "/shutdown", Some("{}")),
            (Method::POST, "/lock", Some("{}")),
            (Method::POST, "/unlock", Some("{}")),
            (Method::GET, "/locks", None),
            (Method::POST, "/activity", Some("{}")),
            (Method::GET, "/activity", None),
            (Method::POST, "/message", Some("{}")),
            (Method::GET, "/messages", None),
            (Method::POST, "/session/start", Some("{}")),
            (Method::POST, "/session/heartbeat", Some("{}")),
            (Method::POST, "/session/end", Some("{}")),
            (Method::GET, "/sessions", None),
            (Method::POST, "/tasks", Some("{}")),
            (Method::GET, "/tasks", None),
            (Method::GET, "/tasks/next", None),
            (Method::POST, "/tasks/claim", Some("{}")),
            (Method::POST, "/tasks/complete", Some("{}")),
            (Method::POST, "/tasks/abandon", Some("{}")),
            (Method::POST, "/tasks/delete", Some("{}")),
            (Method::POST, "/feed", Some("{}")),
            (Method::GET, "/feed", None),
            (Method::POST, "/feed/ack", Some("{}")),
            (Method::GET, "/feed/demo", None),
            (Method::GET, "/export", None),
            (Method::POST, "/import", Some("{}")),
            (Method::GET, "/events/stream", None),
            (Method::GET, "/brain/firing", None),
            (Method::POST, "/mcp-rpc", Some("{}")),
        ];

        for (method, path, body) in cases {
            let solo_status = route_status(&solo_router, method.clone(), path, body).await;
            let team_status = route_status(&team_router, method, path, body).await;

            assert_ne!(solo_status, StatusCode::NOT_FOUND, "solo missing {path}");
            assert_ne!(team_status, StatusCode::NOT_FOUND, "team missing {path}");
            assert_eq!(
                solo_status, team_status,
                "status drift for route {path}: solo={solo_status} team={team_status}"
            );
        }
    }

    #[tokio::test]
    async fn admin_permission_routes_are_registered() {
        let solo_router = build_router(build_state(false).await, 7437);
        let team_router = build_router(build_state(true).await, 7437);

        let cases: Vec<(Method, &str, Option<&str>)> = vec![
            (Method::GET, "/permissions", None),
            (Method::POST, "/permissions/grant", Some("{}")),
            (Method::POST, "/permissions/revoke", Some("{}")),
        ];

        for (method, path, body) in cases {
            let solo_status = route_status(&solo_router, method.clone(), path, body).await;
            let team_status = route_status(&team_router, method, path, body).await;

            assert_ne!(solo_status, StatusCode::NOT_FOUND, "solo missing {path}");
            assert_ne!(team_status, StatusCode::NOT_FOUND, "team missing {path}");
        }
    }

    #[tokio::test]
    async fn mcp_rpc_malformed_json_returns_jsonrpc_parse_error() {
        let state = build_state(false).await;
        let token = state.token.as_ref().clone();
        let router = build_router(state, 7437);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/mcp-rpc")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("x-cortex-request", "true")
            .body(Body::from("{\"jsonrpc\":"))
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["error"]["code"], -32700);
        assert_eq!(payload["error"]["message"], "Parse error");
        assert_eq!(payload["id"], Value::Null);
    }

    #[tokio::test]
    async fn mcp_rpc_malformed_json_without_auth_returns_unauthorized() {
        let state = build_state(false).await;
        let router = build_router(state, 7437);

        let req = Request::builder()
            .method(Method::POST)
            .uri("/mcp-rpc")
            .header("content-type", "application/json")
            .header("x-cortex-request", "true")
            .body(Body::from("{\"jsonrpc\":"))
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["error"]["message"], "Unauthorized");
        assert_eq!(payload["id"], Value::Null);
    }

    #[tokio::test]
    async fn mcp_rpc_auth_failures_are_rate_limited_for_remote_callers() {
        let state = build_state(false).await;
        let router = build_router(state, 7437);
        let remote: std::net::SocketAddr = "10.10.10.10:43210".parse().unwrap();

        for _ in 0..10 {
            let req = Request::builder()
                .method(Method::POST)
                .uri("/mcp-rpc")
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-token")
                .header("x-cortex-request", "true")
                .extension(ConnectInfo(remote))
                .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
                .unwrap();

            let resp = router.clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }

        let req = Request::builder()
            .method(Method::POST)
            .uri("/mcp-rpc")
            .header("content-type", "application/json")
            .header("authorization", "Bearer wrong-token")
            .header("x-cortex-request", "true")
            .extension(ConnectInfo(remote))
            .body(Body::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#))
            .unwrap();

        let resp = router.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn handler_panic_response_does_not_expose_panic_payload() {
        let response = handle_handler_panic(Box::new("secret internal detail"));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["error"], "internal server error");
        assert!(!String::from_utf8_lossy(&body).contains("secret internal detail"));
    }

    #[tokio::test]
    async fn store_budget_exhaustion_returns_429_and_skips_decision_write() {
        let state = build_state_with_budgets(false, Some(&budget_toml("store", 1))).await;
        let token = state.token.as_ref().clone();
        let router = build_router(state.clone(), 7437);

        let first = r#"{
            "decision": "Budget governance regression test allows the first store mutation in the configured window.",
            "context": "budget integration test",
            "confidence": 0.95
        }"#;
        let (status, _, _) = send_json(&router, &token, Method::POST, "/store", first).await;
        assert_eq!(status, StatusCode::OK);

        let second = r#"{
            "decision": "Budget governance regression test rejects the second store mutation before writing a row.",
            "context": "budget integration test",
            "confidence": 0.95
        }"#;
        let (status, headers, payload) =
            send_json(&router, &token, Method::POST, "/store", second).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(payload["error"], "budget_exceeded");
        assert_eq!(payload["endpoint"], "store");
        assert_eq!(payload["limit"], 1);
        assert_eq!(payload["window_seconds"], 60);
        assert_eq!(payload["source"], "budgets.toml");
        assert!(headers.get("Retry-After").is_some());

        let conn = state.db.lock().await;
        let decisions: i64 = conn
            .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(decisions, 1);
    }

    #[tokio::test]
    async fn recall_budget_exhaustion_returns_429_and_health_stays_reachable() {
        let state = build_state_with_budgets(false, Some(&budget_toml("recall", 1))).await;
        let token = state.token.as_ref().clone();
        let router = build_router(state.clone(), 7437);

        let (status, _, _) = send_json(&router, &token, Method::GET, "/recall?q=budget", "").await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, payload) =
            send_json(&router, &token, Method::GET, "/recall?q=budget", "").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(payload["error"], "budget_exceeded");
        assert_eq!(payload["endpoint"], "recall");

        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let health: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(health["budgets"]["enabled"], true);
        assert_eq!(health["budgets"]["recentDenials"], 1);
    }

    #[tokio::test]
    async fn boot_budget_exhaustion_returns_429() {
        let state = build_state_with_budgets(false, Some(&budget_toml("boot", 1))).await;
        let token = state.token.as_ref().clone();
        let router = build_router(state, 7437);

        let (status, _, _) = send_json(&router, &token, Method::GET, "/boot?agent=test", "").await;
        assert_eq!(status, StatusCode::OK);

        let (status, _, payload) =
            send_json(&router, &token, Method::GET, "/boot?agent=test", "").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(payload["error"], "budget_exceeded");
        assert_eq!(payload["endpoint"], "boot");
    }

    #[tokio::test]
    async fn mcp_budget_exhaustion_returns_jsonrpc_error_data() {
        let state = build_state_with_budgets(false, Some(&budget_toml("mcp", 1))).await;
        let token = state.token.as_ref().clone();
        let router = build_router(state, 7437);
        let body = r#"{
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "cortex_health", "arguments": {} }
        }"#;

        let (status, _, first) = send_json(&router, &token, Method::POST, "/mcp-rpc", body).await;
        assert_eq!(status, StatusCode::OK);
        assert!(first.get("result").is_some());

        let (status, _, denied) = send_json(&router, &token, Method::POST, "/mcp-rpc", body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(denied["jsonrpc"], "2.0");
        assert_eq!(denied["id"], 7);
        assert_eq!(denied["error"]["code"], -32029);
        assert_eq!(denied["error"]["message"], "budget_exceeded");
        assert_eq!(denied["error"]["data"]["endpoint"], "mcp");
        assert_eq!(denied["error"]["data"]["limit"], 1);
        assert_eq!(denied["error"]["data"]["window_seconds"], 60);
        assert!(
            denied["error"]["data"]["retry_after_seconds"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
    }

    #[test]
    fn local_bind_detection_is_strict() {
        assert!(is_local_bind_addr("127.0.0.1"));
        assert!(is_local_bind_addr("localhost"));
        assert!(is_local_bind_addr("::1"));
        assert!(!is_local_bind_addr("0.0.0.0"));
        assert!(!is_local_bind_addr("100.84.247.96"));
    }

    #[test]
    fn plain_http_policy_rejects_team_mode_and_non_local_binds() {
        assert_eq!(
            plain_http_rejection_reason("127.0.0.1", true, false),
            Some(PlainHttpRejectionReason::TeamMode)
        );
        assert_eq!(
            plain_http_rejection_reason("0.0.0.0", false, false),
            Some(PlainHttpRejectionReason::NonLocalBind)
        );
        assert_eq!(
            plain_http_rejection_reason("100.84.247.96", false, false),
            Some(PlainHttpRejectionReason::NonLocalBind)
        );
        assert_eq!(plain_http_rejection_reason("127.0.0.1", false, false), None);
        assert_eq!(plain_http_rejection_reason("0.0.0.0", false, true), None);
    }

    #[tokio::test]
    async fn plain_http_policy_uses_socket_activation_bind_address() {
        let local_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        assert_eq!(
            effective_bind_addr_for_policy("0.0.0.0", Some(&local_listener)),
            "127.0.0.1"
        );

        let wildcard_listener = tokio::net::TcpListener::bind(("0.0.0.0", 0)).await.unwrap();
        assert_eq!(
            plain_http_rejection_reason(
                &effective_bind_addr_for_policy("127.0.0.1", Some(&wildcard_listener)),
                false,
                false,
            ),
            Some(PlainHttpRejectionReason::NonLocalBind)
        );
    }

    #[cfg(unix)]
    #[test]
    fn socket_activation_fd_validation_rejects_closed_fd() {
        let err = validate_socket_activation_fd(-1).unwrap_err();
        assert!(err.contains("not open"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn socket_activation_fd_validation_rejects_regular_file() {
        use std::os::fd::AsRawFd;

        let file = tempfile::tempfile().unwrap();
        let err = validate_socket_activation_fd(file.as_raw_fd()).unwrap_err();
        assert!(err.contains("not a socket"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn socket_activation_fd_validation_accepts_tcp_listener() {
        use std::os::fd::AsRawFd;

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        validate_socket_activation_fd(listener.as_raw_fd()).unwrap();
    }

