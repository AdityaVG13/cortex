use super::path::*;
use super::{INDEXER_MAX_CONFIG_BYTES, capture_file, skip_unusable_source};
use rusqlite::Connection;
use serde::Deserialize;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct SourcesConfig {
    #[serde(default)]
    source: Vec<CustomSource>,
}
#[derive(Debug, Deserialize)]
struct CustomSource {
    path: String,
    #[serde(default = "default_glob")]
    glob: String,
    #[serde(default)]
    recursive: bool,
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

fn load_custom_sources(home: &Path) -> Result<Vec<CustomSource>, String> {
    let path = home.join(".cortex").join("sources.toml");
    if relative_has_symlink(home, &path)? {
        return Err("source_symlink_requires_explicit_path".into());
    }
    match crate::auth::open_nofollow(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound || is_symlink_open_error(&err) => {}
        Err(err) => return Err(err.to_string()),
        Ok(file) => {
            if !opened_under_root(&file, home)? {
                return Err("source_symlink_requires_explicit_path".into());
            }
            let mut content = String::new();
            file.take(INDEXER_MAX_CONFIG_BYTES + 1)
                .read_to_string(&mut content)
                .map_err(|err| err.to_string())?;
            if content.len() as u64 > INDEXER_MAX_CONFIG_BYTES {
                return Err("source_config_byte_limit".into());
            }
            return toml::from_str::<SourcesConfig>(&content)
                .map(|cfg| cfg.source)
                .map_err(|err| format!("invalid_source_config: {err}"));
        }
    }
    Ok(std::env::var("CORTEX_EXTRA_SOURCES")
        .unwrap_or_default()
        .split(';')
        .filter(|p| !p.is_empty())
        .map(|p| CustomSource {
            path: p.into(),
            glob: "*".into(),
            recursive: false,
        })
        .collect())
}

fn resolve_listed_source(raw: &str, home: &Path) -> PathBuf {
    let expanded = expand_tilde(raw);
    if expanded.is_absolute() {
        expanded
    } else {
        home.join(expanded)
    }
}

fn normalize_listed_components(path: &Path) -> Result<PathBuf, String> {
    let abs = lexical_absolute(path).map_err(source_unavailable)?;
    let mut out = PathBuf::new();
    for component in abs.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => out.push(component),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    return Err("source_outside_home".into());
                }
            }
            std::path::Component::Normal(_) => out.push(component),
        }
    }
    Ok(out)
}

fn listed_stays_under_home(listed: &Path, home: &Path, root: &Path) -> Result<bool, String> {
    let normalized = normalize_listed_components(listed)?;
    Ok(normalized.starts_with(home) || normalized.starts_with(root))
}

pub(super) fn index_custom_sources(
    conn: &mut Connection,
    home: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    let sources = load_custom_sources(home)?;
    let root = home.canonicalize().map_err(|err| err.to_string())?;
    let mut total = 0;
    for src in &sources {
        let listed = resolve_listed_source(&src.path, home);
        if !listed_stays_under_home(&listed, home, &root)? {
            return Err("source_outside_home".into());
        }
        // Same confine as automatic `.claude/state.md`: a planted alias is
        // skipped, never canonicalize-followed onto another file.
        if relative_has_symlink(home, &listed)? || relative_has_symlink(&root, &listed)? {
            continue;
        }
        let meta = match fs::symlink_metadata(&listed) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Err(source_unavailable(err));
            }
            Err(err) => return Err(err.to_string()),
            Ok(meta) => meta,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            total += index_directory(conn, &listed, &root, src, owner_id, home)?;
        } else {
            match capture_file(conn, &listed, owner_id, false, Some(home)) {
                Err(err) if skip_unusable_source(&err) => {}
                Err(err) => return Err(err),
                Ok(_) => total += 1,
            }
        }
    }
    Ok(total)
}

fn index_directory(
    conn: &mut Connection,
    dir: &Path,
    root: &Path,
    src: &CustomSource,
    owner_id: Option<i64>,
    home: &Path,
) -> Result<usize, String> {
    if relative_has_symlink(home, dir)? || relative_has_symlink(root, dir)? {
        return Ok(0);
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    entries.sort_by_key(|entry| entry.path());
    let mut count = 0;
    for entry in entries {
        let kind = entry.file_type().map_err(|err| err.to_string())?;
        // Do not follow directory aliases or cycles during recursive discovery.
        // Skip the alias; do not abort siblings the way automatic intake skips.
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if !listed_stays_under_home(&path, home, root)? {
            return Err("source_outside_home".into());
        }
        if kind.is_dir() {
            if src.recursive {
                count += index_directory(conn, &path, root, src, owner_id, home)?;
            }
        } else if matches_glob(&path, &src.glob) {
            match capture_file(conn, &path, owner_id, false, Some(home)) {
                Err(err) if skip_unusable_source(&err) => {}
                Err(err) => return Err(err),
                Ok(_) => count += 1,
            }
        }
    }
    Ok(count)
}

fn matches_glob(path: &Path, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if let Some(ext_pattern) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{ext_pattern}"));
    }
    name == pattern
}
