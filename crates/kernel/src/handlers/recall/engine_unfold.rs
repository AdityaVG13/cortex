use super::{RecallContext, is_missing_team_visibility_columns, is_visible};
use crate::db::{ACTIVE_TEMPORAL_SQL, active_temporal_sql};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::collections::HashSet;

pub type CrystalMemberSourceRow = (Option<String>, Option<i64>, Option<String>);
pub fn crystal_source(crystal_id: i64, label: &str) -> String {
    format!("crystal::{crystal_id}::{label}")
}
pub fn parse_crystal_source_id(source: &str) -> Option<i64> {
    let rest = source.strip_prefix("crystal::")?;
    let (id, _) = rest.split_once("::")?;
    id.parse::<i64>().ok()
}
fn crystal_member_joins() -> String {
    format!(
        " FROM cluster_members cm LEFT JOIN memories m ON cm.target_type = 'memory' AND cm.target_id = m.id AND {mem} LEFT JOIN decisions d ON cm.target_type = 'decision' AND cm.target_id = d.id AND {dec} WHERE cm.cluster_id = ?1 ORDER BY cm.target_type, cm.target_id",
        mem = active_temporal_sql("m"),
        dec = active_temporal_sql("d")
    )
}
const CRYSTAL_MEMBER_SOURCE: &str = "SELECT CASE WHEN cm.target_type = 'memory' THEN COALESCE(m.source, 'memory::' || m.id) ELSE COALESCE(d.context, 'decision::' || d.id) END AS source";
const CRYSTAL_MEMBER_VISIBILITY: &str = ", CASE WHEN cm.target_type = 'memory' THEN m.owner_id ELSE d.owner_id END AS owner_id, CASE WHEN cm.target_type = 'memory' THEN m.visibility ELSE d.visibility END AS visibility";
pub fn crystal_member_sources(
    conn: &Connection,
    crystal_id: i64,
    ctx: &RecallContext,
) -> Vec<String> {
    let query_rows = |sql: &str,
                      with_visibility: bool|
     -> Result<Vec<CrystalMemberSourceRow>, rusqlite::Error> {
        let mut stmt = conn.prepare_cached(sql)?;
        let mapped = stmt.query_map(params![crystal_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                if with_visibility {
                    row.get::<_, Option<i64>>(1)?
                } else {
                    None
                },
                if with_visibility {
                    row.get::<_, Option<String>>(2)?
                } else {
                    None
                },
            ))
        })?;
        Ok(mapped.flatten().collect())
    };
    let joins = crystal_member_joins();
    let sql_with_visibility = format!("{CRYSTAL_MEMBER_SOURCE}{CRYSTAL_MEMBER_VISIBILITY}{joins}");
    let sql_legacy = format!("{CRYSTAL_MEMBER_SOURCE}{joins}");
    let rows = query_rows(&sql_with_visibility, true)
        .or_else(|err| {
            if is_missing_team_visibility_columns(&err) {
                query_rows(&sql_legacy, false)
            } else {
                Err(err)
            }
        })
        .unwrap_or_default();
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    for (source, owner_id, visibility) in rows {
        let Some(source) = source else {
            continue;
        };
        if !is_visible(owner_id, visibility.as_deref(), ctx) {
            continue;
        }
        if seen.insert(source.clone()) {
            sources.push(source);
        }
    }
    sources
}
pub type CrystalUnfoldRow = (String, String, i64, Option<i64>, Option<String>);
pub fn query_crystal_for_unfold(conn: &Connection, crystal_id: i64) -> Option<CrystalUnfoldRow> {
    query_acl_row(
        conn,
        "SELECT label, consolidated_text, member_count, owner_id, visibility FROM memory_clusters WHERE id = ?1",
        "SELECT label, consolidated_text, member_count FROM memory_clusters WHERE id = ?1",
        &[&crystal_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, None, None)),
    )
}

