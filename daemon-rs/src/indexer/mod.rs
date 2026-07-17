use crate::workspace::claude_project_slug;
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
const STATE_SECTIONS: &[&str] = &["## What Was Done", "## Next Session", "## Pending", "## Known Issues"];
pub fn index_all(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let mut total = 0;
    total += index_state_file(conn, home, owner_id);
    total += index_memory_files(conn, home, owner_id);
    total += index_custom_sources(conn, home, owner_id);
    total
}
fn upsert_memory(conn: &Connection, text: &str, source: &str, mem_type: &str, agent: &str, owner_id: Option<i64>) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let existing: Option<i64> = conn
        .query_row("SELECT id FROM memories WHERE source = ? AND status = 'active'", [source], |row| row.get(0))
        .ok();
    if let Some(id) = existing {
        let _ = conn.execute("UPDATE memories SET text = ?, updated_at = datetime('now') WHERE id = ?", rusqlite::params![text, id]);
        let _ = conn.execute("DELETE FROM embeddings WHERE target_type = 'memory' AND target_id = ?", [id]);
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
fn index_memory_files(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let slug = match claude_project_slug() {
        Some(s) => s,
        None => return 0,
    };
    let mem_dir = home.join(".claude").join("projects").join(slug).join("memory");
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
            let name = fm
                .get("name")
                .cloned()
                .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string());
            let mem_type = fm.get("type").cloned().unwrap_or_else(|| "memory".to_string());
            let desc = fm.get("description").cloned().unwrap_or_default();
            let body_preview: String = body.chars().take(500).collect();
            let text = if !desc.is_empty() {
                format!("[{name}] ({mem_type}) {desc}\n{body_preview}")
            } else {
                format!("[{name}] ({mem_type})\n{body_preview}")
            };
            let source = format!("memory::{}", path.file_name().unwrap_or_default().to_string_lossy());
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
fn default_mem_type() -> String {
    "custom".to_string()
}
fn default_glob() -> String {
    "*.md".to_string()
}
fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}
fn load_custom_sources(home: &Path) -> Vec<CustomSource> {
    let config_path = home.join(".cortex").join("sources.toml");
    if config_path.exists() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if let Ok(cfg) = toml::from_str::<SourcesConfig>(&content) {
                return cfg.source;
            }
            eprintln!("[indexer] failed to parse {}", config_path.display());
        }
    }
    if let Ok(val) = std::env::var("CORTEX_EXTRA_SOURCES") {
        return val
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|p| CustomSource {
                name: Path::new(p).file_name().unwrap_or_default().to_string_lossy().to_string(),
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
fn index_custom_sources(conn: &Connection, home: &Path, owner_id: Option<i64>) -> usize {
    let sources = load_custom_sources(home);
    let mut total = 0;
    let home_root = home.canonicalize().ok();
    for src in &sources {
        let resolved = expand_tilde(&src.path);
        if !resolved.exists() {
            continue;
        }
        if let Some(root) = home_root.as_ref() {
            let Ok(canonical) = resolved.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(root) {
                eprintln!("[cortex] skipping custom source outside Cortex home: {}", resolved.display());
                continue;
            }
        }
        if resolved.is_dir() {
            total += index_directory(conn, &resolved, src, owner_id);
        } else if resolved.is_file() {
            total += index_single_file(conn, &resolved, src, owner_id);
        }
    }
    total
}
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
fn index_single_file(conn: &Connection, path: &Path, src: &CustomSource, owner_id: Option<i64>) -> usize {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let file_stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let text = if src.truncate > 0 { content.chars().take(src.truncate).collect() } else { content };
    let source = format!("{}::{}", src.name, file_stem);
    if upsert_memory(conn, &text, &source, &src.mem_type, "indexer", owner_id) {
        1
    } else {
        0
    }
}
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
mod tests;
