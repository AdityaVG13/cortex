// SPDX-License-Identifier: MIT
use std::env;
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