fn unfold_crystal(conn: &Connection, crystal_id: i64, ctx: &RecallContext) -> Option<Value> {
    let (label, mut text, member_count, owner_id, visibility) =
        query_crystal_for_unfold(conn, crystal_id)?;
    if !is_visible(owner_id, visibility.as_deref(), ctx) {
        return None;
    }
    let members = crystal_member_sources(conn, crystal_id, ctx);
    if !members.is_empty() {
        text.push_str("\n\nFamily members:\n");
        for member in members.iter().take(16) {
            text.push_str("- ");
            text.push_str(member);
            text.push('\n');
        }
        if member_count as usize > members.len() {
            text.push_str(&format!(
                "... plus {} more hidden or archived member(s)",
                (member_count as usize).saturating_sub(members.len())
            ));
        }
    }
    Some(
        json!({"source":crystal_source(crystal_id,&label),"text":text.trim_end(),"type":"crystal","label":label,"clusterId":crystal_id,"members":members,"memberCount":member_count}),
    )
}

fn unfold_decision_by_id(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    let id = source.strip_prefix("decision::")?.parse::<i64>().ok()?;
    let (decision, context, owner_id, visibility) = query_decision_by_id_for_unfold(conn, id)?;
    if !is_visible(owner_id, visibility.as_deref(), ctx) {
        return None;
    }
    // A cold route marker is never served as the exact source bytes.
    if crate::db::cold::is_cold_marker(&decision) {
        return Some(cold_hydrate_value(conn, "decision", id, "decision", true));
    }
    Some(
        json!({"text":with_context(decision, context),"type":"decision","physical_state":"inline"}),
    )
}

