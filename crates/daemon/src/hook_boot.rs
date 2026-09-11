use cortex_kernel::hook_event::{frame_from_host, process};
use cortex_logic::adapter::CapabilityManifest;
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

const DEFAULT_BUDGET: u32 = 600;

struct BootResult {
    boot_prompt: String,
    token_estimate: Option<i64>,
    savings: Option<serde_json::Value>,
}
struct HealthResult {
    memories: i64,
    decisions: i64,
    embeddings: i64,
}

async fn fetch_boot(cx: &asupersync::Cx, agent: &str, budget: u32, runtime: &crate::CortexRuntime) -> Option<BootResult> {
    let state = runtime.state();
    if state.team_mode && state.default_owner_id.is_none() {
        return None;
    }
    let result = runtime
        .boot(cx, crate::runtime::BootInput { agent: agent.to_string(), max_tokens: budget as usize, owner_id: state.default_owner_id, ..Default::default() })
        .await
        .ok()?;
    Some(BootResult {
        boot_prompt: result.boot_prompt,
        token_estimate: i64::try_from(result.token_estimate).ok(),
        savings: Some(result.savings),
    })
}

async fn fetch_health(cx: &asupersync::Cx, runtime: &crate::CortexRuntime) -> Option<HealthResult> {
    let data = crate::handlers::health::build_health_payload(cx, runtime.state(), false).await.ok()?;
    let stats = data.get("stats")?;
    Some(HealthResult {
        memories: stats.get("memories")?.as_i64()?,
        decisions: stats.get("decisions")?.as_i64()?,
        embeddings: stats.get("embeddings").and_then(|v| v.as_i64()).unwrap_or(0),
    })
}

fn status_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claude").join("brain-status.json")
}

async fn register_session(cx: &asupersync::Cx, agent: &str, runtime: &crate::CortexRuntime) {
    let state = runtime.state();
    let source = crate::handlers::SourceIdentity { agent: agent.to_string(), model: None };
    if let Err(err) = crate::handlers::register_agent_presence(cx, state, &source, state.default_owner_id, "", "Active coding session").await {
        eprintln!("[cortex] session registration failed: {err}");
    }
}

fn read_optional_payload() -> Value {
    let mut raw = Vec::new();
    let _ = std::io::stdin().lock().take(64 * 1024).read_to_end(&mut raw);
    serde_json::from_slice(&raw).unwrap_or_else(|_| {
        json!({
            "hook_event_name": "SessionStart",
            "cwd": std::env::var("PWD").or_else(|_| std::env::current_dir().map(|p| p.display().to_string())).unwrap_or_default(),
        })
    })
}

/// Same View as MCP `cortex_orient` / `process()` SessionStart. Empty means fall back to boot.
pub async fn session_start_context(
    cx: &asupersync::Cx,
    agent: &str,
    runtime: &crate::CortexRuntime,
    payload: &Value,
) -> Option<String> {
    let manifest = CapabilityManifest::claude_code_plugin();
    let frame = frame_from_host("SessionStart", payload, manifest);
    let result = process(cx, runtime, agent, &frame, payload).await.ok()?;
    result.additional_context.then_nonempty()
}

trait ThenNonEmpty {
    fn then_nonempty(self) -> Option<String>;
}
impl ThenNonEmpty for String {
    fn then_nonempty(self) -> Option<String> {
        if self.trim().is_empty() { None } else { Some(self) }
    }
}

pub async fn run_boot(cx: &asupersync::Cx, agent: &str) {
    let paths = crate::auth::CortexPaths::resolve();
    let runtime = match crate::CortexRuntime::open(&paths) {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("[cortex-hook] memory unavailable: {err}");
            return;
        }
    };
    let payload = read_optional_payload();
    let (boot, health) = futures_util::future::join(fetch_boot(cx, agent, DEFAULT_BUDGET, &runtime), fetch_health(cx, &runtime)).await;
    if boot.is_some() {
        register_session(cx, agent, &runtime).await;
    }
    let (total, memories, decisions) = health.as_ref().map(|h| (h.memories + h.decisions, h.memories, h.decisions)).unwrap_or((0, 0, 0));
    let status = json!({"timestamp":chrono::Utc::now().to_rfc3339(),"overall":if boot.is_some() {"ONLINE"} else {"DEGRADED"},"cortex":{"connected":health.is_some(),"booted":boot.is_some(),"total":total,"memories":memories,"decisions":decisions,"tokenEstimate":boot.as_ref().and_then(|b|b.token_estimate),"savings":boot.as_ref().and_then(|b|b.savings.clone())}});
    if let Err(err) = std::fs::write(status_path(), status.to_string()) {
        eprintln!("[cortex-hook] status write failed: {err}");
    }
    let context = match session_start_context(cx, agent, &runtime, &payload).await {
        Some(context) => Some(context),
        None => match boot {
            Some(boot) if !boot.boot_prompt.trim().is_empty() => {
                let mut context = boot.boot_prompt;
                let cues = crate::runtime::assembly::tokenize_cues(&context);
                if let Ok(compiled) = runtime
                    .compile_assemblies(cx, "project", &cues, 4, None, None, None, "")
                    .await
                {
                    if !compiled.brief.is_empty() {
                        context.push('\n');
                        context.push_str(&compiled.brief);
                    }
                }
                Some(context)
            }
            Some(_) => None,
            None => {
                eprintln!("[cortex-hook] boot unavailable; no memory context delivered");
                None
            }
        },
    };
    if let Some(context) = context {
        println!("{}", json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":context}}));
    }
}

pub async fn run_status(cx: &asupersync::Cx) {
    let paths = crate::auth::CortexPaths::resolve();
    let health = match crate::CortexRuntime::open(&paths) {
        Ok(runtime) => fetch_health(cx, &runtime).await,
        Err(_) => None,
    };
    match health {
        Some(h) => {
            println!("ONLINE | {} mem | {} dec | {} emb", h.memories, h.decisions, h.embeddings);
        }
        None => {
            println!("OFFLINE");
        }
    }
}
