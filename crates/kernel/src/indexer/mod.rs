use crate::runtime::observation::{self, ObservationEvent, ObservationReceipt};
use crate::workspace::claude_project_slug;
use rusqlite::Connection;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Each registered file commits independently; earlier receipts remain replayable on error.
pub fn index_all(conn: &mut Connection, home: &Path, owner_id: Option<i64>) -> Result<usize, String> {
    Ok(index_state_file(conn, home, owner_id)?
        + index_memory_files(conn, home, owner_id)?
        + index_custom_sources(conn, home, owner_id)?)
}
fn source_unavailable(err: io::Error) -> String {
    format!("source_unavailable: {err}")
}

fn index_discovered_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    match crate::auth::open_nofollow(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.to_string()),
        Ok(_) => {}
    }
    capture_file(conn, path, owner_id, false)?;
    Ok(1)
}

/// Capture a registered UTF-8 file without interpreting or truncating its content.
/// Key: file:<canonical UTF-8 absolute path>. Owner comes from the trusted host.
pub fn index_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
) -> Result<ObservationReceipt, String> {
    capture_file(conn, path, owner_id, true)
}

fn capture_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
    follow: bool,
) -> Result<ObservationReceipt, String> {
    // Operator-chosen source (`follow`): canonicalize once, then open that
    // target with O_NOFOLLOW so a later symlink swap cannot redirect the fd.
    // Auto-discovered files never follow: a planted alias must fail closed.
    let resolved = if follow {
        path.canonicalize().map_err(source_unavailable)?
    } else {
        path.to_path_buf()
    };
    let file = crate::auth::open_nofollow(&resolved).map_err(source_unavailable)?;
    let before = file.metadata().map_err(|err| err.to_string())?;
    if !before.is_file() {
        return Err("source_not_regular_file".into());
    }
    let key_path = if follow {
        resolved
    } else {
        path.canonicalize().map_err(source_unavailable)?
    };
    let key = format!("file:{}", key_path.to_str().ok_or("source_path_not_utf8")?);
    let principal = owner_id.map_or_else(|| "local".into(), |id| format!("user:{id}"));
    let limit = observation::source_capture_limit(conn, &principal, &key)?
        .min(INDEXER_MAX_FILE_BYTES as usize);
    if before.len() > limit as u64 {
        return Err("capture_byte_limit".into());
    }
    let modified = before.modified().map_err(|err| err.to_string())?;
    let mut bytes = Vec::new();
    (&file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| err.to_string())?;
    if bytes.len() > limit {
        return Err("capture_byte_limit".into());
    }
    let after = fs::metadata(&key_path).map_err(|err| format!("source_changed: {err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err("source_changed".into());
        }
    }
    if before.len() != after.len()
        || after.modified().map_err(|err| err.to_string())? != modified
        || bytes.len() as u64 != before.len()
    {
        return Err("source_changed".into());
    }
    let observed_at = chrono::DateTime::<chrono::Utc>::from(modified)
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            before.dev(),
            before.ino(),
            before.ctime(),
            before.ctime_nsec()
        )
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", before.created().map_err(|err| err.to_string())?);
    let generation = format!("file/1:{identity}:{observed_at}:{digest}");
    let text = String::from_utf8(bytes).map_err(|_| "source_not_utf8")?;
    observation::capture_registered(
        conn,
        &principal,
        &key,
        &generation,
        ObservationEvent {
            event_key: "document".into(),
            text,
            observed_at: Some(observed_at),
        },
    )
}
fn index_state_file(
    conn: &mut Connection,
    home: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    index_discovered_file(conn, &home.join(".claude").join("state.md"), owner_id)
}

fn index_memory_files(
    conn: &mut Connection,
    home: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    let Some(slug) = claude_project_slug() else {
        return Ok(0);
    };
    let dir = home
        .join(".claude")
        .join("projects")
        .join(slug)
        .join("memory");
    let mut paths = match fs::read_dir(&dir) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.to_string()),
        Ok(entries) => entries
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?,
    };
    paths.sort();
    let mut count = 0;
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            count += index_discovered_file(conn, &path, owner_id)?;
        }
    }
    Ok(count)
}

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
    match crate::auth::open_nofollow(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.to_string()),
        Ok(file) => {
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
fn index_custom_sources(
    conn: &mut Connection,
    home: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    let sources = load_custom_sources(home)?;
    let root = home.canonicalize().map_err(|err| err.to_string())?;
    let mut total = 0;
    for src in &sources {
        let resolved = expand_tilde(&src.path)
            .canonicalize()
            .map_err(|err| format!("source_unavailable: {err}"))?;
        if !resolved.starts_with(&root) {
            return Err("source_outside_home".into());
        }
        if resolved.is_dir() {
            total += index_directory(conn, &resolved, &root, src, owner_id)?;
        } else {
            index_file(conn, &resolved, owner_id)?;
            total += 1;
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
) -> Result<usize, String> {
    let mut entries = fs::read_dir(dir)
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    entries.sort_by_key(|entry| entry.path());
    let mut count = 0;
    for entry in entries {
        let kind = entry.file_type().map_err(|err| err.to_string())?;
        // Do not follow directory aliases or cycles during recursive discovery.
        if kind.is_symlink() {
            return Err("source_symlink_requires_explicit_path".into());
        }
        let path = entry.path().canonicalize().map_err(|err| err.to_string())?;
        if !path.starts_with(root) {
            return Err("source_outside_home".into());
        }
        if kind.is_dir() {
            if src.recursive {
                count += index_directory(conn, &path, root, src, owner_id)?;
            }
        } else if matches_glob(&path, &src.glob) {
            index_file(conn, &path, owner_id)?;
            count += 1;
        }
    }
    Ok(count)
}
/// Hard ceiling for exact file capture; larger files fail without a partial receipt.
pub const INDEXER_MAX_FILE_BYTES: u64 = 1024 * 1024;
/// `sources.toml` is a small operator config loaded automatically on index.
pub const INDEXER_MAX_CONFIG_BYTES: u64 = 64 * 1024;

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
