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

fn is_symlink_open_error(err: &io::Error) -> bool {
    #[cfg(unix)]
    {
        if err.raw_os_error() == Some(libc::ELOOP) {
            return true;
        }
    }
    err.kind() == io::ErrorKind::InvalidInput
}

fn lexical_absolute(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

/// Path of the opened vnode. Intermediate symlinks are already resolved on
/// this fd; comparing it to a trusted root closes check-then-open follows.
fn normalize_fd_path(path: PathBuf) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn fd_path(file: &fs::File) -> io::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::io::AsRawFd;
        let mut buf = [0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated absolute path into `buf`.
        let rc = unsafe {
            libc::fcntl(
                file.as_raw_fd(),
                libc::F_GETPATH,
                buf.as_mut_ptr() as *mut libc::c_char,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let raw = std::str::from_utf8(&buf[..len])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path not utf8"))?;
        Ok(normalize_fd_path(PathBuf::from(raw)))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::io::AsRawFd;
        fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map(normalize_fd_path)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::HANDLE;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
        };
        let handle = file.as_raw_handle() as HANDLE;
        let mut buf = vec![0u16; 1024];
        // SAFETY: `handle` is a live `File`; `buf` is writable wide storage.
        let n = unsafe {
            GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), buf.len() as u32, VOLUME_NAME_DOS)
        };
        if n == 0 || (n as usize) > buf.len() {
            return Err(io::Error::last_os_error());
        }
        let raw = String::from_utf16(&buf[..n as usize])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "path not utf16"))?;
        Ok(normalize_fd_path(PathBuf::from(raw)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "fd path unavailable",
        ))
    }
}

fn relative_has_symlink(root: &Path, path: &Path) -> Result<bool, String> {
    let Ok(rel) = path.strip_prefix(root) else {
        return Ok(false);
    };
    let mut cur = root.to_path_buf();
    for component in rel.components() {
        cur.push(component);
        match fs::symlink_metadata(&cur) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.to_string()),
            Ok(meta) if meta.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
        }
    }
    Ok(false)
}

fn opened_under_root(file: &fs::File, root: &Path) -> Result<bool, String> {
    let root = root.canonicalize().map_err(|err| err.to_string())?;
    // Unknown fd path: fail closed. Returning true here would let a planted
    // intermediate alias (home/.claude -> secret) pass after O_NOFOLLOW
    // follows that directory.
    let Ok(actual) = fd_path(file) else {
        return Ok(false);
    };
    let actual = actual.canonicalize().unwrap_or(actual);
    Ok(actual.starts_with(&root))
}

fn capture_key_path(file: &fs::File, path: &Path, follow: bool, confine: bool) -> Result<PathBuf, String> {
    if follow {
        return path.canonicalize().map_err(source_unavailable);
    }
    let expected = lexical_absolute(path).map_err(source_unavailable)?;
    let actual = fd_path(file).map_err(|_| "source_symlink_requires_explicit_path".to_string())?;
    let actual = actual.canonicalize().unwrap_or(actual);
    let expected = expected.canonicalize().unwrap_or(expected);
    if confine {
        return Ok(actual);
    }
    if actual != expected {
        return Err("source_symlink_requires_explicit_path".into());
    }
    Ok(expected)
}

fn open_capture(path: &Path, follow: bool) -> Result<(fs::File, PathBuf), String> {
    match crate::auth::open_nofollow(path) {
        Ok(file) => Ok((file, path.to_path_buf())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            if follow {
                Err(source_unavailable(err))
            } else {
                Err("source_not_found".into())
            }
        }
        Err(err) if is_symlink_open_error(&err) => {
            if !follow {
                // Last-component alias: skip automatic intake instead of
                // aborting index_all with source_unavailable.
                return Err("source_symlink_requires_explicit_path".into());
            }
            let target = fs::read_link(path).map_err(source_unavailable)?;
            let resolved = if target.is_absolute() {
                target
            } else {
                path.parent().unwrap_or_else(|| Path::new(".")).join(target)
            };
            let file = crate::auth::open_nofollow(&resolved).map_err(source_unavailable)?;
            Ok((file, resolved))
        }
        Err(err) => Err(source_unavailable(err)),
    }
}

fn skip_unusable_source(err: &str) -> bool {
    err == "source_not_found" || err == "source_symlink_requires_explicit_path"
}

fn index_discovered_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
    home: &Path,
) -> Result<usize, String> {
    match capture_file(conn, path, owner_id, false, Some(home)) {
        Err(err) if skip_unusable_source(&err) => Ok(0),
        Err(err) => Err(err),
        Ok(_) => Ok(1),
    }
}

