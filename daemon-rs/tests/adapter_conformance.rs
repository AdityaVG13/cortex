use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod support;
use support::{
    adapter_conformance_guard, read_token, request_json, request_json_with_headers, reserve_port,
    shutdown_daemon, shutdown_daemon_best_effort, spawn_daemon, unique_temp_dir, wait_for_exit,
    wait_for_health, JsonHttpResponse, SpawnTrackedExt, STARTUP_TIMEOUT, HEALTH_POLL_INTERVAL,
};
const SPEC: &str = include_str!("../../specs/cortex-adapter-contract.yaml");
const COVERAGE_REPORT: &str = include_str!("../../specs/cortex-adapter-contract/COVERAGE.md");
const DISCREPANCIES: &str = include_str!("../../specs/cortex-adapter-contract/DISCREPANCIES.md");

#[test]
fn adapter_contract_spec_covers_required_matrix() {
    let spec: Value = serde_json::from_str(SPEC).expect("contract spec is JSON-compatible YAML");
    assert_eq!(spec["schema"], "cortex.adapter.contract");
    assert_eq!(spec["version"], "0.6.0");
    assert_eq!(
        spec["conformance"]["specificationSource"],
        "specs/cortex-adapter-contract.yaml"
    );
    assert_eq!(
        spec["conformance"]["coverageReport"],
        "specs/cortex-adapter-contract/COVERAGE.md"
    );
    assert_eq!(
        spec["conformance"]["discrepancies"],
        "specs/cortex-adapter-contract/DISCREPANCIES.md"
    );

    let scenarios = spec["scenarios"].as_array().expect("scenarios array");
    assert!(
        scenarios.len() >= 10,
        "adapter contract must keep at least 10 scenarios"
    );

    let ids = contract_scenario_ids(&spec);
    assert_eq!(
        ids.len(),
        scenarios.len(),
        "contract scenario ids must be unique"
    );
    for required in [
        "health-public",
        "store-decision",
        "recall-get",
        "recall-post",
        "peek",
        "boot",
        "export-json",
        "mcp-initialize",
        "mcp-tools-list",
        "mcp-health-tool",
        "mcp-store-tool",
        "mcp-recall-tool",
    ] {
        assert!(
            ids.contains(required),
            "missing contract scenario {required}"
        );
    }

    assert_contract_requirement_metadata(&spec);
    assert_discrepancies_documented(&spec);
    assert_coverage_report_current(&spec);
}

#[test]
fn http_and_mcp_rpc_match_adapter_contract() {
    let _guard = adapter_conformance_guard();
    let spec: Value = serde_json::from_str(SPEC).expect("contract spec is JSON-compatible YAML");
    let mut exercised = BTreeSet::new();
    let home_dir = unique_temp_dir("adapter_conformance");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let health = request_json(port, "GET", "/health", None, None).expect("health");
    assert_contract_response(&spec, "health-public", &health);
    record_scenario(&mut exercised, "health-public");

    let store = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Adapter conformance sentinel memory",
            "context": "C4 contract round trip",
            "type": "decision",
            "source_agent": "adapter-conformance-sdk",
            "source_model": "gpt-5.4",
            "confidence": 0.93,
            "reasoning_depth": "high",
            "ttl_seconds": 3600
        })),
    )
    .expect("store");
    assert_contract_response(&spec, "store-decision", &store);
    assert_eq!(store.body["stored"], true);
    assert!(
        store.body.get("entry").is_some(),
        "store entry missing: {}",
        store.body
    );
    record_scenario(&mut exercised, "store-decision");

    let recall_get = request_json(
        port,
        "GET",
        "/recall?q=Adapter%20conformance%20sentinel%20memory&budget=200&k=5&agent=adapter-conformance-sdk",
        Some(&token),
        None,
    )
    .expect("recall get");
    assert_contract_response(&spec, "recall-get", &recall_get);
    assert!(
        recall_get.body["results"].as_array().is_some(),
        "recall results should be an array: {}",
        recall_get.body
    );
    record_scenario(&mut exercised, "recall-get");

    let recall_post = request_json(
        port,
        "POST",
        "/recall",
        Some(&token),
        Some(json!({
            "q": "Adapter conformance sentinel memory",
            "budget": 200,
            "k": 5,
            "agent": "adapter-conformance-sdk"
        })),
    )
    .expect("recall post");
    assert_contract_response(&spec, "recall-post", &recall_post);
    record_scenario(&mut exercised, "recall-post");

    let peek = request_json(
        port,
        "GET",
        "/peek?q=Adapter%20conformance%20sentinel%20memory&k=5",
        Some(&token),
        None,
    )
    .expect("peek");
    assert_contract_response(&spec, "peek", &peek);
    record_scenario(&mut exercised, "peek");

    let boot = request_json(
        port,
        "GET",
        "/boot?agent=adapter-conformance-sdk&budget=120",
        Some(&token),
        None,
    )
    .expect("boot");
    assert_contract_response(&spec, "boot", &boot);
    record_scenario(&mut exercised, "boot");

    let export =
        request_json(port, "GET", "/export?format=json", Some(&token), None).expect("export");
    assert_contract_response(&spec, "export-json", &export);
    record_scenario(&mut exercised, "export-json");

    let initialize = mcp_rpc(
        port,
        &token,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "adapter-conformance", "version": "0.6.0" }
            }
        }),
    )
    .expect("mcp initialize");
    assert_contract_response(&spec, "mcp-initialize", &initialize);
    record_scenario(&mut exercised, "mcp-initialize");

    let tools = mcp_rpc(
        port,
        &token,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .expect("mcp tools/list");
    assert_contract_status(&spec, "mcp-tools-list", &tools);
    let tool_names: BTreeSet<&str> = tools.body["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_contract_required_tools(&spec, "mcp-tools-list", &tool_names);
    record_scenario(&mut exercised, "mcp-tools-list");

    let mcp_health = mcp_rpc(
        port,
        &token,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "cortex_health", "arguments": {} }
        }),
    )
    .expect("mcp cortex_health");
    assert_contract_status(&spec, "mcp-health-tool", &mcp_health);
    assert_mcp_tool_ok(&mcp_health.body);
    record_scenario(&mut exercised, "mcp-health-tool");

    let mcp_store = mcp_rpc(
        port,
        &token,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "cortex_store",
                "arguments": {
                    "decision": "Adapter conformance MCP memory",
                    "context": "C4 MCP contract"
                }
            }
        }),
    )
    .expect("mcp cortex_store");
    assert_contract_status(&spec, "mcp-store-tool", &mcp_store);
    assert_mcp_tool_ok(&mcp_store.body);
    record_scenario(&mut exercised, "mcp-store-tool");

    let mcp_recall = mcp_rpc(
        port,
        &token,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "cortex_recall",
                "arguments": {
                    "query": "Adapter conformance MCP memory",
                    "budget": 120,
                    "k": 5
                }
            }
        }),
    )
    .expect("mcp cortex_recall");
    assert_contract_status(&spec, "mcp-recall-tool", &mcp_recall);
    assert_mcp_tool_ok(&mcp_recall.body);
    record_scenario(&mut exercised, "mcp-recall-tool");

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
    let _ = fs::remove_dir_all(&home_dir);
    assert_all_contract_scenarios_exercised(&spec, &exercised);
}

