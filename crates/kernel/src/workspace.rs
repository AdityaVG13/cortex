use std::env;
use std::path::Path;

pub fn claude_project_slug() -> Option<String> {
    encode_claude_project_slug(&env::current_dir().ok()?)
}

/// Claude Code stores per-project files under `.claude/projects/<slug>/`.
/// The slug is the working directory with every path separator turned into
/// a hyphen (`/Users/x/repo` → `-Users-x-repo`). Leaving `/` in place makes
/// `Path::join` treat a Unix cwd as absolute, drop the `.claude/projects`
/// prefix, and walk the working tree (or `cwd/memory`) instead.
pub fn encode_claude_project_slug(cwd: &Path) -> Option<String> {
    let slug = cwd.to_string_lossy().replace(['/', '\\', ':'], "-");
    if slug.is_empty() { None } else { Some(slug) }
}
