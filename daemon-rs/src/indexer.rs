//! Knowledge indexer: reads filesystem sources and upserts into memories table.
//!
//! **Core indexers** (always run): `~/.claude/state.md` and Claude Code project memory under
//! `~/.claude/projects/<cwd-slug>/memory`.
//!
//! **Custom sources** (opt-in): user-defined paths via `~/.cortex/sources.toml` or
//! `CORTEX_EXTRA_SOURCES` env var. See `config/sources.toml.example`.

use crate::compiler::claude_project_slug;
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const STATE_SECTIONS: &[&str] = &[
    "## What Was Done",
    "## Next Session",
    "## Pending",
    "## Known Issues",
];

/// Run core indexers always; custom sources from config if present.
pub fn index_all(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let mut total = 0;
    total += index_state_file(conn, home, owner_id);
    total += index_memory_files(conn, home, owner_id);
    total += index_custom_sources(conn, home, owner_id);
    total
}

/// Upsert a memory by source. If source exists, update text. Otherwise insert.
fn upsert_memory(conn: &Connection, text: &str, source: &str, mem_type: &str, agent: &str, owner_id: Option<i64>) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memories WHERE source = ? AND status = 'active'",
            [source],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        let _ = conn.execute(
            "UPDATE memories SET text = ?, updated_at = datetime('now') WHERE id = ?",
            rusqlite::params![text, id],
        );
        let _ = conn.execute(
            "DELETE FROM embeddings WHERE target_type = 'memory' AND target_id = ?",
            [id],
        );
    } else if let Some(oid) = owner_id {
        let _ = conn.execute(
            "INSERT INTO memories (text, source, type, source_agent, owner_id) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![text, source, mem_type, agent, oid],
        );
    } else {
        let _ = conn.execute(
            "INSERT INTO memories (text, source, type, source_agent) VALUES (?, ?, ?, ?)",
            rusqlite::params![text, source, mem_type, agent],
        );
    }

    true
}

// ── Source 1: state.md ──────────────────────────────────────────────────────

fn index_state_file(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let state_path = home.join(".claude").join("state.md");
    if !state_path.exists() {
        return 0;
    }

    let content = match fs::read_to_string(&state_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut count = 0;
    for section in STATE_SECTIONS {
        if let Some(text) = extract_section(&content, section) {
            let source = format!("state.md::{}", section.trim_start_matches("## "));
            if upsert_memory(conn, &text, &source, "state", "indexer", owner_id) {
                count += 1;
            }
        }
    }
    count
}

fn extract_section(markdown: &str, header: &str) -> Option<String> {
    let idx = markdown.find(header)?;
    let start = idx + header.len();
    let rest = &markdown[start..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    let text = rest[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

// ── Source 2: Memory files ──────────────────────────────────────────────────

fn index_memory_files(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let slug = match claude_project_slug() {
        Some(s) => s,
        None => return 0,
    };
    let mem_dir = home
        .join(".claude")
        .join("projects")
        .join(slug)
        .join("memory");
    if !mem_dir.exists() {
        return 0;
    }

    let mut count = 0;
    let entries = match fs::read_dir(&mem_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_name().and_then(|f| f.to_str()) == Some("MEMORY.md") {
            continue;
        }

        if let Ok(raw) = fs::read_to_string(&path) {
            let (fm, body) = parse_frontmatter(&raw);
            let name = fm.get("name").cloned().unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });
            let mem_type = fm
                .get("type")
                .cloned()
                .unwrap_or_else(|| "memory".to_string());
            let desc = fm.get("description").cloned().unwrap_or_default();

            let body_preview: String = body.chars().take(500).collect();
            let text = if !desc.is_empty() {
                format!("[{name}] ({mem_type}) {desc}\n{body_preview}")
            } else {
                format!("[{name}] ({mem_type})\n{body_preview}")
            };

            let source = format!(
                "memory::{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            if upsert_memory(conn, &text, &source, &mem_type, "indexer", owner_id) {
                count += 1;
            }
        }
    }
    count
}

fn parse_frontmatter(raw: &str) -> (HashMap<String, String>, String) {
    let mut fm = HashMap::new();
    let body;

    if let Some(rest) = raw.strip_prefix("---") {
        if let Some(end) = rest.find("---") {
            let yaml_block = &rest[..end];
            body = rest[end + 3..].trim().to_string();

            for line in yaml_block.lines() {
                if let Some(colon) = line.find(':') {
                    let key = line[..colon].trim().to_string();
                    let val = line[colon + 1..].trim().to_string();
                    fm.insert(key, val);
                }
            }
        } else {
            body = raw.to_string();
        }
    } else {
        body = raw.to_string();
    }

    (fm, body)
}

// ── Custom sources from config ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SourcesConfig {
    #[serde(default)]
    source: Vec<CustomSource>,
}

#[derive(Debug, Deserialize)]
struct CustomSource {
    name: String,
    path: String,
    #[serde(default = "default_mem_type")]
    mem_type: String,
    #[serde(default = "default_glob")]
    glob: String,
    #[serde(default)]
    truncate: usize,
    #[serde(default)]
    recursive: bool,
}

fn default_mem_type() -> String { "custom".to_string() }
fn default_glob() -> String { "*.md".to_string() }

/// Resolve `~` to the user's home directory.
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

/// Load custom source definitions from `~/.cortex/sources.toml`, falling back
/// to `CORTEX_EXTRA_SOURCES` env var (semicolon-separated directory paths).
fn load_custom_sources(home: &Path) -> Vec<CustomSource> {
    // Try sources.toml first
    let config_path = home.join(".cortex").join("sources.toml");
    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if let Ok(cfg) = toml::from_str::<SourcesConfig>(&content) {
                return cfg.source;
            }
            eprintln!("[indexer] failed to parse {}", config_path.display());
        }
    }

    // Fallback: CORTEX_EXTRA_SOURCES env var (semicolon-separated paths)
    if let Ok(val) = std::env::var("CORTEX_EXTRA_SOURCES") {
        return val
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|p| CustomSource {
                name: Path::new(p)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                path: p.to_string(),
                mem_type: "custom".to_string(),
                glob: "*".to_string(),
                truncate: 0,
                recursive: false,
            })
            .collect();
    }

    Vec::new()
}

