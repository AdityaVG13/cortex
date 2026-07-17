// SPDX-License-Identifier: MIT
use super::common::json_str;
use crate::auth;
use crate::daemon_lifecycle::{is_cortex_health_payload, readiness_state_from_payload};
use crate::transport;
use serde_json::{json, Value};
use std::time::Duration;
pub(crate) const STATUS_SCHEMA_VERSION: u32 = 1;
#[derive(Debug, Clone)]
struct StatusRepair {
    kind: &'static str,
    label: String,
    command: Option<String>,
    docs: &'static str,
}
#[derive(Debug, Clone)]
struct StatusCheck {
    name: &'static str,
    status: &'static str,
    detail: String,
    repair: Option<StatusRepair>,
}
#[derive(Debug, Clone)]
enum StatusRuntimeProbe {
    Ready(String),
    Starting(String),
    WrongIdentity(String),
    Unavailable(String),
    Error(String),
}
struct StatusReport {
    payload: Value,
    exit_code: i32,
}
fn status_docs_path() -> &'static str {
    "Info/connecting.md"
}
fn status_start_repair() -> StatusRepair {
    StatusRepair {
        kind: "start_local_runtime",
        label: "Open Cortex Control Center, or run `cortex serve` for CLI-only local mode.".to_string(),
        command: Some("cortex serve".to_string()),
        docs: status_docs_path(),
    }
}
fn status_wait_repair() -> StatusRepair {
    StatusRepair {
        kind: "wait_for_startup",
        label: "Wait a few seconds, then run `cortex status` again.".to_string(),
        command: Some("cortex status".to_string()),
        docs: status_docs_path(),
    }
}
fn status_identity_repair() -> StatusRepair {
    StatusRepair {
        kind: "repair_runtime_identity",
        label: "Stop the other process or switch to the matching CORTEX_HOME/CORTEX_PORT, then retry.".to_string(),
        command: Some("cortex status --json".to_string()),
        docs: "Info/startup-matrix-troubleshooting.md",
    }
}
fn status_doctor_repair() -> StatusRepair {
    StatusRepair {
        kind: "run_doctor",
        label: "Run `cortex doctor`, then retry `cortex status --json`.".to_string(),
        command: Some("cortex doctor".to_string()),
        docs: status_docs_path(),
    }
}
fn status_setup_repair() -> StatusRepair {
    StatusRepair {
        kind: "run_setup",
        label: "Run `cortex setup`, then start Cortex from Control Center or `cortex serve`.".to_string(),
        command: Some("cortex setup".to_string()),
        docs: status_docs_path(),
    }
}
fn status_connect_next_action() -> StatusRepair {
    StatusRepair {
        kind: "connect_tool_or_smoke",
        label: "Connect an AI tool, then store and recall one memory; CLI users can start with `cortex boot --agent smoke-test --json`.".to_string(),
        command: Some("cortex boot --agent smoke-test --json".to_string()),
        docs: status_docs_path(),
    }
}
fn status_repair_json(repair: &StatusRepair) -> Value {
    json!({
        "kind": repair.kind,
        "label": repair.label,
        "command": repair.command,
        "docs": repair.docs
    })
}
fn status_check_json(check: &StatusCheck) -> Value {
    let repair = check.repair.as_ref().map(status_repair_json);
    json!({
        "name": check.name,
        "status": check.status,
        "detail": check.detail,
        "repair": repair
    })
}
fn compact_status_detail(value: &str) -> String {
    let compacted = value.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_DETAIL_CHARS: usize = 220;
    if compacted.chars().count() <= MAX_DETAIL_CHARS {
        return compacted;
    }
    let mut out = compacted.chars().take(MAX_DETAIL_CHARS).collect::<String>();
    out.push_str("...");
    out
}
pub(crate) fn build_status_report(paths: &auth::CortexPaths, runtime_probe: StatusRuntimeProbe, token_exists: bool, db_exists: bool) -> StatusReport {
    let runtime_base_url = transport::local_http_base_url(paths);
    let (mut status, summary, runtime_check, mut next_action) = match runtime_probe {
        StatusRuntimeProbe::Ready(detail) => {
            let next = status_connect_next_action();
            (
                "ready",
                "Cortex memory is ready for local AI tools.".to_string(),
                StatusCheck {
                    name: "runtime_identity",
                    status: "ok",
                    detail,
                    repair: None,
                },
                next,
            )
        }
        StatusRuntimeProbe::Starting(detail) => {
            let repair = status_wait_repair();
            (
                "needs_action",
                "Cortex is starting but is not ready yet.".to_string(),
                StatusCheck {
                    name: "runtime_identity",
                    status: "warn",
                    detail,
                    repair: Some(repair.clone()),
                },
                repair,
            )
        }
        StatusRuntimeProbe::WrongIdentity(detail) => {
            let repair = status_identity_repair();
            (
                "error",
                "A Cortex-like service answered, but it is not the expected local runtime.".to_string(),
                StatusCheck {
                    name: "runtime_identity",
                    status: "fail",
                    detail,
                    repair: Some(repair.clone()),
                },
                repair,
            )
        }
        StatusRuntimeProbe::Unavailable(detail) => {
            let repair = status_start_repair();
            (
                "needs_action",
                "No ready Cortex runtime answered the local readiness probe.".to_string(),
                StatusCheck {
                    name: "runtime_identity",
                    status: "warn",
                    detail,
                    repair: Some(repair.clone()),
                },
                repair,
            )
        }
        StatusRuntimeProbe::Error(detail) => {
            let repair = status_doctor_repair();
            (
                "error",
                "The local runtime probe returned an unexpected response.".to_string(),
                StatusCheck {
                    name: "runtime_identity",
                    status: "fail",
                    detail,
                    repair: Some(repair.clone()),
                },
                repair,
            )
        }
    };
    let mut checks = vec![runtime_check];
    if token_exists {
        checks.push(StatusCheck {
            name: "auth_token",
            status: "ok",
            detail: format!("Token file exists at {}", paths.token.display()),
            repair: None,
        });
    } else {
        let repair = status_setup_repair();
        if status == "ready" {
            status = "needs_action";
            next_action = repair.clone();
        }
        checks.push(StatusCheck {
            name: "auth_token",
            status: "fail",
            detail: format!("No token file found at {}", paths.token.display()),
            repair: Some(repair),
        });
    }
    checks.push(StatusCheck {
        name: "database",
        status: if db_exists { "ok" } else { "warn" },
        detail: if db_exists {
            format!("Database exists at {}", paths.db.display())
        } else {
            format!("Database not found at {}; it is created during first setup/start.", paths.db.display())
        },
        repair: if db_exists { None } else { Some(status_setup_repair()) },
    });
    let exit_code = if status == "ready" { 0 } else { 1 };
    let repair = if status == "ready" { None } else { Some(status_repair_json(&next_action)) };
    let payload = json!({
        "schemaVersion": STATUS_SCHEMA_VERSION,
        "status": status,
        "summary": summary,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": {
            "baseUrl": runtime_base_url,
            "port": paths.port,
            "bind": paths.bind,
            "home": paths.home.display().to_string(),
            "dbPath": paths.db.display().to_string(),
            "tokenPath": paths.token.display().to_string(),
            "pidPath": paths.pid.display().to_string()
        },
        "nextAction": status_repair_json(&next_action),
        "repair": repair,
        "checks": checks.iter().map(status_check_json).collect::<Vec<_>>()
    });
    StatusReport { payload, exit_code }
}
async fn probe_status_runtime(paths: &auth::CortexPaths) -> StatusRuntimeProbe {
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(2)).build() {
        Ok(client) => client,
        Err(err) => {
            return StatusRuntimeProbe::Error(format!("Could not create HTTP client: {err}"));
        }
    };
    let base_url = transport::local_http_base_url(paths);
    let mut probe_errors = Vec::new();
    let probe_headers = [(String::from("X-Cortex-Request"), String::from("true"))];
    match transport::request_with_local_ipc_fallback(&client, "GET", &base_url, "/readiness", paths, &probe_headers, None, Duration::from_secs(2)).await {
        Ok((status, body)) => {
            let status_code = status.as_u16();
            if let Some(ready) = readiness_state_from_payload(status_code, &body, Some(paths.port), Some(paths)) {
                return if ready {
                    StatusRuntimeProbe::Ready(format!("Readiness endpoint reports ready at {base_url}/readiness."))
                } else {
                    StatusRuntimeProbe::Starting(format!("Readiness endpoint reports startup in progress at {base_url}/readiness."))
                };
            }
            if readiness_state_from_payload(status_code, &body, Some(paths.port), None).is_some() {
                return StatusRuntimeProbe::WrongIdentity(format!(
                    "Readiness endpoint answered on port {}, but home/db/token paths do not match {}.",
                    paths.port,
                    paths.home.display()
                ));
            }
            probe_errors.push(format!("readiness HTTP {status}: {}", compact_status_detail(&body)));
        }
        Err(err) => probe_errors.push(format!("readiness failed: {err}")),
    }
    match transport::request_with_local_ipc_fallback(&client, "GET", &base_url, "/health", paths, &probe_headers, None, Duration::from_secs(2)).await {
        Ok((status, body)) => {
            let status_code = status.as_u16();
            if is_cortex_health_payload(status_code, &body, Some(paths.port), Some(paths)) {
                return StatusRuntimeProbe::Ready(format!("Health endpoint reports canonical Cortex runtime at {base_url}/health."));
            }
            if is_cortex_health_payload(status_code, &body, Some(paths.port), None) {
                return StatusRuntimeProbe::WrongIdentity(format!(
                    "Health endpoint answered on port {}, but home/db/token paths do not match {}.",
                    paths.port,
                    paths.home.display()
                ));
            }
            probe_errors.push(format!("health HTTP {status}: {}", compact_status_detail(&body)));
            StatusRuntimeProbe::Error(probe_errors.join("; "))
        }
        Err(err) => {
            probe_errors.push(format!("health failed: {err}"));
            StatusRuntimeProbe::Unavailable(probe_errors.join("; "))
        }
    }
}
pub(crate) async fn run_status_cli(paths: &auth::CortexPaths, json_output: bool) -> i32 {
    let runtime_probe = probe_status_runtime(paths).await;
    let report = build_status_report(paths, runtime_probe, paths.token.exists(), paths.db.exists());
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report.payload).unwrap());
    } else {
        print_status_human(&report.payload);
    }
    report.exit_code
}
fn print_status_human(payload: &Value) {
    println!("Cortex Memory Status");
    println!("{}", "=".repeat(50));
    println!("Status: {}", json_str(payload, "status"));
    println!("Summary: {}", json_str(payload, "summary"));
    if let Some(runtime) = payload.get("runtime").and_then(Value::as_object) {
        println!("Runtime: {}", runtime.get("baseUrl").and_then(Value::as_str).unwrap_or("unknown"));
        println!("Home: {}", runtime.get("home").and_then(Value::as_str).unwrap_or("unknown"));
    }
    if let Some(next_action) = payload.get("nextAction").and_then(Value::as_object) {
        println!("Next action: {}", next_action.get("label").and_then(Value::as_str).unwrap_or("Run cortex status --json"));
        if let Some(command) = next_action.get("command").and_then(Value::as_str) {
            println!("Command: {command}");
        }
    }
    println!();
    println!("Checks:");
    if let Some(checks) = payload.get("checks").and_then(Value::as_array) {
        for check in checks {
            let status = check.get("status").and_then(Value::as_str).unwrap_or("unknown");
            let marker = match status {
                "ok" => "[OK]",
                "warn" => "[!!]",
                "fail" => "[FAIL]",
                _ => "[??]",
            };
            let name = check.get("name").and_then(Value::as_str).unwrap_or("check");
            let detail = check.get("detail").and_then(Value::as_str).unwrap_or("");
            println!("  {marker} {name}: {detail}");
            if let Some(repair) = check.get("repair").and_then(Value::as_object) {
                if let Some(label) = repair.get("label").and_then(Value::as_str) {
                    println!("       Repair: {label}");
                }
            }
        }
    }
    println!();
    println!("JSON: cortex status --json");
}
