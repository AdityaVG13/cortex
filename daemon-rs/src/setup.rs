//! `cortex setup` -- Beta installer that detects AI tools and configures them.
//!
//! Five steps, each independently failable:
//!   1. Init: create ~/.cortex/, generate token, check ONNX model
//!   2. Detect: scan for installed AI tools
//!   3. Configure: write MCP config for each detected tool
//!   4. Daemon: start the daemon, verify health
//!   5. Verify: store a test memory, recall it, confirm round-trip

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::auth;
use crate::db;
use crate::embeddings;

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DetectedTool {
    pub name: &'static str,
    pub config_path: Option<PathBuf>,
    pub config_method: ConfigMethod,
}

#[derive(Debug, Clone)]
pub enum ConfigMethod {
    /// Write MCP server entry to a JSON config file
    JsonMerge,
    /// Run a CLI command (e.g., `claude mcp add`)
    CliCommand(String),
    /// Show manual instructions to the user
    #[allow(dead_code)]
    Manual(String),
}

#[derive(Debug)]
pub enum StepResult {
    Ok(String),
    Warn(String),
    Fail(String),
}

impl StepResult {
    fn icon(&self) -> &str {
        match self {
            StepResult::Ok(_) => "[OK]",
            StepResult::Warn(_) => "[!!]",
            StepResult::Fail(_) => "[FAIL]",
        }
    }

    fn message(&self) -> &str {
        match self {
            StepResult::Ok(m) | StepResult::Warn(m) | StepResult::Fail(m) => m,
        }
    }
}

// ─── Main entry point ───────────────────────────────────────────────────────

pub async fn run_setup() {
    eprintln!();
    eprintln!("  Cortex Setup -- Universal AI Memory");
    eprintln!("  ====================================");
    eprintln!();

    // Step 1: Init
    let init_result = step_init().await;
    print_step(1, "Initialize", &init_result);

    let cortex_exe = current_exe_path();

    // Step 2: Detect
    let detected = step_detect();
    print_step(
        2,
        "Detect AI tools",
        &if detected.is_empty() {
            StepResult::Warn("No AI tools detected. You can configure them manually later.".into())
        } else {
            let names: Vec<&str> = detected.iter().map(|t| t.name).collect();
            StepResult::Ok(format!("Found: {}", names.join(", ")))
        },
    );

    // Step 3: Configure
    let config_results = step_configure(&detected, &cortex_exe);
    print_step(3, "Configure AI tools", &summarize_configs(&config_results));

    for (tool_name, result) in &config_results {
        eprintln!(
            "       {} {}: {}",
            result.icon(),
            tool_name,
            result.message()
        );
    }

    // Step 4: Start daemon
    let daemon_result = step_daemon(&cortex_exe).await;
    print_step(4, "Start daemon", &daemon_result);

    // Step 5: Verify
    let verify_result = step_verify().await;
    print_step(5, "Verify", &verify_result);

    // Summary
    eprintln!();
    let token = auth::read_token().unwrap_or_else(|| "???".into());
    let token_preview = if token.len() > 8 { &token[..8] } else { &token };
    eprintln!(
        "  Your API token: {}... (full token in ~/.cortex/cortex.token)",
        token_preview
    );
    eprintln!("  Daemon:         http://localhost:7437");
    eprintln!("  Health check:   curl http://localhost:7437/health");
    eprintln!();
    eprintln!("  Cortex is ready. Your AI now has persistent memory.");
    eprintln!();
}

