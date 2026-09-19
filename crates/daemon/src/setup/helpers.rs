use super::types::StepResult;
use crate::auth;
use std::fs;
use std::path::{Path, PathBuf};

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
    let src_meta = fs::metadata(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?;
    let needs_copy = match fs::metadata(dest) {
        Ok(dest_meta) if dest_meta.len() == src_meta.len() => {
            fs::read(dest).map_err(|e| format!("Cannot read {}: {e}", dest.display()))?
                != fs::read(src).map_err(|e| format!("Cannot read {}: {e}", src.display()))?
        }
        Ok(_) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => return Err(format!("Cannot read {}: {err}", dest.display())),
    };
    if needs_copy {
        // Stage beside the destination then rename. In-place `fs::copy`
        // truncates the installed inode first; a crash or a live MCP
        // process mapped to `~/.cortex/bin/cortex` then observes a
        // truncated image (Unix SIGBUS / Windows sharing failure).
        let tmp = dest.with_extension("tmp");
        if let Err(err) = fs::copy(src, &tmp) {
            let _ = fs::remove_file(&tmp);
            return Err(format!("Cannot copy {} to {}: {err}", src.display(), tmp.display()));
        }
        if let Err(rename_err) = fs::rename(&tmp, dest) {
            // Unix rename replaces. Windows refuses to rename over an
            // existing dest; remove then rename so we never truncate the
            // live inode in place. A locked running image fails closed.
            let replaced = fs::remove_file(dest).and_then(|_| fs::rename(&tmp, dest));
            if let Err(err) = replaced {
                let _ = fs::remove_file(&tmp);
                return Err(format!("Cannot install {} from {} (rename: {rename_err}; replace: {err})", dest.display(), src.display()));
            }
        }
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
    crate::cli::parse_flag_value(args, key)
}
#[allow(dead_code)]
pub(crate) fn collect_reembed_backlog_counts(_db_path: &Path, _model_key: &str) -> Option<(i64, i64)> {
    Some((0, 0))
}
