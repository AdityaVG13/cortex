//! Address overlay over the authoritative `addresses` table
//! (`scheme, namespace, address → record_id`). Records reference logical ids
//! only, so binding, rebasing or dropping an address never rewrites a record.
//! Content addressing is optional and no digest size is assumed; a short
//! address is measured for collision and ambiguity, never trusted blindly.

use crate::db::like_prefix;
use rusqlite::{Connection, OptionalExtension, params};

fn ensure(conn: &Connection) -> rusqlite::Result<()> {
    crate::db::records::ensure_authoritative_schema(conn)
        .map_err(|e| rusqlite::Error::InvalidParameterName(e.to_string()))
}

/// Bind a locator under `(scheme, namespace)` to a record. Returns how many
/// addresses the record now carries in that namespace.
pub fn assign(
    conn: &Connection,
    record_id: &str,
    namespace: &str,
    scheme: &str,
    locator: &str,
) -> rusqlite::Result<i64> {
    ensure(conn)?;
    conn.execute("INSERT INTO addresses (scheme, namespace, address, record_id) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(scheme, namespace, address) DO UPDATE SET record_id = excluded.record_id", params![scheme, namespace, locator, record_id])?;
    conn.query_row(
        "SELECT COUNT(*) FROM addresses WHERE record_id = ?1 AND namespace = ?2",
        params![record_id, namespace],
        |r| r.get(0),
    )
}

/// Every `(scheme, locator)` bound to a record in a namespace, scheme-ordered.
pub fn resolve(
    conn: &Connection,
    record_id: &str,
    namespace: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    ensure(conn)?;
    let mut stmt = conn.prepare_cached("SELECT scheme, address FROM addresses WHERE record_id = ?1 AND namespace = ?2 ORDER BY scheme, address")?;
    let rows = stmt.query_map(params![record_id, namespace], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn record_for(
    conn: &Connection,
    scheme: &str,
    namespace: &str,
    locator: &str,
) -> rusqlite::Result<Option<String>> {
    ensure(conn)?;
    conn.query_row(
        "SELECT record_id FROM addresses WHERE scheme = ?1 AND namespace = ?2 AND address = ?3",
        params![scheme, namespace, locator],
        |r| r.get(0),
    )
    .optional()
}

/// The short-address question: how many records does this prefix name?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShortAddress {
    Unique(String),
    Ambiguous(Vec<String>),
    Unknown,
}

pub fn resolve_short(
    conn: &Connection,
    scheme: &str,
    namespace: &str,
    prefix: &str,
) -> rusqlite::Result<ShortAddress> {
    ensure(conn)?;
    if prefix.is_empty() {
        return Ok(ShortAddress::Unknown);
    }
    let mut stmt = conn.prepare_cached("SELECT DISTINCT record_id FROM addresses WHERE scheme = ?1 AND namespace = ?2 AND address LIKE ?3 ESCAPE '\\' ORDER BY record_id LIMIT 16")?;
    let ids: Vec<String> = stmt
        .query_map(params![scheme, namespace, like_prefix(prefix)], |r| {
            r.get(0)
        })?
        .collect::<Result<_, _>>()?;
    Ok(match ids.len() {
        0 => ShortAddress::Unknown,
        1 => ShortAddress::Unique(ids.into_iter().next().unwrap()),
        _ => ShortAddress::Ambiguous(ids),
    })
}

/// Smallest prefix length at which every locator under `(scheme, namespace)`
/// is unique: the collision floor of a short-address scheme on this brain.
pub fn minimum_unique_prefix(
    conn: &Connection,
    scheme: &str,
    namespace: &str,
) -> rusqlite::Result<usize> {
    ensure(conn)?;
    let mut stmt = conn.prepare(
        "SELECT address FROM addresses WHERE scheme = ?1 AND namespace = ?2 ORDER BY address",
    )?;
    let locators: Vec<String> = stmt
        .query_map(params![scheme, namespace], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let longest = locators
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    for n in 1..=longest {
        let mut seen = std::collections::BTreeSet::new();
        if locators
            .iter()
            .all(|l| seen.insert(l.chars().take(n).collect::<String>()))
        {
            return Ok(n);
        }
    }
    Ok(longest)
}

/// Re-address every record of one scheme under a new scheme, then drop the
/// old bindings. No record row is touched. Returns the number rebased.
pub fn rebase(
    conn: &Connection,
    namespace: &str,
    old_scheme: &str,
    new_scheme: &str,
    locate: impl Fn(&str, &str) -> String,
) -> rusqlite::Result<usize> {
    ensure(conn)?;
    // The trailing DELETE is by old_scheme. Rebasing onto the same scheme
    // would assign new locators then delete those same rows.
    if old_scheme == new_scheme {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    let mut stmt = tx.prepare("SELECT record_id, address FROM addresses WHERE scheme = ?1 AND namespace = ?2 ORDER BY record_id, address")?;
    let rows: Vec<(String, String)> = stmt
        .query_map(params![old_scheme, namespace], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<Result<_, _>>()?;
    drop(stmt);
    for (id, old) in &rows {
        assign(&tx, id, namespace, new_scheme, &locate(id, old))?;
    }
    tx.execute(
        "DELETE FROM addresses WHERE scheme = ?1 AND namespace = ?2",
        params![old_scheme, namespace],
    )?;
    tx.commit()?;
    Ok(rows.len())
}
