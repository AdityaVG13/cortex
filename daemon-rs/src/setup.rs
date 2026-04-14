// SPDX-License-Identifier: MIT
//! `cortex setup` -- Beta installer that detects AI tools and configures them.
//!
//! Five steps, each independently failable:
//!   1. Init: create ~/.cortex/, generate token, check ONNX model
//!   2. Detect: scan for installed AI tools
//!   3. Configure: write MCP config for each detected tool
//!   4. Daemon: verify whether a daemon is already available
//!   5. Verify: if a daemon is already running, store a test memory and confirm round-trip

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::auth;
use crate::db;
use crate::embeddings;

fn daemon_port() -> u16 {
    auth::CortexPaths::resolve().port
}

fn daemon_base_url() -> String {
    format!("http://localhost:{}", daemon_port())
}

fn daemon_url(path: &str) -> String {
    format!("{}{}", daemon_base_url(), path)
}

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DetectedTool {
    pub name: &'static str,
    pub agent_name: &'static str,
    pub config_path: Option<PathBuf>,
    pub config_method: ConfigMethod,
}

#[derive(Debug, Clone)]
pub enum ConfigMethod {
    /// Write MCP server entry to a JSON config file
    JsonMerge,
    /// Write MCP server entry to a TOML config file
    TomlMerge,
    /// Run a CLI command (e.g., `claude mcp add`)
    CliCommand {
        program: &'static str,
        args: &'static [&'static str],
    },
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

    let cortex_exe = stable_mcp_binary_path();

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

    // Step 4: Check daemon availability
    let daemon_result = step_daemon().await;
    print_step(4, "Daemon availability", &daemon_result);

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
    eprintln!("  Daemon:         {}", daemon_base_url());
    eprintln!("  Health check:   curl {}", daemon_url("/health"));
    eprintln!();
    eprintln!("  Cortex is configured. Start it from Control Center or let your client run `cortex mcp --agent <name>` when you want a live daemon.");
    eprintln!();
}