#[test]
fn cli_http_and_mcp_golden_summary_matches() {
    let _guard = adapter_conformance_guard();
    let daemon = AdapterDaemon::start("adapter_golden");
    let import_target = AdapterDaemon::start("adapter_golden_import_target");
    let home = daemon.home_arg();
    let token = daemon.token();
    let import_target_token = import_target.token();
    let status = run_status_json(&home, daemon.port);

    let readiness = request_json(daemon.port, "GET", "/readiness", None, None).expect("readiness");
    let health = request_json(daemon.port, "GET", "/health", None, None).expect("health");
    let store = request_json(
        daemon.port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Adapter golden sentinel memory with POST recall sentinel memory and enough specificity for quality gates",
            "context": "Phase 4 golden capture fixture",
            "type": "decision",
            "source_agent": "adapter-golden-sdk",
            "source_model": "gpt-5.4",
            "confidence": 0.91,
            "reasoning_depth": "high",
            "ttl_seconds": 3600
        })),
    )
    .expect("store");
    let recall = request_json(
        daemon.port,
        "GET",
        "/recall?q=Adapter%20golden%20sentinel%20memory&budget=200&k=5&agent=adapter-golden-sdk",
        Some(&token),
        None,
    )
    .expect("recall");
    let recall_post = request_json(
        daemon.port,
        "POST",
        "/recall",
        Some(&token),
        Some(json!({
            "q": "POST recall sentinel memory",
            "budget": 200,
            "k": 5,
            "agent": "adapter-golden-sdk"
        })),
    )
    .expect("recall post");
    let peek = request_json(
        daemon.port,
        "GET",
        "/peek?q=Adapter%20golden%20sentinel%20memory&k=5",
        Some(&token),
        None,
    )
    .expect("peek");
    let boot = request_json(
        daemon.port,
        "GET",
        "/boot?agent=adapter-golden-sdk&budget=120",
        Some(&token),
        None,
    )
    .expect("boot");
    let tools = mcp_rpc(
        daemon.port,
        &token,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    )
    .expect("mcp tools/list");
    let export = request_json(
        daemon.port,
        "GET",
        "/export?format=json&limit=50",
        Some(&token),
        None,
    )
    .expect("export");
    let import = request_json(
        import_target.port,
        "POST",
        "/import",
        Some(&import_target_token),
        Some(export.body.clone()),
    )
    .expect("import");
    let import_recall = request_json(
        import_target.port,
        "GET",
        "/recall?q=Adapter%20golden%20sentinel%20memory&budget=200&k=5&agent=adapter-golden-sdk",
        Some(&import_target_token),
        None,
    )
    .expect("import recall");
    let auth_failures = [
        (
            "http-store-missing-auth",
            "POST",
            "/store",
            Some(json!({ "decision": "unauthenticated golden store should fail" })),
        ),
        (
            "http-export-missing-auth",
            "GET",
            "/export?format=json",
            None,
        ),
        (
            "http-import-missing-auth",
            "POST",
            "/import",
            Some(json!({ "version": 1, "memories": [], "decisions": [] })),
        ),
        (
            "mcp-rpc-missing-auth",
            "POST",
            "/mcp-rpc",
            Some(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/list" })),
        ),
    ]
    .into_iter()
    .map(|(id, method, path, body)| {
        let response = request_json_with_headers(
            daemon.port,
            method,
            path,
            &[("X-Cortex-Request", "true")],
            body,
        )
        .unwrap_or_else(|err| panic!("{id} should return JSON: {err}"));
        response_fixture_summary(
            id,
            method,
            path,
            &response,
            auth_error_body_summary(path, &response.body),
        )
    })
    .collect::<Vec<_>>();
    let malformed_mcp = [
        (
            "mcp-missing-jsonrpc",
            json!({ "id": 101, "method": "tools/list" }),
        ),
        (
            "mcp-wrong-jsonrpc",
            json!({ "jsonrpc": "1.0", "id": 102, "method": "tools/list" }),
        ),
        ("mcp-missing-method", json!({ "jsonrpc": "2.0", "id": 103 })),
        (
            "mcp-non-string-method",
            json!({ "jsonrpc": "2.0", "id": 104, "method": 104 }),
        ),
    ]
    .into_iter()
    .map(|(id, body)| {
        let response = mcp_rpc(daemon.port, &token, body)
            .unwrap_or_else(|err| panic!("{id} should return JSON-RPC error: {err}"));
        response_fixture_summary(
            id,
            "POST",
            "/mcp-rpc",
            &response,
            mcp_error_body_summary(&response.body),
        )
    })
    .collect::<Vec<_>>();

    let summary = json!({
        "schemaVersion": "cortex.adapter-golden-summary.v1",
        "equivalenceTier": "Tier3Logical",
        "fixtures": [
            cli_status_fixture_summary(&status),
            response_fixture_summary(
                "http-readiness-public",
                "GET",
                "/readiness",
                &readiness,
                readiness_body_summary(&readiness.body),
            ),
            response_fixture_summary(
                "http-health-public",
                "GET",
                "/health",
                &health,
                health_body_summary(&health.body),
            ),
            response_fixture_summary(
                "http-store-decision",
                "POST",
                "/store",
                &store,
                store_body_summary(&store.body),
            ),
            response_fixture_summary(
                "http-recall-get",
                "GET",
                "/recall?q=Adapter%20golden%20sentinel%20memory&budget=200&k=5&agent=adapter-golden-sdk",
                &recall,
                recall_body_summary(&recall.body),
            ),
            response_fixture_summary(
                "http-recall-post",
                "POST",
                "/recall",
                &recall_post,
                recall_body_summary(&recall_post.body),
            ),
            response_fixture_summary(
                "http-peek",
                "GET",
                "/peek?q=Adapter%20golden%20sentinel%20memory&k=5",
                &peek,
                peek_body_summary(&peek.body),
            ),
            response_fixture_summary(
                "http-boot",
                "GET",
                "/boot?agent=adapter-golden-sdk&budget=120",
                &boot,
                boot_body_summary(&boot.body),
            ),
            response_fixture_summary(
                "http-export-json",
                "GET",
                "/export?format=json&limit=50",
                &export,
                export_body_summary(&export.body),
            ),
            response_fixture_summary(
                "http-import-json",
                "POST",
                "/import",
                &import,
                import_body_summary(&import.body),
            ),
            response_fixture_summary(
                "http-import-recall-get",
                "GET",
                "/recall?q=Adapter%20golden%20sentinel%20memory&budget=200&k=5&agent=adapter-golden-sdk",
                &import_recall,
                recall_body_summary(&import_recall.body),
            ),
            response_fixture_summary(
                "mcp-tools-list",
                "POST",
                "/mcp-rpc",
                &tools,
                mcp_tools_body_summary(&tools.body),
            ),
        ],
        "negativeFixtures": {
            "missingAuth": auth_failures,
            "malformedMcp": malformed_mcp,
        }
    });

    assert_json_golden("adapter/http_mcp_status_contract_summary", &summary);
}

