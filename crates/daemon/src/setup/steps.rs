use super::configure::{step_configure, summarize_configs};
use super::detect::step_detect;
use super::helpers::{print_step, stable_mcp_binary_path};
use super::types::StepResult;
use crate::{CortexRuntime, auth};
use asupersync::Cx;

pub async fn run_setup(cx: &Cx) {
    let paths = auth::CortexPaths::resolve();
    let runtime = match CortexRuntime::open(&paths) {
        Ok(runtime) => runtime,
        Err(err) => {
            print_step(1, "Initialize local brain", &StepResult::Fail(err.to_string()));
            return;
        }
    };
    print_step(1, "Initialize local brain", &StepResult::Ok(paths.db.display().to_string()));
    let detected = step_detect();
    let names = detected.iter().map(|tool| tool.name).collect::<Vec<_>>().join(", ");
    print_step(
        2,
        "Detect AI tools",
        &if detected.is_empty() { StepResult::Warn("No tools detected; configure cortex mcp manually.".into()) } else { StepResult::Ok(names) },
    );
    let results = step_configure(&detected, &stable_mcp_binary_path());
    print_step(3, "Configure AI tools", &summarize_configs(&results));
    let result = runtime
        .boot(cx, crate::runtime::BootInput { agent: "cortex-setup".into(), max_tokens: 100, owner_id: runtime.state().default_owner_id })
        .await;
    let verified = match result {
        Ok(_) => StepResult::Ok("Local boot compilation succeeded. No HTTP server or service required.".into()),
        Err(err) => StepResult::Fail(format!("Local boot failed: {err}")),
    };
    print_step(4, "Verify local brain", &verified);
}