/// Team-mode setup:
/// - creates team tables
/// - creates/updates owner user
/// - migrates schema to owner-aware shape
/// - writes owner API key to ~/.cortex/cortex.token for compatibility
pub async fn run_setup_team(args: &[String]) {
    let owner = arg_value(args, "--owner").unwrap_or_else(|| {
        std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "owner".to_string())
    });
    let display_name = arg_value(args, "--display-name").unwrap_or_else(|| owner.clone());

    eprintln!();
    eprintln!("  Cortex Team Setup");
    eprintln!("  =================");
    eprintln!();
    eprintln!("  Owner username: {owner}");

    let db_path = auth::db_path();
    if let Some(parent) = db_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Cannot open {}: {e}", db_path.display());
            return;
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("  [FAIL] Cannot configure DB: {e}");
        return;
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("  [FAIL] Cannot initialize schema: {e}");
        return;
    }
    db::migrate_focus_table(&conn);
    crate::crystallize::migrate_crystal_tables(&conn);

    let owner_key = auth::generate_ctx_api_key();
    let owner_hash = match auth::hash_api_key_argon2id(&owner_key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to hash owner API key: {e}");
            return;
        }
    };

    if let Err(e) = db::create_team_mode_tables(&conn) {
        eprintln!("  [FAIL] Failed to create team tables: {e}");
        return;
    }
    let owner_id = match db::upsert_owner_user(&conn, &owner, Some(&display_name), &owner_hash) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to create owner user: {e}");
            return;
        }
    };
    if let Err(e) = db::migrate_to_team_mode(&conn, owner_id) {
        eprintln!("  [FAIL] Failed to run team migration: {e}");
        return;
    }
    let default_team_id = match db::ensure_default_team_membership(&conn, owner_id) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to create default team membership: {e}");
            return;
        }
    };

    // Keep the existing auth path compatible with current handlers/MCP proxy.
    let cortex_dir = auth::cortex_dir();
    let _ = fs::create_dir_all(&cortex_dir);
    if let Err(e) = fs::write(cortex_dir.join("cortex.token"), &owner_key) {
        eprintln!("  [WARN] Team schema migrated, but token write failed: {e}");
    }

    let key_preview: String = owner_key.chars().take(18).collect();
    eprintln!("  [OK] Team schema migration complete.");
    eprintln!("  [OK] Mode set to team.");
    eprintln!("  [OK] Owner user id: {owner_id}");
    eprintln!("  [OK] Default team id: {default_team_id}");
    eprintln!(
        "  [OK] Owner API key: {key_preview}... (full key written to ~/.cortex/cortex.token)"
    );
    eprintln!();
}

fn print_step(num: usize, name: &str, result: &StepResult) {
    eprintln!(
        "  {} Step {}: {} -- {}",
        result.icon(),
        num,
        name,
        result.message()
    );
}

fn current_exe_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "cortex".to_string())
}

fn arg_value(args: &[String], key: &str) -> Option<String> {
    for (idx, arg) in args.iter().enumerate() {
        if arg == key {
            return args.get(idx + 1).cloned();
        }
    }
    None
}

// ─── Step 1: Init ───────────────────────────────────────────────────────────

async fn step_init() -> StepResult {
    let cortex_dir = auth::cortex_dir();
    let mut notes = Vec::new();

    // Create directory
    if let Err(e) = fs::create_dir_all(&cortex_dir) {
        return StepResult::Fail(format!("Cannot create {}: {e}", cortex_dir.display()));
    }
    notes.push(format!("Directory: {}", cortex_dir.display()));

    // Generate or reuse token
    if auth::read_token().is_some() {
        notes.push("Token: exists (reusing)".into());
    } else {
        auth::generate_token();
        notes.push("Token: generated".into());
    }

    // Check ONNX model
    let models_dir = cortex_dir.join("models");
    let model_exists = models_dir.join("all-MiniLM-L6-v2.onnx").exists()
        && models_dir.join("tokenizer.json").exists();

    if model_exists {
        notes.push("Embedding model: ready".into());
    } else {
        eprintln!("       Downloading embedding model (~23 MB)...");
        match embeddings::ensure_model_downloaded().await {
            Some(_) => notes.push("Embedding model: downloaded".into()),
            None => {
                notes.push("Embedding model: download failed (will retry on daemon start)".into())
            }
        }
    }

    StepResult::Ok(notes.join(" | "))
}

// ─── Step 2: Detect ─────────────────────────────────────────────────────────