#[test]
fn http_export_import_roundtrip_preserves_recallable_decisions() {
    let _guard = adapter_conformance_guard();
    let source = AdapterDaemon::start("adapter_export_source");
    let target = AdapterDaemon::start("adapter_import_target");
    let source_token = source.token();
    let target_token = target.token();
    let decision_text = "Adapter Phase 6 export import sentinel keeps recall roundtrip coverage";

    let store = request_json(
        source.port,
        "POST",
        "/store",
        Some(&source_token),
        Some(json!({
            "decision": decision_text,
            "context": "Phase 6 public HTTP export/import conformance fixture",
            "type": "decision",
            "source_agent": "phase6-export-source",
            "source_model": "gpt-5.4",
            "confidence": 0.94,
            "reasoning_depth": "high",
            "ttl_seconds": 3600
        })),
    )
    .expect("store source fixture");
    assert_eq!(store.status, 200);
    assert_eq!(store.body["stored"], true);

    let exported = request_json(
        source.port,
        "GET",
        "/export?format=json&limit=50",
        Some(&source_token),
        None,
    )
    .expect("export source fixture");
    assert_eq!(exported.status, 200);
    assert_eq!(exported.body["version"], 1);
    assert_eq!(exported.body["mode"], "page");
    assert!(
        exported.body["decisions"]
            .as_array()
            .expect("export decisions array")
            .iter()
            .any(|decision| decision["decision"].as_str() == Some(decision_text)),
        "export should include the stored decision: {}",
        exported.body
    );

    let imported = request_json(
        target.port,
        "POST",
        "/import",
        Some(&target_token),
        Some(exported.body.clone()),
    )
    .expect("import exported fixture");
    assert_eq!(imported.status, 200);
    assert!(
        imported.body["imported"]["decisions"].as_u64().unwrap_or(0) >= 1,
        "import should report at least one decision: {}",
        imported.body
    );

    let recalled = request_json(
        target.port,
        "GET",
        "/recall?q=Adapter%20Phase%206%20export%20import%20sentinel&budget=200&k=5",
        Some(&target_token),
        None,
    )
    .expect("recall imported fixture");
    assert_eq!(recalled.status, 200);
    assert!(
        recalled.body.to_string().contains(decision_text),
        "imported decision should be recallable through the public HTTP surface: {}",
        recalled.body
    );
}