/// Capture a registered UTF-8 file without interpreting or truncating its content.
/// Key: file:<canonical UTF-8 absolute path>. Owner comes from the trusted host.
pub fn index_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
) -> Result<ObservationReceipt, String> {
    capture_file(conn, path, owner_id, true, None)
}

/// Intake a registered canonical path without following a planted alias.
/// Inventory bootstrap uses this after a snapshot so a swapped symlink cannot
/// redirect canonicalize-then-open onto another file.
pub(crate) fn index_file_nofollow(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
) -> Result<ObservationReceipt, String> {
    capture_file(conn, path, owner_id, false, None)
}

fn capture_file(
    conn: &mut Connection,
    path: &Path,
    owner_id: Option<i64>,
    follow: bool,
    confine: Option<&Path>,
) -> Result<ObservationReceipt, String> {
    // Open the given path with O_NOFOLLOW first. Canonicalize-then-open
    // follows a last-component symlink planted after a snapshot check.
    // Operator `follow` may take one readlink hop of that last component.
    if let Some(root) = confine {
        if relative_has_symlink(root, path)? {
            return Err("source_symlink_requires_explicit_path".into());
        }
    }
    let (file, opened_path) = open_capture(path, follow)?;
    if let Some(root) = confine {
        if !opened_under_root(&file, root)? {
            return Err("source_symlink_requires_explicit_path".into());
        }
    }
    let key_path = capture_key_path(&file, if follow { &opened_path } else { path }, follow, confine.is_some())?;
    if let Some(root) = confine {
        let root = root.canonicalize().map_err(|err| err.to_string())?;
        if !key_path.starts_with(&root) {
            return Err("source_symlink_requires_explicit_path".into());
        }
    }
    let before = file.metadata().map_err(|err| err.to_string())?;
    if !before.is_file() {
        return Err("source_not_regular_file".into());
    }
    let key = format!("file:{}", key_path.to_str().ok_or("source_path_not_utf8")?);
    let principal = owner_id.map_or_else(|| "local".into(), |id| format!("user:{id}"));
    let limit = file_capture_limit(conn, &principal, &key)?;
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
    let after = file
        .metadata()
        .map_err(|err| format!("source_changed: {err}"))?;
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
    index_discovered_file(conn, &home.join(".claude").join("state.md"), owner_id, home)
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
    if relative_has_symlink(home, &dir)? {
        return Ok(0);
    }
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
            count += index_discovered_file(conn, &path, owner_id, home)?;
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

fn index_custom_sources(
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
/// Hard ceiling for exact file capture; larger files fail without a partial receipt.
pub const INDEXER_MAX_FILE_BYTES: u64 = 1024 * 1024;
/// `sources.toml` is a small operator config loaded automatically on index.
pub const INDEXER_MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Effective file intake ceiling: registered grant, capped at the indexer hard limit.
pub(crate) fn file_capture_limit(
    conn: &Connection,
    principal: &str,
    source: &str,
) -> Result<usize, String> {
    Ok(observation::source_capture_limit(conn, principal, source)?
        .min(INDEXER_MAX_FILE_BYTES as usize))
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
    // Blank `''` is not NULL; `julianday('')` is NULL and the age predicate
    // never matches, so those rows skipped every decay. Fall through with
    // NULLIF(TRIM(...)) the same way aging/gc does.
    let mem_result = conn.execute(
        "UPDATE memories SET score = MAX(0.05, score * POWER(
            MIN(1.0, 0.95 + 0.005 * MIN(retrievals, 10)),
            CAST((julianday('now') - julianday(
                COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))
            )) AS REAL)
         ))
         WHERE status = 'active' AND score > 0.05 AND pinned = 0
           AND (julianday('now') - julianday(
                COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))
           )) > 1",
        [],
    );
    let dec_result = conn.execute(
        "UPDATE decisions SET score = MAX(0.05, score * POWER(
            MIN(1.0, 0.95 + 0.005 * MIN(retrievals, 10)),
            CAST((julianday('now') - julianday(
                COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))
            )) AS REAL)
         ))
         WHERE status = 'active' AND score > 0.05 AND pinned = 0
           AND (julianday('now') - julianday(
                COALESCE(NULLIF(TRIM(last_accessed), ''), NULLIF(TRIM(updated_at), ''), NULLIF(TRIM(created_at), ''))
           )) > 1",
        [],
    );
    mem_result.unwrap_or(0) + dec_result.unwrap_or(0)
}
