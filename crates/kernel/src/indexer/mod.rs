use crate::db::LAST_TOUCHED_STAMP_SQL;
use crate::runtime::observation::{self, ObservationEvent, ObservationReceipt};
use crate::workspace::claude_project_slug;
use rusqlite::Connection;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

mod path;
mod sources;
use path::*;
use sources::index_custom_sources;

/// Each registered file commits independently; earlier receipts remain replayable on error.
pub fn index_all(
    conn: &mut Connection,
    home: &Path,
    owner_id: Option<i64>,
) -> Result<usize, String> {
    Ok(index_state_file(conn, home, owner_id)?
        + index_memory_files(conn, home, owner_id)?
        + index_custom_sources(conn, home, owner_id)?)
}

fn skip_unusable_source(err: &str) -> bool {
    // Discovery must not abort siblings. Unregistered, binary, oversize,
    // stale-policy, or transient IO on one path still leaves the rest of
    // `index_all` replayable. Explicit `index_file` / `observe_file` do
    // not use this skip list.
    matches!(
        err,
        "source_not_found"
            | "source_symlink_requires_explicit_path"
            | "source_not_authorized"
            | "source_disabled_or_policy_stale"
            | "capture_disabled"
            | "source_not_utf8"
            | "source_not_regular_file"
            | "source_path_not_utf8"
            | "capture_byte_limit"
            | "source_changed"
            | "invalid_capture_limit"
    ) || err.starts_with("source_unavailable:")
        || err.starts_with("source_changed:")
        || err.starts_with("permission_required:")
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
    let key_path = capture_key_path(
        &file,
        if follow { &opened_path } else { path },
        follow,
        confine.is_some(),
    )?;
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
    let digest = crate::handlers::sha256_hex(&bytes);
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

fn decay_table(conn: &Connection, table: &str) -> usize {
    // Blank `''` is not NULL; `julianday('')` is NULL and the age predicate
    // never matches, so those rows skipped every decay. Fall through with
    // NULLIF(TRIM(...)) the same way aging/gc does.
    conn.execute(&format!("UPDATE {table} SET score = MAX(0.05, score * POWER(MIN(1.0, 0.95 + 0.005 * MIN(retrievals, 10)), CAST((julianday('now') - julianday({LAST_TOUCHED_STAMP_SQL})) AS REAL))) WHERE status = 'active' AND score > 0.05 AND pinned = 0 AND (julianday('now') - julianday({LAST_TOUCHED_STAMP_SQL})) > 1"), []).unwrap_or(0)
}

pub fn decay_pass(conn: &Connection) -> usize {
    decay_table(conn, "memories") + decay_table(conn, "decisions")
}