pub fn unfold_source(conn: &Connection, source: &str, ctx: &RecallContext) -> Option<Value> {
    if let Some(value) =
        parse_crystal_source_id(source).and_then(|id| unfold_crystal(conn, id, ctx))
    {
        return Some(value);
    }
    // A retained memory identity takes precedence over named sources, even
    // when its ACL denies access. Do not fall through after an invisible id.
    if let Some(row) =
        parse_memory_source_id(source).and_then(|id| query_memory_by_id_for_unfold(conn, id))
    {
        return memory_if_visible(conn, ctx, row);
    }
    query_memory_for_unfold(conn, source)
        .and_then(|row| memory_if_visible(conn, ctx, row))
        .or_else(|| unfold_decision_by_id(conn, source, ctx))
        .or_else(|| {
            let (decision, context, owner_id, visibility) =
                query_decision_by_context_for_unfold(conn, source)?;
            is_visible(owner_id, visibility.as_deref(), ctx)
                .then(|| json!({"text":with_context(decision, context),"type":"decision"}))
        })
        .or_else(|| {
            let stripped = source.strip_prefix("memory::")?;
            query_memory_for_unfold(conn, stripped)
                .and_then(|row| memory_if_visible(conn, ctx, row))
        })
}
fn parse_memory_source_id(source: &str) -> Option<i64> {
    source.strip_prefix("memory::").and_then(|s| s.parse().ok())
}
fn with_context(text: String, context: Option<String>) -> String {
    context
        .map(|c| format!("{text}\n\nContext: {c}"))
        .unwrap_or(text)
}
fn cold_hydrate_value(
    conn: &Connection,
    kind: &str,
    id: i64,
    ty: &str,
    join_context: bool,
) -> Value {
    match crate::db::cold::hydrate(conn, kind, id) {
        Ok(Some((text, context, intact))) => {
            let text = if join_context {
                with_context(text, context)
            } else {
                text
            };
            json!({"text":text,"type":ty,"physical_state":"cold_segment","intact":intact})
        }
        _ => {
            json!({"text":Value::Null,"type":ty,"physical_state":"unavailable","error":"cold block unreadable"})
        }
    }
}
fn memory_if_visible(
    conn: &Connection,
    ctx: &RecallContext,
    row: MemoryUnfoldRow,
) -> Option<Value> {
    let (id, text, ty, owner_id, visibility) = row;
    is_visible(owner_id, visibility.as_deref(), ctx)
        .then(|| memory_unfold_value(conn, id, text, ty))
}
fn memory_unfold_value(conn: &Connection, id: i64, text: String, ty: String) -> Value {
    if crate::db::cold::is_cold_marker(&text) {
        return cold_hydrate_value(conn, "memory", id, &ty, false);
    }
    json!({"text":text,"type":ty})
}
// RFC3339 (`T`) and SQLite `datetime('now')` (space) are not lexicographic.
// Empty strings mean unbounded (same as NULL); julianday('') is NULL and
// would otherwise exclude the row.
const UNFOLD_ACTIVE: &str = ACTIVE_TEMPORAL_SQL;
pub type MemoryUnfoldRow = (i64, String, String, Option<i64>, Option<String>);
pub type DecisionUnfoldRow = (String, Option<String>, Option<i64>, Option<String>);
fn query_acl_row<T, F, G>(
    conn: &Connection,
    with_sql: &str,
    without_sql: &str,
    bind: &[&dyn rusqlite::types::ToSql],
    map_with: F,
    map_without: G,
) -> Option<T>
where
    F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    G: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    match conn.query_row(with_sql, bind, map_with) {
        Ok(row) => Some(row),
        Err(err) if is_missing_team_visibility_columns(&err) => {
            conn.query_row(without_sql, bind, map_without).ok()
        }
        Err(_) => None,
    }
}
fn query_memory_pred(
    conn: &Connection,
    pred: &str,
    bind: &[&dyn rusqlite::types::ToSql],
    extra: &str,
) -> Option<MemoryUnfoldRow> {
    query_acl_row(
        conn,
        &format!("SELECT id, text, type, owner_id, visibility FROM memories WHERE {pred}{extra}"),
        &format!("SELECT id, text, type FROM memories WHERE {pred}{extra}"),
        bind,
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, None, None)),
    )
}
pub fn query_memory_for_unfold(conn: &Connection, source: &str) -> Option<MemoryUnfoldRow> {
    let extra = format!(" AND {UNFOLD_ACTIVE} ORDER BY score DESC LIMIT 1");
    query_memory_pred(conn, "source = ?1", &[&source], &extra)
}
/// Exact expansion by logical id ignores the active-status gate: an
/// archived or superseded row is still a retained source (archive is not
/// erasure). Sibling of `query_decision_by_id_for_unfold`.
pub fn query_memory_by_id_for_unfold(conn: &Connection, id: i64) -> Option<MemoryUnfoldRow> {
    query_memory_pred(conn, "id = ?1", &[&id], "")
}
fn query_decision_for_unfold(
    conn: &Connection,
    predicate: &str,
    bind: &[&dyn rusqlite::types::ToSql],
    extra: &str,
) -> Option<DecisionUnfoldRow> {
    query_acl_row(
        conn,
        &format!(
            "SELECT decision, context, owner_id, visibility FROM decisions WHERE {predicate}{extra}"
        ),
        &format!("SELECT decision, context FROM decisions WHERE {predicate}{extra}"),
        bind,
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        |row| Ok((row.get(0)?, row.get(1)?, None, None)),
    )
}
/// Exact expansion by logical id ignores the active-status gate: an
/// archived or superseded row is still a retained source (archive is not
/// erasure); callers see its status in the row itself.
pub fn query_decision_by_id_for_unfold(conn: &Connection, id: i64) -> Option<DecisionUnfoldRow> {
    query_decision_for_unfold(conn, "id = ?1", &[&id], "")
}
pub fn query_decision_by_context_for_unfold(
    conn: &Connection,
    source: &str,
) -> Option<DecisionUnfoldRow> {
    let extra = format!(" AND {UNFOLD_ACTIVE} ORDER BY score DESC LIMIT 1");
    query_decision_for_unfold(conn, "context = ?1", &[&source], &extra)
}
