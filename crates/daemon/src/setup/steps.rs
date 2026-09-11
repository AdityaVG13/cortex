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
        .boot(cx, crate::runtime::BootInput { agent: "cortex-setup".into(), max_tokens: 100, owner_id: runtime.state().default_owner_id, ..Default::default() })
        .await;
    let verified = match result {
        Ok(_) => StepResult::Ok("Local boot compilation succeeded. No HTTP server or service required.".into()),
        Err(err) => StepResult::Fail(format!("Local boot failed: {err}")),
    };
    print_step(4, "Verify local brain", &verified);
    print_step(5, "Enable host capture", &enable_host_capture(cx, &runtime, &paths).await);
}

async fn enable_host_capture(cx: &Cx, runtime: &CortexRuntime, paths: &auth::CortexPaths) -> StepResult {
    let grant = cortex_kernel::runtime::host_capture::installed_host_grant();
    let grant_msg = match runtime.register_host_capture(cx, grant).await {
        Ok(()) => "grant registered".to_string(),
        Err(err) if err.contains("host_registration_conflict") => "grant already present".into(),
        Err(err) => return StepResult::Warn(format!("Grant not registered: {err}. Host events stay silent until you register one.")),
    };
    let sidecar = cortex_kernel::runtime::host_capture::installed_capture_sidecar();
    match cortex_kernel::hook_event::write_installed_capture_sidecar(paths, &sidecar) {
        Ok(true) => StepResult::Ok(format!(
            "{grant_msg}; wrote {}. Plugin SessionStart uses cortex_orient; live hooks read this file when CORTEX_CAPTURE is unset.",
            paths.capture_sidecar().display()
        )),
        Ok(false) => StepResult::Ok(format!(
            "{grant_msg}; left existing {} in place.",
            paths.capture_sidecar().display()
        )),
        Err(err) => StepResult::Warn(format!("{grant_msg}; sidecar not written: {err}")),
    }
}