/// Index all user-configured custom sources.
fn index_custom_sources(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let sources = load_custom_sources(home);
    let mut total = 0;

    for src in &sources {
        let resolved = expand_tilde(&src.path);
        if !resolved.exists() {
            continue;
        }

        if resolved.is_dir() {
            total += index_directory(conn, &resolved, src, owner_id);
        } else if resolved.is_file() {
            total += index_single_file(conn, &resolved, src, owner_id);
        }
    }
    total
}

/// Index all matching files in a directory.
fn index_directory(conn: &Connection, dir: &Path, src: &CustomSource, owner_id: Option<i64>) -> usize {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            if src.recursive {
                count += index_directory(conn, &path, src, owner_id);
            }
            continue;
        }

        if !matches_glob(&path, &src.glob) {
            continue;
        }

        count += index_single_file(conn, &path, src, owner_id);
    }
    count
}

/// Index a single file's content as a memory entry.
fn index_single_file(conn: &Connection, path: &Path, src: &CustomSource, owner_id: Option<i64>) -> usize {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let file_stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let text = if src.truncate > 0 {
        content.chars().take(src.truncate).collect()
    } else {
        content
    };

    let source = format!("{}::{}", src.name, file_stem);
    if upsert_memory(conn, &text, &source, &src.mem_type, "indexer", owner_id) {
        1
    } else {
        0
    }
}

/// Simple glob matching: supports `*` (any filename) and `*.ext` patterns.
fn matches_glob(path: &Path, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let name = match path.file_name().and_then(|n| n.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if let Some(ext_pattern) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{ext_pattern}"));
    }
    name == pattern
}

/// Collect resolved paths from custom sources config (used by compiler baseline).
pub fn custom_source_paths(home: &Path) -> Vec<PathBuf> {
    load_custom_sources(home)
        .iter()
        .map(|s| expand_tilde(&s.path))
        .filter(|p| p.exists())
        .collect()
}

// ── Ebbinghaus decay ────────────────────────────────────────────────────────