/// Team-mode setup:
/// - backs up the database
/// - creates team tables
/// - creates/updates owner user
/// - migrates schema to owner-aware shape
/// - reports per-table row counts
/// - writes owner API key to ~/.cortex/cortex.token for compatibility
///
/// When `dry_run` is true the migration runs inside a transaction that is
/// rolled back so the database is left untouched.
pub async fn run_setup_team(args: &[String], dry_run: bool) {
    let db_path = auth::db_path();
    if let Some(parent) = db_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Open DB early so we can check current mode before prompting.
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

    // Idempotency: refuse to re-migrate.
    if db::is_team_mode(&conn) {
        eprintln!();
        eprintln!("  Already in team mode. No changes needed.");
        eprintln!();
        return;
    }

    // Resolve owner: flag > interactive prompt > env fallback.
    let default_owner = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "owner".to_string());

    let owner = if let Some(v) = arg_value(args, "--owner") {
        v
    } else {
        eprint!("  Enter owner username [default: {default_owner}]: ");
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() && !input.trim().is_empty() {
            input.trim().to_string()
        } else {
            default_owner.clone()
        }
    };

    let display_name = arg_value(args, "--display-name").unwrap_or_else(|| owner.clone());

    eprintln!();
    if dry_run {
        eprintln!("  [DRY RUN] Cortex Team Migration Preview");
        eprintln!("  ========================================");
    } else {
        eprintln!("  Cortex Team Setup");
        eprintln!("  =================");
    }
    eprintln!();
    eprintln!("  Owner username: {owner}");
    eprintln!();

    // ── Pre-migration backup (skip for dry-run) ────────────────────────────
    if !dry_run && db_path.exists() {
        let bak_path = db_path.with_extension("db.bak");
        eprint!("  Backing up database to {}... ", bak_path.display());

        // Close the connection to release the WAL lock before copying.
        drop(conn);

        if let Err(e) = fs::copy(&db_path, &bak_path) {
            eprintln!("FAILED");
            eprintln!("  [FAIL] Backup failed: {e}  -- aborting migration.");
            return;
        }
        eprintln!("done");

        // Also copy WAL/SHM if they exist (ensures consistent backup).
        let wal = db_path.with_extension("db-wal");
        let shm = db_path.with_extension("db-shm");
        if wal.exists() {
            let _ = fs::copy(&wal, bak_path.with_extension("db.bak-wal"));
        }
        if shm.exists() {
            let _ = fs::copy(&shm, bak_path.with_extension("db.bak-shm"));
        }
    } else if !dry_run {
        // No DB yet -- nothing to back up, will be created fresh.
    }

    // Re-open the connection (closed above for backup).
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Cannot reopen {}: {e}", db_path.display());
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

    // For dry-run we wrap everything in a transaction we will roll back.
    if dry_run {
        if let Err(e) = conn.execute_batch("BEGIN") {
            eprintln!("  [FAIL] Cannot begin transaction: {e}");
            return;
        }
    }

    eprintln!("  Migrating to team mode...");
    eprintln!();

    let owner_key = auth::generate_ctx_api_key();
    let owner_hash = match auth::hash_api_key_argon2id(&owner_key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to hash owner API key: {e}");
            if dry_run {
                let _ = conn.execute_batch("ROLLBACK");
            }
            return;
        }
    };

    eprint!("  Creating team tables... ");
    if let Err(e) = db::create_team_mode_tables(&conn) {
        eprintln!("FAILED");
        eprintln!("  [FAIL] {e}");
        if dry_run {
            let _ = conn.execute_batch("ROLLBACK");
        }
        return;
    }
    eprintln!("done");

    let owner_id = match db::upsert_owner_user(&conn, &owner, Some(&display_name), &owner_hash) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to create owner user: {e}");
            if dry_run {
                let _ = conn.execute_batch("ROLLBACK");
            }
            return;
        }
    };

    eprint!("  Adding ownership columns... ");
    if let Err(e) = db::migrate_to_team_mode(&conn, owner_id) {
        eprintln!("FAILED");
        eprintln!("  [FAIL] {e}");
        if dry_run {
            let _ = conn.execute_batch("ROLLBACK");
        }
        return;
    }
    eprintln!("done");
    eprintln!();

    // ── Row count report ───────────────────────────────────────────────────
    let counts = db::migration_counts(&conn);
    let total: i64 = counts.iter().map(|(_, n)| n).sum();

    if dry_run {
        eprintln!("  [DRY RUN] Would migrate to team mode:");
    } else {
        eprintln!("  Assigned ownership:");
    }

    let label_width = 22;
    for (table, count) in &counts {
        if dry_run {
            eprintln!(
                "    {:<width$} {:>6} rows would be assigned",
                format!("{table}:"),
                count,
                width = label_width,
            );
        } else {
            eprintln!(
                "    {:<width$} {:>6} rows",
                format!("{table}:"),
                count,
                width = label_width,
            );
        }
    }

    if dry_run {
        eprintln!(
            "    {:<width$} {:>6} rows",
            "Total:",
            total,
            width = label_width
        );
        eprintln!();
        let _ = conn.execute_batch("ROLLBACK");
        eprintln!("  No changes made.");
        eprintln!();
        return;
    }

    // Non-dry-run: finish migration.
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

    eprintln!("    ────────────────────────────");
    eprintln!(
        "    {:<width$} {:>6} rows -> owner \"{owner}\" (id: {owner_id})",
        "Total:",
        total,
        width = label_width,
    );
    eprintln!();
    eprintln!("  All rows set to visibility: private");
    eprintln!();
    eprintln!("  Generated API key: {key_preview}...");
    eprintln!("  Save this key -- it will not be shown again.");
    eprintln!("  (Full key written to ~/.cortex/cortex.token)");
    eprintln!();
    eprintln!("  Default team id: {default_team_id}");
    eprintln!();
    eprintln!("  Migration complete. Restart daemon: cortex serve");
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

