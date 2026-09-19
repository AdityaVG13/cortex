use super::scope_is_path;
use crate::clockwork::AnchorKind;
use rusqlite::{Connection, params};
use std::collections::BTreeSet;

/// Canonical observation scope. Path-like values use the same path
/// identity as CQR roots; coarse labels (`project`, `repo`) stay as given.
pub fn normalize_scope(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !scope_is_path(trimmed) {
        return trimmed.to_string();
    }
    crate::clockwork::strip_path_globs(crate::clockwork::normalize_anchor_value(
        AnchorKind::Path,
        trimmed,
    ))
}

pub(crate) fn scopes_compatible(candidate: &str, query: &str) -> bool {
    candidate == query
        || candidate.starts_with(&(query.to_string() + "/"))
        || query.starts_with(&(candidate.to_string() + "/"))
}

/// A Deposit may cite this observation scope. The unscoped `project`
/// bucket is always citable. A path-scoped observation is citable iff
/// every commit root is the same repository or a parent/child of it (a
/// union of sibling roots cannot smuggle a cite). An empty path list
/// cannot lift a path-scoped observation.
pub(crate) fn cite_scope_allowed(commit_paths: &[String], obs_scope: &str) -> bool {
    let obs = normalize_scope(obs_scope);
    if !scope_is_path(&obs) {
        return obs == "project";
    }
    let incoming: Vec<String> = commit_paths
        .iter()
        .map(|path| normalize_scope(path))
        .filter(|path| !path.is_empty())
        .collect();
    !incoming.is_empty() && incoming.iter().all(|path| scopes_compatible(&obs, path))
}

/// Scopes to pull for a caller. Empty paths keep the exact extra label
/// (default `project`). Named roots include compatible path-scoped sources
/// plus the extra/unscoped bucket (`project` when extra is omitted).
pub(crate) fn resolve_query_scopes(
    conn: &Connection,
    principal: &str,
    paths: &[String],
    extra_scope: Option<&str>,
) -> Result<Vec<String>, String> {
    let query_paths: Vec<String> = paths
        .iter()
        .map(|p| normalize_scope(p))
        .filter(|p| !p.is_empty())
        .collect();
    let extra = extra_scope.map(normalize_scope).filter(|s| !s.is_empty());
    if query_paths.is_empty() {
        return Ok(vec![extra.unwrap_or_else(|| "project".into())]);
    }
    let mut wanted: BTreeSet<String> = BTreeSet::new();
    wanted.insert(extra.unwrap_or_else(|| "project".into()));
    for path in &query_paths {
        wanted.insert(path.clone());
    }
    let mut stmt = conn
        .prepare("SELECT DISTINCT scope_label FROM observation_sources WHERE principal=?1")
        .map_err(|e| e.to_string())?;
    let stored = stmt
        .query_map(params![principal], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for label in stored {
        let normalized = normalize_scope(&label);
        if scope_is_path(&normalized)
            && query_paths
                .iter()
                .any(|query| scopes_compatible(&normalized, query))
        {
            wanted.insert(label);
        }
    }
    Ok(wanted.into_iter().collect())
}
