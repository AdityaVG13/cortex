// SPDX-License-Identifier: MIT
//! Workspace-level helpers shared by boot compilation and indexing.

use std::env;

/// Derive the Claude Code project folder slug from the current working directory.
/// Claude encodes paths as e.g. `C--Users-jane-cortex` for `C:\Users\jane\cortex`.
pub(crate) fn claude_project_slug() -> Option<String> {
    let cwd = env::current_dir().ok()?;
    let canonical = cwd.to_string_lossy().to_string();
    let slug = canonical.replace(['\\', ':'], "-");
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}