#[test]
fn protected_http_surfaces_reject_missing_bearer_token() {
    let _guard = adapter_conformance_guard();
    let daemon = AdapterDaemon::start("adapter_auth_invariants");

    for (case, method, path, body) in [
        (
            "store",
            "POST",
            "/store",
            Some(json!({ "decision": "unauthenticated store should fail" })),
        ),
        ("export", "GET", "/export?format=json", None),
        (
            "import",
            "POST",
            "/import",
            Some(json!({ "version": 1, "memories": [], "decisions": [] })),
        ),
        (
            "mcp-rpc",
            "POST",
            "/mcp-rpc",
            Some(json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/list" })),
        ),
    ] {
        let response = request_json_with_headers(
            daemon.port,
            method,
            path,
            &[("X-Cortex-Request", "true")],
            body,
        )
        .unwrap_or_else(|err| panic!("{case} unauthenticated request should return JSON: {err}"));
        assert_eq!(
            response.status, 401,
            "{case} should reject missing bearer auth"
        );
        if case == "mcp-rpc" {
            assert_eq!(response.body["jsonrpc"], "2.0");
            assert_eq!(response.body["error"]["message"], "Unauthorized");
            assert_eq!(response.body["id"], Value::Null);
        } else {
            assert_eq!(response.body["error"], "Unauthorized");
        }
    }
}

#[test]
fn mcp_jsonrpc_malformed_envelopes_return_invalid_request_errors() {
    let _guard = adapter_conformance_guard();
    let daemon = AdapterDaemon::start("adapter_mcp_malformed");
    let token = daemon.token();

    for (case, body, expected_id, expected_message) in [
        (
            "missing-jsonrpc",
            json!({ "id": 101, "method": "tools/list" }),
            json!(101),
            "Missing JSON-RPC version",
        ),
        (
            "wrong-jsonrpc",
            json!({ "jsonrpc": "1.0", "id": 102, "method": "tools/list" }),
            json!(102),
            "Invalid JSON-RPC version",
        ),
        (
            "missing-method",
            json!({ "jsonrpc": "2.0", "id": 103 }),
            json!(103),
            "Missing JSON-RPC method",
        ),
        (
            "non-string-method",
            json!({ "jsonrpc": "2.0", "id": 104, "method": 104 }),
            json!(104),
            "Missing JSON-RPC method",
        ),
    ] {
        let response = mcp_rpc(daemon.port, &token, body)
            .unwrap_or_else(|err| panic!("{case} request should return JSON-RPC error: {err}"));
        assert_eq!(response.status, 200, "{case} should stay in JSON-RPC");
        assert_eq!(response.body["jsonrpc"], "2.0", "{case}");
        assert_eq!(response.body["id"], expected_id, "{case}");
        assert_eq!(response.body["error"]["code"], -32600, "{case}");
        assert_eq!(
            response.body["error"]["message"], expected_message,
            "{case}"
        );
    }
}


struct AdapterDaemon {
    child: Child,
    home_dir: PathBuf,
    port: u16,
}

impl AdapterDaemon {
    fn start(prefix: &str) -> Self {
        let home_dir = unique_temp_dir(prefix);
        fs::create_dir_all(&home_dir).expect("create temp home");
        let port = reserve_port();
        let home = home_dir.to_string_lossy().to_string();
        let mut child = spawn_daemon(&home, port);
        wait_for_health(port, &mut child);
        Self {
            child,
            home_dir,
            port,
        }
    }

    fn home_arg(&self) -> String {
        self.home_dir.to_string_lossy().to_string()
    }

    fn token(&self) -> String {
        read_token(&self.home_dir)
    }
}

impl Drop for AdapterDaemon {
    fn drop(&mut self) {
        shutdown_daemon_best_effort(self.port, &self.home_dir);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.home_dir);
    }
}


fn mcp_rpc(port: u16, token: &str, body: Value) -> Result<JsonHttpResponse, String> {
    request_json(port, "POST", "/mcp-rpc", Some(token), Some(body))
}