fn copy_if_changed(src: &Path, dest: &Path) -> Result<(), String> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => {
            existing != fs::read(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(format!("Cannot read {}: {err}", dest.display())),
    };

    if needs_copy {
        fs::copy(src, dest)
            .map_err(|e| format!("Cannot copy {} to {}: {e}", src.display(), dest.display()))?;
    }

    Ok(())
}

fn stable_mcp_binary_path() -> String {
    let current = PathBuf::from(current_exe_path());
    let installed = auth::cortex_dir().join("bin").join(if cfg!(windows) {
        "cortex.exe"
    } else {
        "cortex"
    });

    if current == installed {
        return installed.to_string_lossy().to_string();
    }

    if let Some(parent) = installed.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!(
                "  [!!] Failed to create stable MCP binary dir {}: {}",
                parent.display(),
                err
            );
            return current.to_string_lossy().to_string();
        }
    }

    if let Err(err) = copy_if_changed(&current, &installed) {
        eprintln!("  [!!] Failed to refresh stable MCP binary: {err}");
        return current.to_string_lossy().to_string();
    }

    installed.to_string_lossy().to_string()
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

    // Claude Code
    if let Some(config_path) = find_claude_code_config() {
        found.push(DetectedTool {
            name: "Claude Code",
            agent_name: "claude",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    } else if command_exists("claude") {
        found.push(DetectedTool {
            name: "Claude Code",
            agent_name: "claude",
            config_path: None,
            config_method: ConfigMethod::CliCommand {
                program: "claude",
                args: &["mcp", "add", "cortex", "-s", "user", "--"],
            },
        });
    }

    // Claude Desktop
    if let Some(config_path) = find_claude_desktop_config() {
        found.push(DetectedTool {
            name: "Claude Desktop",
            agent_name: "claude",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    // Codex
    if let Some(config_path) = find_codex_config() {
        found.push(DetectedTool {
            name: "Codex CLI",
            agent_name: "codex",
            config_path: Some(config_path),
            config_method: ConfigMethod::TomlMerge,
        });
    } else if command_exists("codex") {
        found.push(DetectedTool {
            name: "Codex CLI",
            agent_name: "codex",
            config_path: None,
            config_method: ConfigMethod::CliCommand {
                program: "codex",
                args: &["mcp", "add", "cortex", "--"],
            },
        });
    }

    // Cursor
    if let Some(config_path) = find_cursor_config() {
        found.push(DetectedTool {
            name: "Cursor",
            agent_name: "cursor",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    // Windsurf
    if let Some(config_path) = find_windsurf_config() {
        found.push(DetectedTool {
            name: "Windsurf",
            agent_name: "windsurf",
            config_path: Some(config_path),
            config_method: ConfigMethod::JsonMerge,
        });
    }

    found
}

fn find_claude_desktop_config() -> Option<PathBuf> {
    find_first_config_path(claude_desktop_config_paths())
}

fn find_claude_code_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    find_existing_config(home.join(".claude").join("settings.json"))
}

fn find_codex_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    find_existing_config(home.join(".codex").join("config.toml"))
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
    find_existing_config(home.join(".cursor").join("mcp.json"))
}

fn find_windsurf_config() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    find_existing_config(home.join(".windsurf").join("mcp.json"))
}

fn find_first_config_path(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find_map(find_existing_config)
}

fn find_existing_config(path: PathBuf) -> Option<PathBuf> {
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
            match merge_mcp_config(config_path, cortex_exe, tool.agent_name) {
                Ok(action) => StepResult::Ok(action),
                Err(e) => StepResult::Warn(format!("Auto-config failed: {e}. Configure manually.")),
            }
        }
        ConfigMethod::TomlMerge => {
            let Some(config_path) = &tool.config_path else {
                return StepResult::Fail("No config path".into());
            };
            match merge_toml_config(config_path, cortex_exe, tool.agent_name) {
                Ok(action) => StepResult::Ok(action),
                Err(e) => StepResult::Warn(format!("Auto-config failed: {e}. Configure manually.")),
            }
        }
        ConfigMethod::CliCommand { program, args } => {
            match run_mcp_add(program, args, cortex_exe, tool.agent_name) {
                Ok(()) => StepResult::Ok("Registered via CLI".into()),
                Err(e) => StepResult::Warn(format!(
                    "CLI failed: {e}. Run manually: {} {} {cortex_exe} mcp --agent {}",
                    program,
                    args.join(" "),
                    tool.agent_name
                )),
            }
        }
        ConfigMethod::Manual(instructions) => {
            StepResult::Ok(format!("Manual setup needed: {instructions}"))
        }
    }
}

/// Merge a Cortex MCP server entry into a JSON config file.
/// Reads existing config, adds/updates the "cortex" entry under "mcpServers",
/// writes back. Preserves all existing config.
fn merge_mcp_config(
    config_path: &Path,
    cortex_exe: &str,
    agent_name: &str,
) -> Result<String, String> {
    let original: serde_json::Value = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Cannot read {}: {e}", config_path.display()))?;
        serde_json::from_str(&content)
            .map_err(|e| format!("Invalid JSON in {}: {e}", config_path.display()))?
    } else {
        serde_json::json!({})
    };
    let mut config = original.clone();

    let mcp_servers = config
        .as_object_mut()
        .ok_or("Config is not a JSON object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));

    let exe_path = PathBuf::from(cortex_exe).to_string_lossy().to_string();
    let desired_registration = serde_json::json!({
        "command": exe_path,
        "args": ["mcp", "--agent", agent_name]
    });
    mcp_servers
        .as_object_mut()
        .ok_or("mcpServers is not a JSON object")?
        .insert("cortex".to_string(), desired_registration);

    let action = if config == original {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };

    if config != original {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        let output = serde_json::to_string_pretty(&config)
            .map_err(|e| format!("JSON serialize failed: {e}"))?;
        fs::write(config_path, output)
            .map_err(|e| format!("Cannot write {}: {e}", config_path.display()))?;
    }

    Ok(format!("{action} at {}", config_path.display()))
}

fn merge_toml_config(
    config_path: &Path,
    cortex_exe: &str,
    agent_name: &str,
) -> Result<String, String> {
    let original: toml::Value = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("Cannot read {}: {e}", config_path.display()))?;
        toml::from_str(&content)
            .map_err(|e| format!("Invalid TOML in {}: {e}", config_path.display()))?
    } else {
        toml::Value::Table(Default::default())
    };
    let mut config = original.clone();

    let root = config.as_table_mut().ok_or("Config is not a TOML table")?;
    let servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let servers_table = servers
        .as_table_mut()
        .ok_or("mcp_servers is not a TOML table")?;

    let mut server = toml::map::Map::new();
    server.insert(
        "command".into(),
        toml::Value::String(PathBuf::from(cortex_exe).to_string_lossy().to_string()),
    );
    server.insert(
        "args".into(),
        toml::Value::Array(
            ["mcp", "--agent", agent_name]
                .into_iter()
                .map(|value| toml::Value::String(value.to_string()))
                .collect(),
        ),
    );
    servers_table.insert("cortex".into(), toml::Value::Table(server));

    let action = if config == original {
        "Already configured"
    } else if config_path.exists() {
        "Updated configuration"
    } else {
        "Configured"
    };

    if config != original {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
        }
        let output =
            toml::to_string_pretty(&config).map_err(|e| format!("TOML serialize failed: {e}"))?;
        fs::write(config_path, output)
            .map_err(|e| format!("Cannot write {}: {e}", config_path.display()))?;
    }

    Ok(format!("{action} at {}", config_path.display()))
}