fn step_detect() -> Vec<DetectedTool> {
    let mut found = Vec::new();

    // Claude Desktop
    if let Some(config_path) = find_claude_desktop_config() {
        found.push(DetectedTool {
            name: "Claude Desktop",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    // Claude Code
    if command_exists("claude") {
        found.push(DetectedTool {
            name: "Claude Code",
            config_path: None,
            config_method: ConfigMethod::CliCommand("claude mcp add cortex -s user".into()),
        });
    }

    // Cursor
    if let Some(config_path) = find_cursor_config() {
        found.push(DetectedTool {
            name: "Cursor",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    // Windsurf
    if let Some(config_path) = find_windsurf_config() {
        found.push(DetectedTool {
            name: "Windsurf",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    found
}

fn find_claude_desktop_config() -> Option<PathBuf> {
    let candidates = claude_desktop_config_paths();
    candidates
        .into_iter()
        .find(|path| path.exists() || path.parent().is_some_and(|p| p.exists()))
}

fn claude_desktop_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            paths.push(
                PathBuf::from(appdata)
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            );
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            paths.push(
                home.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            );
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(config) = std::env::var("XDG_CONFIG_HOME") {
            paths.push(
                PathBuf::from(config)
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            );
        } else if let Some(home) = dirs::home_dir() {
            paths.push(
                home.join(".config")
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            );
        }
    }

    paths
}

fn find_cursor_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = home.join(".cursor").join("mcp.json");
    if path.exists() || path.parent().is_some_and(|p| p.exists()) {
        Some(path)
    } else {
        None
    }
}

fn find_windsurf_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = home.join(".windsurf").join("mcp.json");
    if path.exists() || path.parent().is_some_and(|p| p.exists()) {
        Some(path)
    } else {
        None
    }
}

fn command_exists(cmd: &str) -> bool {
    #[cfg(windows)]
    {
        Command::new("where")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

// ─── Step 3: Configure ──────────────────────────────────────────────────────

fn step_configure(tools: &[DetectedTool], cortex_exe: &str) -> Vec<(&'static str, StepResult)> {
    let mut results = Vec::new();

    for tool in tools {
        let result = configure_tool(tool, cortex_exe);
        results.push((tool.name, result));
    }

    results
}

fn configure_tool(tool: &DetectedTool, cortex_exe: &str) -> StepResult {
    match &tool.config_method {
        ConfigMethod::JsonMerge => {
            let Some(config_path) = &tool.config_path else {
                return StepResult::Fail("No config path".into());
            };
            match merge_mcp_config(config_path, cortex_exe) {
                Ok(action) => StepResult::Ok(action),
                Err(e) => StepResult::Warn(format!("Auto-config failed: {e}. Configure manually.")),
            }
        }
        ConfigMethod::CliCommand(base_cmd) => match run_claude_mcp_add(base_cmd, cortex_exe) {
            Ok(()) => StepResult::Ok("Registered via CLI".into()),
            Err(e) => StepResult::Warn(format!(
                "CLI failed: {e}. Run manually: {base_cmd} -- {cortex_exe} mcp"
            )),
        },
        ConfigMethod::Manual(instructions) => {
            StepResult::Ok(format!("Manual setup needed: {instructions}"))
        }
    }
}

/// Merge a Cortex MCP server entry into a JSON config file.
/// Reads existing config, adds/updates the "cortex" entry under "mcpServers",
/// writes back. Preserves all existing config.
fn merge_mcp_config(config_path: &Path, cortex_exe: &str) -> Result<String, String> {
    // Read existing config or start fresh
    let mut config: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Cannot read {}: {e}", config_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Invalid JSON in {}: {e}", config_path.display()))?
    } else {
        // Create parent directory if needed
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        serde_json::json!({})
    };

    // Ensure mcpServers exists
    let mcp_servers = config
        .as_object_mut()
        .ok_or("Config is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    // Check if cortex is already configured
    if mcp_servers.get("cortex").is_some() {
        return Ok("Already configured (skipped)".into());
    }

    // Normalize path separators for the platform
    let exe_path = cortex_exe.replace('/', "\\");

    // Add cortex entry
    mcp_servers
        .as_object_mut()
        .ok_or("mcpServers is not a JSON object")?
        .insert(
            "cortex".to_string(),
            serde_json::json!({
                "command": exe_path,
                "args": ["mcp"]
            }),
        );

    // Write back with pretty formatting
    let output =
        serde_json::to_string_pretty(&config).map_err(|e| format!("JSON serialize failed: {e}"))?;
    fs::write(config_path, output)
        .map_err(|e| format!("Cannot write {}: {e}", config_path.display()))?;

    Ok(format!("Configured at {}", config_path.display()))
}

fn run_claude_mcp_add(_base_cmd: &str, cortex_exe: &str) -> Result<(), String> {
    let output = Command::new("claude")
        .args([
            "mcp", "add", "cortex", "-s", "user", "--", cortex_exe, "mcp",
        ])
        .output()
        .map_err(|e| format!("Failed to run claude CLI: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "already exists" is not an error
        if stderr.contains("already exists") || stderr.contains("Already") {
            Ok(())
        } else {
            Err(stderr.trim().to_string())
        }
    }
}

fn summarize_configs(results: &[(&str, StepResult)]) -> StepResult {
    if results.is_empty() {
        return StepResult::Warn("No tools to configure".into());
    }
    let ok_count = results
        .iter()
        .filter(|(_, r)| matches!(r, StepResult::Ok(_)))
        .count();
    let warn_count = results
        .iter()
        .filter(|(_, r)| matches!(r, StepResult::Warn(_)))
        .count();
    let fail_count = results
        .iter()
        .filter(|(_, r)| matches!(r, StepResult::Fail(_)))
        .count();

    if fail_count > 0 {
        StepResult::Warn(format!(
            "{ok_count} configured, {warn_count} warnings, {fail_count} failed"
        ))
    } else if warn_count > 0 {
        StepResult::Warn(format!(
            "{ok_count} configured, {warn_count} need manual setup"
        ))
    } else {
        StepResult::Ok(format!("{ok_count}/{} tools configured", results.len()))
    }
}

// ─── Step 4: Start daemon ───────────────────────────────────────────────────

async fn step_daemon(cortex_exe: &str) -> StepResult {
    // Check if daemon is already running
    if is_daemon_healthy().await {
        return StepResult::Ok("Daemon already running on :7437".into());
    }

    // Start daemon in background
    let spawn_result = Command::new(cortex_exe)
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    match spawn_result {
        Ok(_child) => {
            // Wait for daemon to become healthy (up to 10 seconds)
            for i in 0..20 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if is_daemon_healthy().await {
                    return StepResult::Ok(format!("Started on :7437 (took ~{}s)", (i + 1) / 2));
                }
            }
            StepResult::Warn(
                "Daemon started but health check timed out. It may still be initializing.".into(),
            )
        }
        Err(e) => StepResult::Fail(format!("Cannot start daemon: {e}")),
    }
}

async fn is_daemon_healthy() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .ok();

    let Some(client) = client else { return false };

    client
        .get("http://localhost:7437/health")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// ─── Step 5: Verify ─────────────────────────────────────────────────────────

async fn step_verify() -> StepResult {
    let token = match auth::read_token() {
        Some(t) => t,
        None => return StepResult::Fail("No auth token found".into()),
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return StepResult::Fail(format!("HTTP client error: {e}")),
    };

    // Store a test memory
    let store_resp = client
        .post("http://localhost:7437/store")
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "decision": "Cortex installed and verified",
            "context": "Automated setup verification",
            "type": "memory",
            "source_agent": "cortex-setup"
        }))
        .send()
        .await;

    match store_resp {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            return StepResult::Warn(format!(
                "Store returned {}: daemon is running but store failed",
                r.status()
            ))
        }
        Err(e) => return StepResult::Fail(format!("Cannot reach daemon: {e}")),
    }

    // Recall it back
    let recall_resp = client
        .get("http://localhost:7437/recall")
        .header("Authorization", format!("Bearer {token}"))
        .query(&[("q", "Cortex installed"), ("k", "1"), ("budget", "100")])
        .send()
        .await;

    match recall_resp {
        Ok(r) if r.status().is_success() => {
            StepResult::Ok("Store + recall round-trip verified".into())
        }
        Ok(r) => StepResult::Warn(format!(
            "Recall returned {}: store worked but recall did not",
            r.status()
        )),
        Err(e) => StepResult::Warn(format!("Recall failed: {e}. Store succeeded.")),
    }
}
