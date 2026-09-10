use super::common::json_str;
use crate::auth;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct StatusRepair {
    kind: &'static str,
    label: String,
    command: Option<String>,
    docs: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) enum StatusRuntimeProbe {
    Ready(String),
    Starting(String),
    WrongIdentity(String),
    Unavailable(String),
    Error(String),
}
pub(crate) struct StatusReport {
    payload: Value,
    exit_code: i32,
}
fn status_docs_path() -> &'static str {
    "Info/connecting.md"
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
        label: "Run cortex setup to initialize the local brain.".into(),
        command: Some("cortex setup".into()),
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
    json!({"kind":repair.kind,"label":repair.label,"command":repair.command,"docs":repair.docs})
}

pub(crate) fn build_status_report(paths: &auth::CortexPaths, runtime_probe: StatusRuntimeProbe, db_exists: bool) -> StatusReport {
    let (status, detail) = match runtime_probe {
        StatusRuntimeProbe::Ready(detail) => ("ready", detail),
        StatusRuntimeProbe::Starting(detail) | StatusRuntimeProbe::Unavailable(detail) => ("needs_action", detail),
        StatusRuntimeProbe::WrongIdentity(detail) | StatusRuntimeProbe::Error(detail) => ("error", detail),
    };
    let next = if status == "ready" {
        status_connect_next_action()
    } else if !db_exists {
        status_setup_repair()
    } else {
        status_doctor_repair()
    };
    let payload = json!({"schemaVersion": 2, "status": status, "summary": detail,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": {"mode": "in-process", "home": paths.home.display().to_string(), "dbPath": paths.db.display().to_string()},
        "nextAction": status_repair_json(&next),
        "repair": if status == "ready" { Value::Null } else { status_repair_json(&next) },
        "checks": [{"name": "database", "status": if status == "ready" { "ok" } else { "fail" }, "detail": detail}]
    });
    StatusReport { payload, exit_code: if status == "ready" { 0 } else { 1 } }
}
async fn probe_status_runtime(paths: &auth::CortexPaths) -> StatusRuntimeProbe {
    if !paths.db.is_file() {
        return StatusRuntimeProbe::Unavailable(format!("Database not found at {}. Run cortex setup.", paths.db.display()));
    }
    match crate::CortexRuntime::open(paths) {
        Ok(runtime) => {
            let state = runtime.state();
            if state.degraded_mode.load(std::sync::atomic::Ordering::Relaxed) || state.db_corrupted.load(std::sync::atomic::Ordering::Relaxed) {
                StatusRuntimeProbe::Error("Local database opened in degraded mode; run cortex doctor.".into())
            } else {
                StatusRuntimeProbe::Ready(format!("Local brain opened at {}; no server required.", paths.db.display()))
            }
        }
        Err(err) => StatusRuntimeProbe::Error(err.to_string()),
    }
}
pub async fn run_status_cli(paths: &auth::CortexPaths, json_output: bool) -> i32 {
    let runtime_probe = probe_status_runtime(paths).await;
    let report = build_status_report(paths, runtime_probe, paths.db.exists());
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report.payload).unwrap());
    } else {
        print_status_human(&report.payload);
    }
    report.exit_code
}
fn print_status_human(payload: &Value) {
    println!("Cortex Memory Status");
    println!("Status: {}", json_str(payload, "status"));
    println!("Summary: {}", json_str(payload, "summary"));
    println!("Runtime: in-process");
    if let Some(runtime) = payload.get("runtime") {
        println!("Database: {}", json_str(runtime, "dbPath"));
    }
    if let Some(next) = payload.get("nextAction") {
        println!("Next action: {}", json_str(next, "label"));
    }
}