fn run_status_json(home: &str, port: u16) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args([
            "status",
            "--json",
            "--home",
            home,
            "--port",
            &port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .env("CORTEX_DISABLE_IPC", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run cortex status --json");
    assert_output_success(&output);
    assert!(
        output.stderr.is_empty(),
        "status --json should not write stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("status --json output should parse")
}

fn cli_status_fixture_summary(payload: &Value) -> Value {
    let checks = payload["checks"]
        .as_array()
        .expect("status checks array")
        .iter()
        .map(|check| {
            json!({
                "name": check["name"],
                "status": check["status"],
                "repairKind": repair_kind(&check["repair"]),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "id": "cli-status-json-ready",
        "source": "cli",
        "command": "cortex status --json --home [CORTEX_HOME] --port [PORT] --bind 127.0.0.1",
        "exit": 0,
        "body": {
            "schemaVersion": payload["schemaVersion"],
            "status": payload["status"],
            "summary": payload["summary"],
            "version": payload["version"],
            "runtime": {
                "baseUrl": "http://127.0.0.1:[PORT]",
                "bind": payload["runtime"]["bind"],
                "port": "[PORT]",
                "home": "[CORTEX_HOME]",
                "dbPath": "[CORTEX_HOME]/cortex.db",
                "tokenPath": "[CORTEX_HOME]/token",
                "pidPath": "[CORTEX_HOME]/cortex.pid"
            },
            "nextActionKind": payload["nextAction"]["kind"],
            "repairKind": repair_kind(&payload["repair"]),
            "checks": checks
        }
    })
}

fn response_fixture_summary(
    id: &str,
    method: &str,
    path: &str,
    response: &JsonHttpResponse,
    body: Value,
) -> Value {
    json!({
        "id": id,
        "source": "http",
        "method": method,
        "path": path,
        "status": response.status,
        "body": body
    })
}

fn readiness_body_summary(body: &Value) -> Value {
    json!({
        "status": body["status"],
        "ready": body["ready"],
        "runtime": public_runtime_summary(body),
        "statsKeys": object_keys(&body["stats"])
    })
}

fn health_body_summary(body: &Value) -> Value {
    json!({
        "status": body["status"],
        "ready": body["ready"],
        "degraded": body["degraded"],
        "db_corrupted": body["db_corrupted"],
        "embedding_status": body["embedding_status"],
        "team_mode": body["team_mode"],
        "runtime": public_runtime_summary(body),
        "stats": {
            "memories": body["stats"]["memories"],
            "decisions": body["stats"]["decisions"],
            "embeddings": body["stats"]["embeddings"],
            "events": body["stats"]["events"]
        },
        "vector_search": {
            "backend": body["vector_search"]["backend"],
            "embedding_model": {
                "key": body["vector_search"]["embedding_model"]["key"],
                "dimension": body["vector_search"]["embedding_model"]["dimension"],
                "pooling": body["vector_search"]["embedding_model"]["pooling"]
            },
            "routing": body["vector_search"]["routing"],
            "sqlite_vec": {
                "available": body["vector_search"]["sqlite_vec"]["available"],
                "versionKind": value_kind(&body["vector_search"]["sqlite_vec"]["version"]),
                "errorKind": value_kind(&body["vector_search"]["sqlite_vec"]["error"])
            }
        },
        "budgetKeys": object_keys(&body["budgets"])
    })
}

fn store_body_summary(body: &Value) -> Value {
    let entry = &body["entry"];
    json!({
        "stored": body["stored"],
        "entry": {
            "action": entry["action"],
            "idKind": value_kind(&entry["id"]),
            "status": entry["status"],
            "retention_class": entry["retention_class"],
            "surprise": entry["surprise"],
            "quality": entry["quality"]
        }
    })
}

fn recall_body_summary(body: &Value) -> Value {
    let results = body["results"].as_array().expect("recall results array");
    let first = results.first();
    json!({
        "fieldKeys": object_keys(body),
        "budget": body["budget"],
        "spentKind": value_kind(&body["spent"]),
        "savedKind": value_kind(&body["saved"]),
        "tokenUsageLineKind": value_kind(&body["tokenUsageLine"]),
        "resultCount": results.len(),
        "firstResultKeys": first.map(object_keys).unwrap_or_else(|| json!([])),
        "firstResultMentionsFixture": first
            .map(|value| value.to_string().contains("Adapter golden sentinel memory"))
            .unwrap_or(false)
    })
}

fn peek_body_summary(body: &Value) -> Value {
    let matches = body["matches"].as_array().expect("peek matches array");
    let first = matches.first();
    json!({
        "fieldKeys": object_keys(body),
        "count": body["count"],
        "matchCount": matches.len(),
        "firstMatchKeys": first.map(object_keys).unwrap_or_else(|| json!([])),
        "tokenUsageKeys": object_keys(&body["tokenUsage"]),
        "tokenUsageLineKind": value_kind(&body["tokenUsageLine"])
    })
}

fn boot_body_summary(body: &Value) -> Value {
    let prompt = body["bootPrompt"].as_str().expect("bootPrompt string");
    json!({
        "fieldKeys": object_keys(body),
        "bootPrompt": {
            "kind": "string",
            "lineCount": prompt.lines().count(),
            "containsAgentName": prompt.contains("adapter-golden-sdk")
        },
        "tokenEstimateKind": value_kind(&body["tokenEstimate"]),
        "savingsKeys": object_keys(&body["savings"]),
        "tokenUsageKeys": object_keys(&body["tokenUsage"]),
        "tokenUsageLineKind": value_kind(&body["tokenUsageLine"])
    })
}

fn export_body_summary(body: &Value) -> Value {
    let memories = body["memories"].as_array().expect("export memories array");
    let decisions = body["decisions"]
        .as_array()
        .expect("export decisions array");
    json!({
        "fieldKeys": object_keys(body),
        "version": body["version"],
        "mode": body["mode"],
        "limit": body["limit"],
        "memoriesCount": body["memories_count"],
        "decisionsCount": body["decisions_count"],
        "memoriesOffsetKind": value_kind(&body["memories_offset"]),
        "decisionsOffsetKind": value_kind(&body["decisions_offset"]),
        "nextMemoriesOffsetKind": value_kind(&body["next_memories_offset"]),
        "nextDecisionsOffsetKind": value_kind(&body["next_decisions_offset"]),
        "truncated": body["truncated"],
        "memoryCount": memories.len(),
        "decisionCount": decisions.len(),
        "firstDecisionKeys": decisions.first().map(object_keys).unwrap_or_else(|| json!([])),
        "containsGoldenDecision": decisions
            .iter()
            .any(|decision| decision["decision"].as_str().is_some_and(|value| {
                value.contains("Adapter golden sentinel memory")
            }))
    })
}

fn import_body_summary(body: &Value) -> Value {
    json!({
        "fieldKeys": object_keys(body),
        "importedKeys": object_keys(&body["imported"]),
        "memoriesImportedKind": value_kind(&body["imported"]["memories"]),
        "decisionsImportedKind": value_kind(&body["imported"]["decisions"]),
        "decisionsImportedAtLeastOne": body["imported"]["decisions"].as_u64().unwrap_or(0) >= 1
    })
}

fn auth_error_body_summary(path: &str, body: &Value) -> Value {
    if path == "/mcp-rpc" {
        json!({
            "jsonrpc": body["jsonrpc"],
            "id": body["id"],
            "errorKeys": object_keys(&body["error"]),
            "errorCode": body["error"]["code"],
            "errorMessage": body["error"]["message"]
        })
    } else {
        json!({
            "fieldKeys": object_keys(body),
            "error": body["error"]
        })
    }
}

fn mcp_error_body_summary(body: &Value) -> Value {
    json!({
        "jsonrpc": body["jsonrpc"],
        "id": body["id"],
        "errorKeys": object_keys(&body["error"]),
        "errorCode": body["error"]["code"],
        "errorMessage": body["error"]["message"]
    })
}

fn mcp_tools_body_summary(body: &Value) -> Value {
    let tools = body["result"]["tools"].as_array().expect("MCP tools array");
    let mut names = tools
        .iter()
        .map(|tool| {
            tool["name"]
                .as_str()
                .expect("MCP tool name string")
                .to_string()
        })
        .collect::<Vec<_>>();
    names.sort();
    json!({
        "jsonrpc": body["jsonrpc"],
        "id": body["id"],
        "resultKeys": object_keys(&body["result"]),
        "toolCount": names.len(),
        "toolNames": names,
        "allToolsHaveDescription": tools.iter().all(|tool| tool["description"].is_string()),
        "allToolsHaveInputSchema": tools.iter().all(|tool| tool["inputSchema"].is_object())
    })
}

fn public_runtime_summary(body: &Value) -> Value {
    let runtime = &body["runtime"];
    let private_fields_present = [
        "db_path",
        "token_path",
        "pid_path",
        "ipc_endpoint",
        "ipc_kind",
        "executable",
        "owner",
    ]
    .iter()
    .any(|key| runtime.get(*key).is_some());
    json!({
        "version": runtime["version"],
        "mode": runtime["mode"],
        "port": "[PORT]",
        "privateFieldsPresent": private_fields_present
    })
}

fn repair_kind(value: &Value) -> Value {
    match value {
        Value::Object(map) => map.get("kind").cloned().unwrap_or(Value::Null),
        Value::Null => json!("none"),
        _ => Value::Null,
    }
}

fn object_keys(value: &Value) -> Value {
    let Some(map) = value.as_object() else {
        return json!([]);
    };
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    json!(keys)
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn assert_output_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed with status {}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_json_golden(name: &str, actual: &Value) {
    let actual = canonical_json(actual);
    let golden_path = golden_path(name);

    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(golden_path.parent().expect("golden parent")).unwrap_or_else(|err| {
            panic!(
                "failed to create golden directory {}: {err}",
                golden_path.display()
            )
        });
        fs::write(&golden_path, actual).unwrap_or_else(|err| {
            panic!("failed to update golden {}: {err}", golden_path.display())
        });
        return;
    }

    let expected = fs::read_to_string(&golden_path).unwrap_or_else(|err| {
        panic!(
            "golden file missing: {}\nerror: {err}\nrun with UPDATE_GOLDENS=1 cargo test --test adapter_conformance, then review git diff daemon-rs/tests/golden/",
            golden_path.display()
        )
    });
    if actual != expected {
        let actual_path = golden_path.with_extension("actual");
        fs::write(&actual_path, &actual).unwrap_or_else(|err| {
            panic!(
                "failed to write actual output {}: {err}",
                actual_path.display()
            )
        });
        panic!(
            "GOLDEN MISMATCH: {name}\n{}\nactual output written to {}\nreview with: git diff --no-index {} {}",
            unified_diff(&expected, &actual),
            actual_path.display(),
            golden_path.display(),
            actual_path.display()
        );
    }
}

fn golden_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut parts = name.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            path.push(part);
        } else {
            path.push(format!("{part}.golden"));
        }
    }
    path
}

fn canonical_json(value: &Value) -> String {
    let sorted = sort_json(value.clone());
    let mut text = serde_json::to_string_pretty(&sorted).expect("golden JSON should serialize");
    text.push('\n');
    text
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sort_json).collect()),
        Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            let mut sorted = Map::new();
            for (key, value) in entries {
                sorted.insert(key, sort_json(value));
            }
            Value::Object(sorted)
        }
        other => other,
    }
}

