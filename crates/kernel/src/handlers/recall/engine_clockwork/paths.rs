use super::{ScoredCandidate, normalize_task_path};
use crate::clockwork::QueryFrame;
use rusqlite::{Connection, params};
use std::collections::HashMap;

pub(super) fn path_compatible(candidate: &str, query: &str) -> bool {
    candidate == query
        || candidate.starts_with(&(query.to_string() + "/"))
        || query.starts_with(&(candidate.to_string() + "/"))
}

pub(crate) fn normalize_query_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| normalize_task_path(path))
        .filter(|path| !path.is_empty())
        .collect()
}

/// Read law: an unscoped row stays visible under a named project.
pub(crate) fn read_path_sets(query_paths: &[String], candidate_paths: &[String]) -> bool {
    if query_paths.is_empty() || candidate_paths.is_empty() {
        return true;
    }
    candidate_paths.iter().any(|candidate| {
        query_paths
            .iter()
            .any(|query| path_compatible(candidate, query))
    })
}

/// Write identity: unscoped and path-scoped facts never Jaccard-merge.
pub(crate) fn jaccard_path_sets(incoming_paths: &[String], candidate_paths: &[String]) -> bool {
    match (incoming_paths.is_empty(), candidate_paths.is_empty()) {
        (true, true) => true,
        (false, false) => candidate_paths.iter().any(|candidate| {
            incoming_paths
                .iter()
                .any(|incoming| path_compatible(candidate, incoming))
        }),
        _ => false,
    }
}

pub(crate) fn explicit_paths_by_target(
    conn: &Connection,
    target_type: &str,
    ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, String> {
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let list = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT e.target_id, a.value FROM clock_anchors a JOIN clock_anchor_evidence e ON e.anchor_id = a.id WHERE e.target_type = ?1 AND e.target_id IN ({list}) AND a.kind = 'path' AND a.specificity >= 3"
    );
    let mut stmt = conn.prepare(&sql).map_err(|err| err.to_string())?;
    let rows = stmt
        .query_map(params![target_type], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|err| err.to_string())?;
    for (target_id, value) in rows.flatten() {
        out.entry(target_id).or_default().push(value);
    }
    Ok(out)
}

pub(crate) fn target_scope_compatible(
    conn: &Connection,
    target_type: &str,
    target_id: i64,
    query_paths: &[String],
) -> Result<bool, String> {
    let query_paths = normalize_query_paths(query_paths);
    if query_paths.is_empty() {
        return Ok(true);
    }
    let candidate_paths = explicit_path_values(conn, target_type, target_id)?;
    Ok(read_path_sets(&query_paths, &candidate_paths))
}

pub(super) fn path_context_compatible(
    conn: &Connection,
    candidate: &ScoredCandidate,
    frame: &QueryFrame,
) -> Result<bool, String> {
    target_scope_compatible(
        conn,
        &candidate.target_type,
        candidate.target_id,
        &frame.paths,
    )
}

pub(super) fn explicit_path_values(
    conn: &Connection,
    target_type: &str,
    target_id: i64,
) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare_cached("SELECT a.value FROM clock_anchors a JOIN clock_anchor_evidence e ON e.anchor_id = a.id WHERE e.target_type = ?1 AND e.target_id = ?2 AND a.kind = 'path' AND a.specificity >= 3").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![target_type, target_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}
