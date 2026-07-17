// SPDX-License-Identifier: MIT
use super::types::StepResult;
use crate::auth;
use crate::db;
use std::fs;
use std::path::{Path, PathBuf};
pub(crate) fn daemon_port() -> u16 {
    auth::CortexPaths::resolve().port
}
pub(crate) fn daemon_base_url() -> String {
    format!("http://localhost:{}", daemon_port())
}
pub(crate) fn daemon_url(path: &str) -> String {
    format!("{}{}", daemon_base_url(), path)
}
pub(crate) fn rollback_team_setup(conn: &rusqlite::Connection) {
    let _ = conn.execute_batch("ROLLBACK");
}
pub(crate) fn persist_team_owner_token(paths: &auth::CortexPaths, owner_key: &str) -> Result<(), String> {
    auth::try_write_token_for(paths, owner_key)
}
pub(crate) fn restore_previous_token(paths: &auth::CortexPaths, previous_token: Option<Vec<u8>>) {
    match previous_token {
        Some(contents) => {
            if let Some(parent) = paths.token.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = auth::write_secret_file(&paths.token, &contents);
        }
        None => {
            let _ = fs::remove_file(&paths.token);
        }
    }
}
pub(crate) fn print_step(num: usize, name: &str, result: &StepResult) {
    eprintln!("  {} Step {}: {} -- {}", result.icon(), num, name, result.message());
}
fn current_exe_path() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "cortex".to_string())
}
pub(crate) fn copy_if_changed(src: &Path, dest: &Path) -> Result<(), String> {
    let needs_copy = match fs::read(dest) {
        Ok(existing) => existing != fs::read(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(format!("Cannot read {}: {err}", dest.display())),
    };
    if needs_copy {
        fs::copy(src, dest).map_err(|e| format!("Cannot copy {} to {}: {e}", src.display(), dest.display()))?;
    }
    Ok(())
}
pub(crate) fn stable_mcp_binary_path() -> String {
    let current = PathBuf::from(current_exe_path());
    let installed = auth::cortex_dir().join("bin").join(if cfg!(windows) { "cortex.exe" } else { "cortex" });
    if current == installed {
        return installed.to_string_lossy().to_string();
    }
    if let Some(parent) = installed.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("  [!!] Failed to create stable MCP binary dir {}: {}", parent.display(), err);
            return current.to_string_lossy().to_string();
        }
    }
    if let Err(err) = copy_if_changed(&current, &installed) {
        eprintln!("  [!!] Failed to refresh stable MCP binary: {err}");
        return current.to_string_lossy().to_string();
    }
    installed.to_string_lossy().to_string()
}
pub(crate) fn arg_value(args: &[String], key: &str) -> Option<String> {
    for (idx, arg) in args.iter().enumerate() {
        if arg == key {
            return args.get(idx + 1).cloned();
        }
    }
    None
}
pub(crate) fn collect_reembed_backlog_counts(db_path: &Path, model_key: &str) -> Option<(i64, i64)> {
    if !db_path.exists() {
        return None;
    }
    let conn = db::open(db_path).ok()?;
    db::configure(&conn).ok()?;
    let backlog_memories: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories m \
             WHERE m.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'memory' \
                     AND e.target_id = m.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            [model_key],
            |row| row.get(0),
        )
        .ok()?;
    let backlog_decisions: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM decisions d \
             WHERE d.status = 'active' \
               AND NOT EXISTS (\
                   SELECT 1 FROM embeddings e \
                   WHERE e.target_type = 'decision' \
                     AND e.target_id = d.id \
                     AND LOWER(COALESCE(e.model, '')) = ?1\
               )",
            [model_key],
            |row| row.get(0),
        )
        .ok()?;
    Some((backlog_memories, backlog_decisions))
}