fn unified_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let max_len = expected_lines.len().max(actual_lines.len());
    let first_diff = (0..max_len)
        .find(|&idx| expected_lines.get(idx) != actual_lines.get(idx))
        .unwrap_or(0);
    let start = first_diff.saturating_sub(3);
    let end = (first_diff + 4).min(max_len);
    let mut diff = String::from("--- expected\n+++ actual\n");

    for idx in start..end {
        match (expected_lines.get(idx), actual_lines.get(idx)) {
            (Some(expected), Some(actual)) if expected == actual => {
                diff.push_str(&format!(" {:>4} {expected}\n", idx + 1));
            }
            (Some(expected), Some(actual)) => {
                diff.push_str(&format!("-{:>4} {expected}\n", idx + 1));
                diff.push_str(&format!("+{:>4} {actual}\n", idx + 1));
            }
            (Some(expected), None) => {
                diff.push_str(&format!("-{:>4} {expected}\n", idx + 1));
            }
            (None, Some(actual)) => {
                diff.push_str(&format!("+{:>4} {actual}\n", idx + 1));
            }
            (None, None) => {}
        }
    }

    diff
}

fn assert_json_fields(payload: &Value, fields: &[&str]) {
    for field in fields {
        assert!(
            payload.get(*field).is_some(),
            "missing field {field} in payload {payload}"
        );
    }
}

