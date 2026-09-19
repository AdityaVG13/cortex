use super::*;
use crate::db::records;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

fn policy_epoch(conn: &Connection) -> Result<String, String> {
    conn.query_row(crate::db::records::POLICY_EPOCH_SELECT, [], |r| r.get(0))
        .map_err(|err| err.to_string())
}

pub(in crate::runtime) fn ensure(conn: &Connection) -> Result<(), String> {
    records::ensure_authoritative_schema(conn).map_err(|err| err.to_string())?;
    crate::db::capture_policy::ensure(conn).map_err(|err| err.to_string())?;
    conn.execute_batch(DDL).map_err(|err| err.to_string())
}

pub(in crate::runtime) fn granted(
    conn: &Connection,
    principal: &str,
    key: &str,
    writing: bool,
) -> Result<GrantedSource, String> {
    let row = conn.query_row("SELECT scope_id,scope_label,role,max_bytes,enabled,policy_epoch FROM observation_sources WHERE principal=?1 AND source_key=?2", params![principal,key], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,bool>(4)?,r.get::<_,String>(5)?))).optional().map_err(|err| err.to_string())?.ok_or("source_not_authorized")?;
    let policy = policy_epoch(conn)?;
    if !row.4 || row.5 != policy {
        return Err("source_disabled_or_policy_stale".into());
    }
    let state = crate::db::capture_policy::raw_state_for(conn, &row.1)?;
    if state
        .as_deref()
        .is_some_and(|s| s == "stopped" || (writing && s != "active"))
    {
        return Err("capture_disabled".into());
    }
    Ok(GrantedSource {
        scope_id: row.0,
        role: row.2,
        max_bytes: usize::try_from(row.3).map_err(|_| "invalid_capture_limit")?,
    })
}

pub(in crate::runtime) fn offset(
    conn: &Connection,
    principal: &str,
    key: &str,
    generation: &str,
) -> Result<u64, String> {
    let value: i64 = conn.query_row("SELECT byte_offset FROM observation_cursors WHERE principal=?1 AND source_key=?2 AND generation=?3", params![principal,key,generation], |r| r.get(0)).optional().map_err(|err| err.to_string())?.unwrap_or(0);
    u64::try_from(value).map_err(|_| "invalid_source_cursor".into())
}

pub(in crate::runtime) fn ensure_source(
    conn: &Connection,
    principal: &str,
    spec: &SourceSpec,
) -> Result<(), String> {
    check_label(&spec.key)?;
    let scope_label = normalize_scope(&spec.scope);
    check_label(&scope_label)?;
    if spec.max_bytes == 0 || spec.max_bytes > MAX_CAPTURE_BYTES {
        return Err("invalid_capture_limit".into());
    }
    ensure(conn)?;
    let scope = format!("observation-scope:{}", json!([principal, scope_label]));
    let existing: Option<(String, String, i64)> = conn.query_row("SELECT scope_id,role,max_bytes FROM observation_sources WHERE principal=?1 AND source_key=?2", params![principal, spec.key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(|err| err.to_string())?;
    if let Some((old_scope, role, limit)) = existing {
        if old_scope != scope || role != spec.role.as_str() || limit != spec.max_bytes as i64 {
            return Err("source_registration_conflict".into());
        }
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO scopes(scope_id,owner_id,kind,descriptor) VALUES(?1,?2,'observation',?3)", params![scope, principal, json!({"label": scope_label}).to_string()]).map_err(|err| err.to_string())?;
    let policy = policy_epoch(conn)?;
    conn.execute(
        "INSERT INTO observation_sources VALUES(?1,?2,?3,?4,?5,?6,1,?7)",
        params![
            principal,
            spec.key,
            scope,
            scope_label,
            spec.role.as_str(),
            spec.max_bytes as i64,
            policy
        ],
    )
    .map_err(|err| err.to_string())?;
    Ok(())
}