/// Apply Ebbinghaus-style forgetting curve to all active entries.
///
/// Formula: score = MAX(floor, score * POWER(decay_rate, days_since_last_touch))
///
/// Key improvements over simple 0.95^days:
///   - Uses last_accessed (recall time) not just updated_at (write time)
///   - Retrieval count strengthens durability: decay_rate = 0.95 + 0.005 * min(retrievals, 10)
///     → 0 recalls: 0.950/day (forgets fast)
///     → 5 recalls: 0.975/day (moderate retention)
///     → 10+ recalls: 1.000/day (permanent -- fully reinforced)
///   - Pinned entries are immune to decay
///   - Floor is 0.05 (not 0.1) to better separate stale from semi-stale
///
/// Also decays decisions table with same formula.
pub fn decay_pass(conn: &Connection) -> usize {
    let mem_result = conn.execute(
        "UPDATE memories SET score = MAX(0.05, score * POWER(
            MIN(1.0, 0.95 + 0.005 * MIN(retrievals, 10)),
            CAST((julianday('now') - julianday(
                COALESCE(last_accessed, updated_at, created_at)
            )) AS REAL)
         ))
         WHERE status = 'active' AND score > 0.05 AND pinned = 0
           AND (julianday('now') - julianday(
                COALESCE(last_accessed, updated_at, created_at)
           )) > 1",
        [],
    );

    let dec_result = conn.execute(
        "UPDATE decisions SET score = MAX(0.05, score * POWER(
            MIN(1.0, 0.95 + 0.005 * MIN(retrievals, 10)),
            CAST((julianday('now') - julianday(
                COALESCE(last_accessed, updated_at, created_at)
            )) AS REAL)
         ))
         WHERE status = 'active' AND score > 0.05 AND pinned = 0
           AND (julianday('now') - julianday(
                COALESCE(last_accessed, updated_at, created_at)
           )) > 1",
        [],
    );

    mem_result.unwrap_or(0) + dec_result.unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn index_all_empty_home_indexes_nothing() {
        let tmp = std::env::temp_dir().join(format!("cortex_ix_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        let n = index_all(&conn, tmp.as_path(), None);
        assert_eq!(n, 0);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn matches_glob_works() {
        assert!(super::matches_glob(Path::new("foo.md"), "*.md"));
        assert!(!super::matches_glob(Path::new("foo.rs"), "*.md"));
        assert!(super::matches_glob(Path::new("anything"), "*"));
        assert!(super::matches_glob(Path::new("data.jsonl"), "*.jsonl"));
    }

    #[test]
    fn expand_tilde_resolves_home() {
        let p = super::expand_tilde("~/test/path");
        assert!(p.components().count() > 2);
        assert!(!p.to_string_lossy().contains('~'));
    }

    #[test]
    fn index_custom_sources_from_toml() {
        let tmp = std::env::temp_dir().join(format!("cortex_cs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        // Set up: ~/.cortex/sources.toml pointing to a test directory
        let cortex_dir = tmp.join(".cortex");
        std::fs::create_dir_all(&cortex_dir).unwrap();

        let notes_dir = tmp.join("test-notes");
        std::fs::create_dir_all(&notes_dir).unwrap();
        std::fs::write(notes_dir.join("alpha.md"), "Alpha note content").unwrap();
        std::fs::write(notes_dir.join("beta.md"), "Beta note content").unwrap();
        std::fs::write(notes_dir.join("ignore.txt"), "Should be skipped").unwrap();

        let single_file = tmp.join("single.json");
        std::fs::write(&single_file, r#"{"key": "value"}"#).unwrap();

        let toml_content = format!(
            r#"
[[source]]
name = "notes"
path = "{}"
mem_type = "note"
glob = "*.md"

[[source]]
name = "config"
path = "{}"
mem_type = "config"
"#,
            notes_dir.to_string_lossy().replace('\\', "/"),
            single_file.to_string_lossy().replace('\\', "/"),
        );
        std::fs::write(cortex_dir.join("sources.toml"), &toml_content).unwrap();

        let conn = Connection::open_in_memory().unwrap();
        crate::db::initialize_schema(&conn).unwrap();

        let n = super::index_custom_sources(&conn, &tmp, None);
        // 2 .md files + 1 single json = 3
        assert_eq!(n, 3, "expected 3 indexed entries (2 md + 1 json)");

        // Verify sources are stored correctly
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE source LIKE 'notes::%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2, "expected 2 note memories");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memories WHERE source LIKE 'config::%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "expected 1 config memory");

        // Verify mem_type is set correctly
        let mem_type: String = conn
            .query_row("SELECT type FROM memories WHERE source LIKE 'notes::%' LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mem_type, "note");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn index_custom_sources_truncate() {
        let tmp = std::env::temp_dir().join(format!("cortex_tr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        let cortex_dir = tmp.join(".cortex");
        std::fs::create_dir_all(&cortex_dir).unwrap();

        let docs_dir = tmp.join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("long.md"), "A".repeat(5000)).unwrap();

        let toml_content = format!(
            r#"
[[source]]
name = "docs"
path = "{}"
mem_type = "doc"
glob = "*.md"
truncate = 100
"#,
            docs_dir.to_string_lossy().replace('\\', "/"),
        );
        std::fs::write(cortex_dir.join("sources.toml"), &toml_content).unwrap();

        let conn = Connection::open_in_memory().unwrap();
        crate::db::initialize_schema(&conn).unwrap();

        let n = super::index_custom_sources(&conn, &tmp, None);
        assert_eq!(n, 1);

        let text: String = conn
            .query_row("SELECT text FROM memories WHERE source = 'docs::long'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text.len(), 100, "text should be truncated to 100 chars");

        let _ = std::fs::remove_dir_all(&tmp);
    }

}