fn contract_scenario<'a>(spec: &'a Value, id: &str) -> &'a Value {
    spec["scenarios"]
        .as_array()
        .expect("scenarios array")
        .iter()
        .find(|scenario| scenario["id"].as_str() == Some(id))
        .unwrap_or_else(|| panic!("missing contract scenario {id}"))
}

fn assert_contract_status(spec: &Value, id: &str, response: &JsonHttpResponse) {
    let expected_status = contract_scenario(spec, id)["expect"]["status"]
        .as_u64()
        .unwrap_or_else(|| panic!("contract scenario {id} missing numeric expect.status"));
    assert_eq!(
        u64::from(response.status),
        expected_status,
        "status mismatch for contract scenario {id}"
    );
}

fn assert_contract_response(spec: &Value, id: &str, response: &JsonHttpResponse) {
    assert_contract_status(spec, id, response);
    if let Some(fields) = contract_scenario(spec, id)["expect"]["jsonFields"].as_array() {
        let fields: Vec<&str> = fields
            .iter()
            .map(|field| {
                field
                    .as_str()
                    .unwrap_or_else(|| panic!("contract scenario {id} has non-string json field"))
            })
            .collect();
        assert_json_fields(&response.body, &fields);
    }
}

fn assert_contract_required_tools(spec: &Value, id: &str, tool_names: &BTreeSet<&str>) {
    let required_tools = contract_scenario(spec, id)["expect"]["requiredTools"]
        .as_array()
        .unwrap_or_else(|| panic!("contract scenario {id} missing expect.requiredTools"));
    for required in required_tools {
        let required = required
            .as_str()
            .unwrap_or_else(|| panic!("contract scenario {id} has non-string required tool"));
        assert!(tool_names.contains(required), "missing MCP tool {required}");
    }
}

fn contract_scenario_ids(spec: &Value) -> BTreeSet<String> {
    spec["scenarios"]
        .as_array()
        .expect("scenarios array")
        .iter()
        .map(|scenario| scenario["id"].as_str().expect("scenario id").to_string())
        .collect()
}

fn record_scenario(exercised: &mut BTreeSet<String>, id: &str) {
    assert!(
        exercised.insert(id.to_string()),
        "contract scenario {id} was exercised more than once"
    );
}

fn assert_all_contract_scenarios_exercised(spec: &Value, exercised: &BTreeSet<String>) {
    let expected = contract_scenario_ids(spec);
    let missing: Vec<&String> = expected.difference(exercised).collect();
    let unexpected: Vec<&String> = exercised.difference(&expected).collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "adapter conformance coverage drift; missing={missing:?}, unexpected={unexpected:?}"
    );
}

#[derive(Default)]
struct CoverageStats {
    must_total: usize,
    should_total: usize,
    tested: usize,
    passing: usize,
    divergent: usize,
}

impl CoverageStats {
    fn record(&mut self, level: &str, status: &str) {
        match level {
            "MUST" => self.must_total += 1,
            "SHOULD" => self.should_total += 1,
            "MAY" => {}
            other => panic!("unsupported requirementLevel {other}"),
        }
        match status {
            "tested" => {
                self.tested += 1;
                self.passing += 1;
            }
            "xfail" => {
                self.tested += 1;
                self.divergent += 1;
            }
            "untested" => {}
            other => panic!("unsupported coverageStatus {other}"),
        }
    }

    fn denominator(&self) -> usize {
        self.must_total + self.should_total
    }

    fn score(&self) -> f64 {
        let denominator = self.denominator();
        if denominator == 0 {
            1.0
        } else {
            self.passing as f64 / denominator as f64
        }
    }
}

fn assert_contract_requirement_metadata(spec: &Value) {
    let threshold = spec["conformance"]["mustCoverageThreshold"]
        .as_f64()
        .expect("conformance.mustCoverageThreshold");
    assert!(
        threshold >= 0.95,
        "MUST coverage threshold must stay at or above 95%"
    );

    let mut must_total = 0usize;
    let mut must_passing = 0usize;
    for scenario in spec["scenarios"].as_array().expect("scenarios array") {
        let id = scenario["id"].as_str().expect("scenario id");
        let section = scenario["specSection"]
            .as_str()
            .unwrap_or_else(|| panic!("scenario {id} missing specSection"));
        assert!(
            !section.trim().is_empty(),
            "scenario {id} has blank specSection"
        );

        let level = scenario["requirementLevel"]
            .as_str()
            .unwrap_or_else(|| panic!("scenario {id} missing requirementLevel"));
        assert!(
            matches!(level, "MUST" | "SHOULD" | "MAY"),
            "scenario {id} has invalid requirementLevel {level}"
        );

        let status = scenario["coverageStatus"]
            .as_str()
            .unwrap_or_else(|| panic!("scenario {id} missing coverageStatus"));
        assert!(
            matches!(status, "tested" | "xfail" | "untested"),
            "scenario {id} has invalid coverageStatus {status}"
        );

        if level == "MUST" {
            must_total += 1;
            if status == "tested" {
                must_passing += 1;
            }
        }
        if status == "xfail" {
            let discrepancy_id = scenario["discrepancyId"]
                .as_str()
                .unwrap_or_else(|| panic!("scenario {id} is xfail without discrepancyId"));
            assert!(
                discrepancy_id.starts_with("DISC-"),
                "scenario {id} uses invalid discrepancyId {discrepancy_id}"
            );
        }
    }

    assert!(
        must_total > 0,
        "contract must enumerate at least one MUST clause"
    );
    let must_score = must_passing as f64 / must_total as f64;
    assert!(
        must_score >= threshold,
        "MUST conformance below threshold: {must_passing}/{must_total} < {threshold:.2}"
    );
}