fn run_mcp_add(
    program: &str,
    args: &[&str],
    cortex_exe: &str,
    agent_name: &str,
) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .args([cortex_exe, "mcp", "--agent", agent_name])
        .output()
        .map_err(|e| format!("Failed to run {program} CLI: {e}"))?;

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

// ─── Step 4: Check daemon availability ──────────────────────────────────────

async fn step_daemon() -> StepResult {
    let port = daemon_port();
    if is_daemon_healthy().await {
        return StepResult::Ok(format!("Daemon already running on :{port}"));
    }

    StepResult::Warn(format!(
        "No daemon is running on :{port}. Start Cortex from Control Center or let your client launch `cortex mcp --agent <name>`."
    ))
}

async fn is_daemon_healthy() -> bool {
    let paths = auth::CortexPaths::resolve();
    crate::daemon_lifecycle::daemon_healthy(&paths).await
}

// ─── Step 5: Verify ─────────────────────────────────────────────────────────

async fn step_verify() -> StepResult {
    if !is_daemon_healthy().await {
        return StepResult::Warn(
            "Skipped live verification because no daemon is currently running. Start Cortex from Control Center or `cortex mcp --agent <name>`, then rerun setup if you want a round-trip check."
                .into(),
        );
    }

    let token = match auth::read_token() {
        Some(t) => t,
        None => return StepResult::Fail("No auth token found".into()),
    };

    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return StepResult::Fail(format!("HTTP client error: {e}")),
    };

    // Store a test memory
    let store_resp = client
        .post(daemon_url("/store"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Cortex-Request", "true")
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
            ));
        }
        Err(e) => return StepResult::Fail(format!("Cannot reach daemon: {e}")),
    }

    // Recall it back
    let recall_resp = client
        .get(daemon_url("/recall"))
        .header("Authorization", format!("Bearer {token}"))
        .header("X-Cortex-Request", "true")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("cortex_setup_{name}_{unique}"))
    }

    #[test]
    fn merge_mcp_config_preserves_explicit_agent_args() {
        let root = temp_test_dir("json_merge");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("mcp.json");

        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();

        let config: Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            config["mcpServers"]["cortex"]["args"],
            serde_json::json!(["mcp", "--agent", "cursor"])
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_mcp_config_is_idempotent_for_existing_registration() {
        let root = temp_test_dir("json_merge_idempotent");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("mcp.json");

        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();
        let first = fs::read_to_string(&config_path).unwrap();
        merge_mcp_config(&config_path, "/tmp/cortex", "cursor").unwrap();
        let second = fs::read_to_string(&config_path).unwrap();

        assert_eq!(first, second);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_toml_config_writes_mcp_servers_without_clobbering_other_values() {
        let root = temp_test_dir("toml_merge");
        fs::create_dir_all(&root).unwrap();
        let config_path = root.join("config.toml");
        fs::write(
            &config_path,
            r#"
title = "Codex"

[other]
enabled = true
"#,
        )
        .unwrap();

        merge_toml_config(&config_path, "/tmp/cortex", "codex").unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            config
                .get("mcp_servers")
                .and_then(|value| value.get("cortex"))
                .and_then(|value| value.get("args"))
                .and_then(|value| value.as_array())
                .map(|values| values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()),
            Some(vec!["mcp", "--agent", "codex"])
        );
        assert_eq!(
            config
                .get("other")
                .and_then(|value| value.get("enabled"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_detection_helpers_accept_existing_parent_directories() {
        let root = temp_test_dir("path_detection");
        let claude = root.join(".claude").join("settings.json");
        let codex = root.join(".codex").join("config.toml");
        let cursor = root.join(".cursor").join("mcp.json");

        fs::create_dir_all(claude.parent().unwrap()).unwrap();
        fs::create_dir_all(codex.parent().unwrap()).unwrap();
        fs::create_dir_all(cursor.parent().unwrap()).unwrap();

        assert!(find_existing_config(claude).is_some());
        assert!(find_existing_config(codex).is_some());
        assert!(find_existing_config(cursor).is_some());
        assert!(find_first_config_path(vec![
            root.join(".missing"),
            root.join(".cursor").join("mcp.json"),
        ])
        .is_some());

        let _ = fs::remove_dir_all(&root);
    }
}
