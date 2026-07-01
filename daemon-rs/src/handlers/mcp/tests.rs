// SPDX-License-Identifier: MIT
use super::{fetch_last_call, handle_mcp_message_with_caller, has_client_permission, mcp_dispatch, mcp_tools, normalize_permission_client_id, parse_client_permission, required_permission_for_tool, ClientPermission};
use crate::db; use crate::handlers::recall::RecallContext; use crate::handlers::SourceIdentity;
use crate::state::{DaemonEvent, RuntimeState}; use serde_json::{json, Value};
use std::collections::HashMap; use std::path::PathBuf; use std::sync::atomic::{AtomicBool, AtomicU64}; use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

    use super::{
        fetch_last_call, handle_mcp_message_with_caller, has_client_permission, mcp_dispatch,
        mcp_tools, normalize_permission_client_id, required_permission_for_tool, ClientPermission,
    };
    use crate::db;
    use crate::handlers::recall::RecallContext;
    use crate::handlers::SourceIdentity;
    use crate::state::{DaemonEvent, RuntimeState};
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use tokio::sync::{broadcast, Mutex};

    fn test_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::configure(&conn).unwrap();
        db::initialize_schema(&conn).unwrap();
        db::run_pending_migrations(&conn);
        conn
    }

    fn test_state() -> RuntimeState {
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
            team_mode: false,
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

    async fn recv_session_event(receiver: &mut broadcast::Receiver<DaemonEvent>) -> DaemonEvent {
        for _ in 0..8 {
            let event = receiver.recv().await.unwrap();
            if event.event_type == "session" {
                return event;
            }
        }
        panic!("expected session event");
    }

    async fn seed_disputed_pair(state: &RuntimeState) -> (i64, i64) {
        let conn = state.db.lock().await;
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'claude', 'claude', 0.71, 0.73, 'active')",
            rusqlite::params!["Use sqlite for local projects", "storage policy"],
        )
        .unwrap();
        let first = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, source_client, confidence, trust_score, status)
             VALUES (?1, ?2, 'codex', 'codex', 0.93, 0.95, 'active')",
            rusqlite::params!["Use postgres for production workloads", "storage policy"],
        )
        .unwrap();
        let second = conn.last_insert_rowid();

        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            rusqlite::params![second, first],
        )
        .unwrap();
        conn.execute(
            "UPDATE decisions SET status = 'disputed', disputes_id = ?1 WHERE id = ?2",
            rusqlite::params![first, second],
        )
        .unwrap();
        (first, second)
    }

    #[test]
    fn fetch_last_call_supports_solo_schema_without_owner_columns() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, source_agent, status, created_at)
             VALUES (?1, ?2, ?3, 'active', datetime('now'))",
            rusqlite::params!["semantic recall added", "thread focus", "codex"],
        )
        .unwrap();

        let payload =
            fetch_last_call(&conn, Some("decision"), None, &RecallContext::solo()).unwrap();

        assert_eq!(payload["found"].as_bool(), Some(true));
        assert_eq!(payload["kind"].as_str(), Some("decision"));
        assert_eq!(payload["sourceAgent"].as_str(), Some("codex"));
        assert_eq!(
            payload["detail"]["decision"].as_str(),
            Some("semantic recall added")
        );
    }

    #[test]
    fn normalize_permission_client_id_strips_model_suffix_and_symbols() {
        assert_eq!(normalize_permission_client_id("Codex (gpt-5.4)"), "codex");
        assert_eq!(
            normalize_permission_client_id("  Claude Code / Desktop  "),
            "claudecodedesktop"
        );
        assert_eq!(normalize_permission_client_id(""), "mcp");
    }

    #[test]
    fn parse_client_permission_accepts_known_values() {
        assert_eq!(
            super::parse_client_permission("read"),
            Some(ClientPermission::Read)
        );
        assert_eq!(
            super::parse_client_permission("WRITE"),
            Some(ClientPermission::Write)
        );
        assert_eq!(
            super::parse_client_permission(" admin "),
            Some(ClientPermission::Admin)
        );
        assert_eq!(super::parse_client_permission("owner"), None);
    }

    #[test]
    fn client_permission_allows_by_default_when_no_policy_rows_exist() {
        let conn = test_conn();
        let allowed =
            has_client_permission(&conn, 0, "codex", "cortex_store", ClientPermission::Write)
                .unwrap();
        assert!(
            allowed,
            "empty policy table should preserve legacy permissive mode"
        );
    }

    #[test]
    fn client_permission_enforces_explicit_policy_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
             VALUES (0, 'claude', 'read', '*', 'test')",
            [],
        )
        .unwrap();

        let claude_read =
            has_client_permission(&conn, 0, "claude", "cortex_recall", ClientPermission::Read)
                .unwrap();
        let claude_write =
            has_client_permission(&conn, 0, "claude", "cortex_store", ClientPermission::Write)
                .unwrap();
        let codex_read =
            has_client_permission(&conn, 0, "codex", "cortex_recall", ClientPermission::Read)
                .unwrap();

        assert!(claude_read);
        assert!(!claude_write);
        assert!(!codex_read);
    }

    #[test]
    fn client_permission_supports_wildcard_grants() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
             VALUES (42, '*', 'write', 'cortex_store', 'test')",
            [],
        )
        .unwrap();

        let allowed =
            has_client_permission(&conn, 42, "gemini", "cortex_store", ClientPermission::Write)
                .unwrap();
        let denied_admin = has_client_permission(
            &conn,
            42,
            "gemini",
            "cortex_forget",
            ClientPermission::Admin,
        )
        .unwrap();

        assert!(allowed);
        assert!(!denied_admin);
    }

    #[test]
    fn conflict_tools_require_admin_permission_scope() {
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_list"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_get"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_conflicts_resolve"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_consensus_promote"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_memory_decay_run"),
            Some(ClientPermission::Admin)
        );
        assert_eq!(
            required_permission_for_tool("cortex_eval_run"),
            Some(ClientPermission::Admin)
        );
    }

    #[test]
    fn recall_explain_tool_requires_read_permission_scope() {
        assert_eq!(
            required_permission_for_tool("cortex_recall_policy_explain"),
            Some(ClientPermission::Read)
        );
        assert_eq!(
            required_permission_for_tool("cortex_boot_audit"),
            Some(ClientPermission::Read)
        );
    }

    #[test]
    fn agent_feedback_tools_require_expected_permission_scopes() {
        assert_eq!(
            required_permission_for_tool("cortex_agent_feedback_record"),
            Some(ClientPermission::Write)
        );
        assert_eq!(
            required_permission_for_tool("cortex_agent_feedback_stats"),
            Some(ClientPermission::Read)
        );
    }

    #[tokio::test]
    async fn conflict_list_denies_non_admin_client_permission() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: None,
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'read', '*', 'test')",
                [],
            )
            .unwrap();
        }

        let result = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_list",
            &json!({"status": "open"}),
            Some(&source),
        )
        .await;

        let err = result.expect_err("list should require admin permission");
        assert!(
            err.contains("Permission denied"),
            "expected permission denied error, got: {err}"
        );
    }

    #[tokio::test]
    async fn team_mode_admin_mcp_tool_denies_member_even_without_policy_rows() {
        let mut state = test_state();
        state.team_mode = true;
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: None,
        };

        let member_id = {
            let conn = state.db.lock().await;
            db::create_team_mode_tables(&conn).unwrap();
            conn.execute(
                "INSERT INTO users (username, api_key_hash, role)
                 VALUES ('member-user', 'argon2id-placeholder', 'member')",
                [],
            )
            .unwrap();
            conn.last_insert_rowid()
        };

        let result = mcp_dispatch(
            &state,
            Some(member_id),
            "cortex_permissions_list",
            &json!({}),
            Some(&source),
        )
        .await;

        let err = result.expect_err("member callers must not inherit admin MCP access");
        assert!(
            err.contains("team admin role required"),
            "expected team admin role denial, got: {err}"
        );
    }

    #[tokio::test]
    async fn team_mode_admin_mcp_tool_preserves_owner_empty_policy_compatibility() {
        let mut state = test_state();
        state.team_mode = true;
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: None,
        };

        let owner_id = {
            let conn = state.db.lock().await;
            db::create_team_mode_tables(&conn).unwrap();
            conn.execute(
                "INSERT INTO users (username, api_key_hash, role)
                 VALUES ('owner-user', 'argon2id-placeholder', 'owner')",
                [],
            )
            .unwrap();
            conn.last_insert_rowid()
        };

        let payload = mcp_dispatch(
            &state,
            Some(owner_id),
            "cortex_permissions_list",
            &json!({}),
            Some(&source),
        )
        .await
        .expect("owner callers should preserve empty-policy admin compatibility");

        assert_eq!(payload["ownerId"].as_i64(), Some(owner_id));
        assert_eq!(payload["count"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn conflict_tools_list_and_resolve_success_path() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'admin', '*', 'test')",
                [],
            )
            .unwrap();
        }
        let (first, second) = seed_disputed_pair(&state).await;
        let conflict_id = format!("decision:{}:{}", first.min(second), first.max(second));

        let listed = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_list",
            &json!({"status": "open", "conflictId": conflict_id}),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(listed["count"].as_u64(), Some(1));
        assert_eq!(listed["conflicts"][0]["status"].as_str(), Some("open"));
        assert_eq!(
            listed["conflicts"][0]["classification"].as_str(),
            Some("CONTRADICTS")
        );

        let resolved = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_resolve",
            &json!({
                "conflictId": conflict_id,
                "winnerId": second,
                "action": "keep",
                "classification": "CONTRADICTS",
                "notes": "codex winner",
                "similarity": 0.62
            }),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(resolved["resolved"].as_bool(), Some(true));
        assert_eq!(resolved["winnerId"].as_i64(), Some(second));
        assert_eq!(resolved["supersededId"].as_i64(), Some(first));

        let fetched = mcp_dispatch(
            &state,
            None,
            "cortex_conflicts_get",
            &json!({"conflictId": format!("decision:{}:{}", first.min(second), first.max(second))}),
            Some(&source),
        )
        .await
        .unwrap();
        assert_eq!(fetched["found"].as_bool(), Some(true));
        assert_eq!(fetched["conflict"]["status"].as_str(), Some("resolved"));
        assert_eq!(
            fetched["conflict"]["resolution"]["notes"].as_str(),
            Some("codex winner")
        );
    }

    #[tokio::test]
    async fn cortex_boot_emits_session_started_event() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        let booted = mcp_dispatch(
            &state,
            None,
            "cortex_boot",
            &json!({"budget": 0}),
            Some(&source),
        )
        .await
        .unwrap();
        assert!(booted.get("bootPrompt").is_some());

        let session_event = recv_session_event(&mut events).await;
        assert_eq!(session_event.data["action"].as_str(), Some("started"));
        assert_eq!(
            session_event.data["agent"].as_str(),
            Some("codex (gpt-5.4)")
        );

        let conn = state.db.lock().await;
        let description: String = conn
            .query_row(
                "SELECT description FROM sessions WHERE agent = ?1",
                rusqlite::params!["codex (gpt-5.4)"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(description, "MCP boot session · gpt-5.4");
    }

    #[tokio::test]
    async fn cortex_boot_audit_tool_returns_mcp_boot_rows() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        mcp_dispatch(
            &state,
            None,
            "cortex_boot",
            &json!({"budget": 0}),
            Some(&source),
        )
        .await
        .unwrap();

        let payload = mcp_dispatch(
            &state,
            None,
            "cortex_boot_audit",
            &json!({"agent": "codex (gpt-5.4)", "limit": 10}),
            Some(&source),
        )
        .await
        .unwrap();

        assert_eq!(payload["count"].as_u64(), Some(1));
        assert_eq!(
            payload["audits"][0]["agent"].as_str(),
            Some("codex (gpt-5.4)")
        );
        assert_eq!(payload["audits"][0]["budget_tokens"].as_i64(), Some(0));
        assert!(payload["retention_days"].as_i64().unwrap_or_default() > 0);
    }

    #[test]
    fn tools_list_includes_cortex_boot_audit_schema() {
        let tools = mcp_tools();
        let tool = tools
            .iter()
            .find(|tool| tool["name"].as_str() == Some("cortex_boot_audit"))
            .expect("cortex_boot_audit should be advertised");
        assert_eq!(
            tool["inputSchema"]["properties"]["limit"]["description"].as_str(),
            Some("Maximum rows to return (default 50, max 500).")
        );
    }

    #[test]
    fn documented_mcp_tool_names_match_tools_list_surface() {
        let documented = documented_mcp_tool_names();
        let advertised = sorted_mcp_tool_names();

        assert_eq!(
            documented, advertised,
            "Info/mcp-tools.md must list exactly the tools advertised by MCP tools/list"
        );

        let docs = include_str!("../../../Info/mcp-tools.md");
        let headline = docs
            .lines()
            .find(|line| line.starts_with("> All "))
            .expect("MCP tool reference should declare the advertised tool count");
        assert!(
            headline.contains(&format!("All {} tools", advertised.len())),
            "MCP tool count headline is stale: {headline}"
        );

        let readme = include_str!("../../../README.md");
        let readme_row = readme
            .lines()
            .find(|line| line.contains("[MCP Tools](Info/mcp-tools.md)"))
            .expect("README should link to the MCP tool reference");
        assert!(
            readme_row.contains(&format!("All {} MCP tool", advertised.len())),
            "README MCP tool count is stale: {readme_row}"
        );
    }

    fn sorted_mcp_tool_names() -> Vec<String> {
        let mut names = mcp_tools()
            .into_iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_string))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn documented_mcp_tool_names() -> Vec<String> {
        let mut names = include_str!("../../../Info/mcp-tools.md")
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("| `"))
            .filter_map(|rest| {
                if rest.starts_with("cortex_") {
                    rest.split('`').next().map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        names.sort();
        let original_len = names.len();
        names.dedup();
        assert_eq!(
            original_len,
            names.len(),
            "Info/mcp-tools.md contains duplicate MCP tool rows"
        );
        names
    }

    #[tokio::test]
    async fn initialize_advertises_mcp_resource_discovery() {
        let state = test_state();
        let response = handle_mcp_message_with_caller(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize"
            }),
            None,
            None,
        )
        .await
        .expect("initialize should return a response");

        assert_eq!(
            response["result"]["capabilities"]["resources"]["listChanged"].as_bool(),
            Some(true)
        );
    }

    #[tokio::test]
    async fn resources_list_advertises_tooling_discovery_resources() {
        let state = test_state();
        let response = handle_mcp_message_with_caller(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/list"
            }),
            None,
            None,
        )
        .await
        .expect("resources/list should return a response");

        let resources = response["result"]["resources"]
            .as_array()
            .expect("resources should be an array");
        assert!(resources
            .iter()
            .any(|resource| { resource["uri"].as_str() == Some("cortex://tooling/capabilities") }));
        assert!(resources
            .iter()
            .all(|resource| resource["mimeType"].as_str() == Some("application/json")));
    }

    #[tokio::test]
    async fn resources_read_returns_clustered_tooling_capabilities() {
        let state = test_state();
        let response = handle_mcp_message_with_caller(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/read",
                "params": { "uri": "cortex://tooling/capabilities" }
            }),
            None,
            None,
        )
        .await
        .expect("resources/read should return a response");

        let text = response["result"]["contents"][0]["text"]
            .as_str()
            .expect("resource content should be text JSON");
        let payload: Value =
            serde_json::from_str(text).expect("resource text should parse as JSON");
        assert_eq!(
            payload["toolCount"].as_u64(),
            Some(mcp_tools().len() as u64)
        );
        assert!(payload["clusters"]["recall"]
            .as_array()
            .expect("recall cluster should be listed")
            .iter()
            .any(|tool| tool.as_str() == Some("cortex_recall")));
        assert_eq!(
            payload["resources"][0].as_str(),
            Some("cortex://tooling/capabilities")
        );
    }

    #[tokio::test]
    async fn unknown_tool_error_includes_recovery_data() {
        let state = test_state();
        let response = handle_mcp_message_with_caller(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "recall" }
            }),
            None,
            None,
        )
        .await
        .expect("unknown tool should return an error response");

        assert_eq!(response["error"]["code"].as_i64(), Some(-32601));
        assert_eq!(
            response["error"]["data"]["errorType"].as_str(),
            Some("UNKNOWN_TOOL")
        );
        assert!(response["error"]["data"]["suggestions"]
            .as_array()
            .expect("suggestions should be listed")
            .iter()
            .any(|tool| tool.as_str() == Some("cortex_recall")));
        assert!(response["error"]["data"]["discoveryHint"]
            .as_str()
            .unwrap_or_default()
            .contains("cortex://tooling/tools"));
    }

    #[tokio::test]
    async fn unknown_resource_error_lists_available_resources() {
        let state = test_state();
        let response = handle_mcp_message_with_caller(
            &state,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "resources/read",
                "params": { "uri": "cortex://missing" }
            }),
            None,
            None,
        )
        .await
        .expect("unknown resource should return an error response");

        assert_eq!(response["error"]["code"].as_i64(), Some(-32602));
        assert_eq!(
            response["error"]["data"]["errorType"].as_str(),
            Some("UNKNOWN_RESOURCE")
        );
        assert!(response["error"]["data"]["availableResources"]
            .as_array()
            .expect("available resources should be listed")
            .iter()
            .any(|uri| uri.as_str() == Some("cortex://tooling/tools")));
    }

    #[tokio::test]
    async fn tools_call_includes_token_usage_line_for_cortex_tools() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };
        let calls = vec![
            ("cortex_boot", json!({"budget": 0})),
            ("cortex_boot_audit", json!({"limit": 5})),
            (
                "cortex_recall",
                json!({"query": "daemon lock lease", "budget": 180}),
            ),
            (
                "cortex_peek",
                json!({"query": "daemon lock lease", "limit": 5}),
            ),
            (
                "cortex_semantic_recall",
                json!({"query": "daemon lock lease", "budget": 180, "k": 5}),
            ),
            ("cortex_health", json!({})),
            ("cortex_digest", json!({})),
            (
                "cortex_store",
                json!({"decision": "daemon lock ttl is 30s", "context": "runtime"}),
            ),
            ("cortex_reconnect", json!({})),
            ("cortex_focus_start", json!({"label": "token-usage-test"})),
            ("cortex_focus_end", json!({"label": "token-usage-test"})),
            ("cortex_focus_status", json!({})),
            ("cortex_permissions_list", json!({})),
            ("cortex_lastCall", json!({"kind": "decision"})),
            ("cortex_unfold", json!({"sources": ["memory::missing"]})),
        ];

        for (tool_name, arguments) in calls {
            let msg = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": arguments
                }
            });
            let response = handle_mcp_message_with_caller(&state, &msg, None, Some(&source))
                .await
                .expect("tools/call should return a response");
            let text_payload = response["result"]["content"][0]["text"]
                .as_str()
                .expect("tools/call should return text content");
            let parsed: Value =
                serde_json::from_str(text_payload).expect("text payload should be JSON");
            assert!(
                parsed
                    .get("tokenUsageLine")
                    .and_then(|value| value.as_str())
                    .map(|line| !line.trim().is_empty())
                    .unwrap_or(false),
                "{tool_name} tools/call payload should include tokenUsageLine, got: {parsed}"
            );
            assert!(
                parsed
                    .get("tokenUsage")
                    .and_then(|value| value.get("used"))
                    .and_then(|value| value.as_u64())
                    .is_some(),
                "{tool_name} tools/call payload should include tokenUsage.used, got: {parsed}"
            );
            if tool_name == "cortex_boot" {
                assert_eq!(
                    parsed
                        .get("tokenUsage")
                        .and_then(|value| value.get("budget"))
                        .and_then(|value| value.as_u64()),
                    Some(0),
                    "cortex_boot should propagate tokenUsage.budget from arguments"
                );
            }
        }
    }

    #[tokio::test]
    async fn cortex_health_tool_redacts_private_runtime_details() {
        let state = test_state();
        let payload = mcp_dispatch(&state, None, "cortex_health", &json!({}), None)
            .await
            .expect("cortex_health should return a payload");

        let runtime = payload["runtime"]
            .as_object()
            .expect("health payload should include runtime object");
        for field in [
            "db_path",
            "token_path",
            "pid_path",
            "ipc_endpoint",
            "ipc_kind",
            "executable",
            "owner",
        ] {
            assert!(
                !runtime.contains_key(field),
                "cortex_health MCP tool should redact runtime.{field}"
            );
        }

        let stats = payload["stats"]
            .as_object()
            .expect("health payload should include stats object");
        assert!(
            !stats.contains_key("home"),
            "cortex_health MCP tool should redact stats.home"
        );
    }

    #[tokio::test]
    async fn cortex_store_rejects_invalid_explicit_ttl() {
        let state = test_state();
        for (ttl_seconds, expected) in [
            (0, "ttl_seconds must be > 0"),
            (-1, "ttl_seconds must be > 0"),
            (31_536_001, "ttl_seconds must be <= 31536000 (365 days)"),
        ] {
            let err = mcp_dispatch(
                &state,
                None,
                "cortex_store",
                &json!({
                    "decision": "temporary decision with enough detail to persist through mcp ttl validation",
                    "ttl_seconds": ttl_seconds
                }),
                None,
            )
            .await
            .expect_err("invalid cortex_store TTL should fail");

            assert!(
                err.contains(expected),
                "ttl {ttl_seconds} should return {expected:?}, got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn read_path_tools_recreate_mcp_presence_when_missing() {
        let cases = vec![
            ("cortex_peek", json!({"query": "sqlite"})),
            ("cortex_recall", json!({"query": "sqlite"})),
            ("cortex_recall_policy_explain", json!({"query": "sqlite"})),
            ("cortex_semantic_recall", json!({"query": "sqlite"})),
            ("cortex_unfold", json!({"sources": ["memory::missing"]})),
        ];

        for (tool_name, args) in cases {
            let state = test_state();
            let mut events = state.events.subscribe();
            let source = SourceIdentity {
                agent: "codex".to_string(),
                model: Some("gpt-5.4".to_string()),
            };

            let payload = mcp_dispatch(&state, None, tool_name, &args, Some(&source))
                .await
                .unwrap();
            assert!(
                payload.is_object(),
                "{tool_name} should return a JSON payload"
            );

            let session_event = recv_session_event(&mut events).await;
            assert_eq!(session_event.data["action"].as_str(), Some("started"));
            assert_eq!(
                session_event.data["agent"].as_str(),
                Some("codex (gpt-5.4)")
            );

            let conn = state.db.lock().await;
            let description: String = conn
                .query_row(
                    "SELECT description FROM sessions WHERE agent = ?1",
                    rusqlite::params!["codex (gpt-5.4)"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(description, "MCP active session · gpt-5.4");
        }
    }

    #[tokio::test]
    async fn recall_presence_refresh_preserves_boot_description_without_new_session_event() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        mcp_dispatch(
            &state,
            None,
            "cortex_boot",
            &json!({"budget": 0}),
            Some(&source),
        )
        .await
        .unwrap();

        while events.try_recv().is_ok() {}

        mcp_dispatch(
            &state,
            None,
            "cortex_recall",
            &json!({"query": "sqlite", "agent": "codex"}),
            Some(&source),
        )
        .await
        .unwrap();

        let conn = state.db.lock().await;
        let description: String = conn
            .query_row(
                "SELECT description FROM sessions WHERE agent = ?1",
                rusqlite::params!["codex (gpt-5.4)"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(description, "MCP boot session · gpt-5.4");
        drop(conn);

        let drained: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.event_type)
            .collect();
        assert!(
            !drained.iter().any(|event_type| event_type == "session"),
            "existing sessions should not emit a new session event on recall refresh: {drained:?}"
        );
    }

    #[tokio::test]
    async fn reconnect_preserves_boot_description_for_existing_session() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        mcp_dispatch(
            &state,
            None,
            "cortex_boot",
            &json!({"budget": 0}),
            Some(&source),
        )
        .await
        .unwrap();

        mcp_dispatch(
            &state,
            None,
            "cortex_reconnect",
            &json!({"agent": "codex"}),
            Some(&source),
        )
        .await
        .unwrap();

        let conn = state.db.lock().await;
        let description: String = conn
            .query_row(
                "SELECT description FROM sessions WHERE agent = ?1",
                rusqlite::params!["codex (gpt-5.4)"],
                |row| row.get(0),
            )
            .unwrap();
        let session_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(description, "MCP boot session · gpt-5.4");
        assert_eq!(session_count, 1);
    }

    #[tokio::test]
    async fn model_less_read_refresh_reuses_existing_modeled_session() {
        let state = test_state();
        let mut events = state.events.subscribe();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        mcp_dispatch(
            &state,
            None,
            "cortex_boot",
            &json!({"budget": 0}),
            Some(&source),
        )
        .await
        .unwrap();

        while events.try_recv().is_ok() {}

        mcp_dispatch(
            &state,
            None,
            "cortex_recall",
            &json!({"query": "sqlite", "agent": "codex"}),
            None,
        )
        .await
        .unwrap();

        let conn = state.db.lock().await;
        let rows: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT agent, description FROM sessions ORDER BY last_heartbeat DESC")
                .unwrap();
            stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "codex (gpt-5.4)");
        assert_eq!(rows[0].1, "MCP boot session · gpt-5.4");
        drop(conn);

        let drained: Vec<String> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| event.event_type)
            .collect();
        assert!(
            !drained.iter().any(|event_type| event_type == "session"),
            "model-less read refresh should reuse the existing session without a new session event: {drained:?}"
        );
    }

    #[tokio::test]
    async fn consensus_promote_requires_admin_permission_scope() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'read', '*', 'test')",
                [],
            )
            .unwrap();
        }

        let result = mcp_dispatch(
            &state,
            None,
            "cortex_consensus_promote",
            &json!({"limit": 5}),
            Some(&source),
        )
        .await;

        let err = result.expect_err("consensus promote should require admin permission");
        assert!(
            err.contains("Permission denied"),
            "expected permission denied error, got: {err}"
        );
    }

    #[tokio::test]
    async fn consensus_promote_resolves_disputed_pair_when_margin_is_high_enough() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'admin', '*', 'test')",
                [],
            )
            .unwrap();
        }

        let (first, second) = seed_disputed_pair(&state).await;
        let payload = mcp_dispatch(
            &state,
            None,
            "cortex_consensus_promote",
            &json!({"limit": 10, "minMargin": 0.1}),
            Some(&source),
        )
        .await
        .unwrap();

        assert_eq!(payload["promotedCount"].as_u64(), Some(1));
        assert_eq!(payload["failedCount"].as_u64(), Some(0));

        let conn = state.db.lock().await;
        let winner_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                rusqlite::params![second],
                |row| row.get(0),
            )
            .unwrap();
        let superseded_status: String = conn
            .query_row(
                "SELECT status FROM decisions WHERE id = ?1",
                rusqlite::params![first],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(winner_status, "active");
        assert_eq!(superseded_status, "superseded");
    }

    #[tokio::test]
    async fn memory_decay_run_executes_decay_pass_and_reports_counts() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'admin', '*', 'test')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memories (text, type, source, status, score, retrievals, pinned, last_accessed, created_at, updated_at)
                 VALUES (?1, 'note', 'test::decay', 'active', 1.0, 0, 0, datetime('now', '-10 days'), datetime('now'), datetime('now'))",
                rusqlite::params!["decay me"],
            )
            .unwrap();
        }

        let payload = mcp_dispatch(
            &state,
            None,
            "cortex_memory_decay_run",
            &json!({"includeAging": false, "cleanupExpired": false}),
            Some(&source),
        )
        .await
        .unwrap();
        assert!(payload["ok"].as_bool().unwrap_or(false));
        assert!(payload["decayed"].is_number());
        assert_eq!(payload["aging"]["ran"].as_bool(), Some(false));
        assert_eq!(payload["expiredCleanup"]["ran"].as_bool(), Some(false));

        let conn = state.db.lock().await;
        let score: f64 = conn
            .query_row(
                "SELECT score FROM memories WHERE source = 'test::decay' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            score <= 1.0,
            "decay pass should not increase score unexpectedly, got {score}"
        );
    }

    #[tokio::test]
    async fn eval_run_returns_windowed_metrics_snapshot() {
        let state = test_state();
        let source = SourceIdentity {
            agent: "codex".to_string(),
            model: Some("gpt-5.4".to_string()),
        };

        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by)
                 VALUES (0, 'codex', 'admin', '*', 'test')",
                [],
            )
            .unwrap();
        }

        let _ = seed_disputed_pair(&state).await;
        let payload = mcp_dispatch(
            &state,
            None,
            "cortex_eval_run",
            &json!({"horizonDays": 14}),
            Some(&source),
        )
        .await
        .unwrap();

        assert!(payload["ok"].as_bool().unwrap_or(false));
        assert_eq!(payload["windowDays"].as_i64(), Some(14));
        assert!(payload["totals"]["openConflicts"].as_i64().unwrap_or(0) >= 1);
        assert!(payload["signals"]["conflictBurden"].is_number());
        assert!(payload["signals"]["decayBurden"].is_number());
        assert!(payload["signals"]["resolutionVelocity"].is_number());
    }