fn assert_discrepancies_documented(spec: &Value) {
    let discrepancy_ids: BTreeSet<&str> = spec["scenarios"]
        .as_array()
        .expect("scenarios array")
        .iter()
        .filter_map(|scenario| scenario["discrepancyId"].as_str())
        .collect();

    if discrepancy_ids.is_empty() {
        assert!(
            DISCREPANCIES.contains("No accepted divergences"),
            "DISCREPANCIES.md must explicitly state when there are no accepted divergences"
        );
        return;
    }

    for discrepancy_id in discrepancy_ids {
        assert!(
            DISCREPANCIES.contains(discrepancy_id),
            "missing documented conformance discrepancy {discrepancy_id}"
        );
    }
}

fn assert_coverage_report_current(spec: &Value) {
    let expected = generate_contract_coverage_report(spec);
    let report_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("specs")
        .join("cortex-adapter-contract")
        .join("COVERAGE.md");
    if std::env::var_os("UPDATE_CONFORMANCE_COVERAGE").is_some() {
        fs::write(&report_path, &expected).expect("update adapter contract coverage report");
    }
    assert_eq!(
        COVERAGE_REPORT, expected,
        "adapter contract coverage report is stale; run with UPDATE_CONFORMANCE_COVERAGE=1 cargo test --test adapter_conformance, then review specs/cortex-adapter-contract/COVERAGE.md"
    );
}

fn generate_contract_coverage_report(spec: &Value) -> String {
    let schema = spec["schema"].as_str().expect("schema");
    let version = spec["version"].as_str().expect("version");
    let threshold = spec["conformance"]["mustCoverageThreshold"]
        .as_f64()
        .expect("conformance.mustCoverageThreshold");
    let mut by_section: BTreeMap<&str, CoverageStats> = BTreeMap::new();
    let mut total = CoverageStats::default();

    for scenario in spec["scenarios"].as_array().expect("scenarios array") {
        let section = scenario["specSection"].as_str().expect("specSection");
        let level = scenario["requirementLevel"]
            .as_str()
            .expect("requirementLevel");
        let status = scenario["coverageStatus"].as_str().expect("coverageStatus");
        by_section.entry(section).or_default().record(level, status);
        total.record(level, status);
    }

    let mut report = String::new();
    report.push_str("# Cortex Adapter Contract Coverage\n\n");
    report.push_str("Generated from `specs/cortex-adapter-contract.yaml` by `daemon-rs/tests/adapter_conformance.rs`.\n\n");
    report.push_str(&format!(
        "- Specification source: `{schema}` version `{version}`\n"
    ));
    report.push_str(&format!(
        "- MUST coverage threshold: `{:.0}%`\n",
        threshold * 100.0
    ));
    report.push_str("- Fixture provenance: scenarios are inline contract vectors in `specs/cortex-adapter-contract.yaml`; there is no external reference fixture generator.\n\n");
    report.push_str(
        "| Spec Section | MUST Clauses | SHOULD Clauses | Tested | Passing | Divergent | Score |\n",
    );
    report.push_str("| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n");
    for (section, stats) in by_section {
        push_coverage_row(&mut report, section, &stats);
    }
    push_coverage_row(&mut report, "TOTAL", &total);
    report.push_str(
        "\nAccepted divergences are tracked in `specs/cortex-adapter-contract/DISCREPANCIES.md`.\n",
    );
    report
}

fn push_coverage_row(report: &mut String, section: &str, stats: &CoverageStats) {
    report.push_str(&format!(
        "| {section} | {} | {} | {} | {} | {} | {:.1}% |\n",
        stats.must_total,
        stats.should_total,
        stats.tested,
        stats.passing,
        stats.divergent,
        stats.score() * 100.0
    ));
}

fn assert_mcp_tool_ok(payload: &Value) {
    assert!(
        payload.get("error").is_none(),
        "MCP tool returned JSON-RPC error: {payload}"
    );
    assert_eq!(payload["jsonrpc"], "2.0");
    assert!(
        payload["result"].is_object(),
        "missing MCP result: {payload}"
    );
    assert_ne!(
        payload["result"]["isError"], true,
        "MCP tool returned isError=true: {payload}"
    );
    let text = payload["result"]["content"][0]["text"]
        .as_str()
        .expect("MCP tool text content");
    let parsed: Value = serde_json::from_str(text).expect("MCP tool text should be JSON");
    assert!(
        parsed.get("tokenUsage").is_some() || parsed.get("stats").is_some(),
        "MCP tool payload should expose result metadata: {parsed}"
    );
}

