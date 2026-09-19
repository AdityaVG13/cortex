use super::{boot_owner, owner_clause, with_boot_paths};
use crate::handlers::{agent_match_params, ident_match_sql, same_agent};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};

const STILL_VALID_EXPIRY_SQL: &str = "expires_at IS NOT NULL AND TRIM(expires_at) != '' AND julianday(expires_at) > julianday('now')";

pub(super) fn cached_rows<T>(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Vec<T> {
    let Ok(mut stmt) = conn.prepare_cached(sql) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map(params, map) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

pub fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    // The messages table has no runtime writer or pruner (auto_repair salvage
    // repopulates it whole), so an unbounded SELECT rendered one capsule line
    // per message ever salvaged on every boot. Bound the capsule to the newest
    // messages, rendered oldest-first as before. Identity matching must stay in
    // SQL: a global newest-N scan then Rust filter drops this agent's mail when
    // other recipients are more recent.
    let Some((ident, like)) = agent_match_params(agent) else {
        return Vec::new();
    };
    let scope = owner_clause(conn, "messages", boot_owner());
    cached_rows(
        conn,
        &format!(
            "SELECT sender, message FROM ( SELECT sender, message, timestamp, id FROM messages WHERE {}{scope} ORDER BY julianday(timestamp) DESC, id DESC LIMIT 10 ) ORDER BY julianday(timestamp) ASC, id ASC",
            ident_match_sql("recipient", 1)
        ),
        params![ident, like],
        |r| Ok(json!({"from":r.get::<_,String>(0)?,"message":r.get::<_,String>(1)?})),
    )
}
pub fn fetch_sessions(conn: &Connection) -> Vec<Value> {
    cached_rows(
        conn,
        &format!(
            "SELECT agent, project, description, files_json FROM sessions WHERE {STILL_VALID_EXPIRY_SQL}{} ORDER BY agent ASC, rowid ASC",
            owner_clause(conn, "sessions", boot_owner())
        ),
        [],
        |r| {
            let files_json: String = r.get(3)?;
            Ok(
                json!({"agent":r.get::<_,String>(0)?,"project":r.get::<_,Option<String>>(1)?,"description":r.get::<_,Option<String>>(2)?,"files": serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))}),
            )
        },
    )
}
pub fn fetch_locks(conn: &Connection) -> Vec<Value> {
    let mut out = cached_rows(
        conn,
        &format!(
            "SELECT path, agent, expires_at FROM locks WHERE {STILL_VALID_EXPIRY_SQL}{} ORDER BY path ASC, rowid ASC",
            owner_clause(conn, "locks", boot_owner())
        ),
        [],
        |r| {
            Ok(
                json!({"path":r.get::<_,String>(0)?,"agent":r.get::<_,String>(1)?,"expiresAt":r.get::<_,String>(2)?}),
            )
        },
    );
    with_boot_paths(|paths| {
        if paths.is_empty() {
            return;
        }
        out.retain(|lock| {
            let path = lock.get("path").and_then(|v| v.as_str()).unwrap_or("");
            crate::handlers::recall::read_path_sets(paths, &[path.to_string()])
        });
    });
    out
}
pub fn fetch_unread_feed(conn: &Connection, agent: &str) -> Vec<Value> {
    // The feed table has no retention pruner, so the previous whole-table scan
    // made every boot O(feed rows ever posted) in time and memory even though
    // the capsule renders at most the last 10 unread entries. Bound the scan
    // to the newest entries. Instant order (julianday) plus rowid matches
    // insertion among equal stamps; an ack whose feed row is gone still
    // yields no unread entries (it marked a position, not a filter).
    const FEED_CAPSULE_LINES: i64 = 10;
    // SQL error is not "never acked": swallowing it showed the newest 10 as
    // unread. Missing ack still means unread-from-start. Exact `lower(trim)`
    // missed an ack stored as `claude-code (opus)` when boot is `claude-code`.
    let Some((ident, like)) = agent_match_params(agent) else {
        return Vec::new();
    };
    let ack = match feed_ack_last_seen(conn, agent) {
        Ok(ack) => ack,
        Err(_) => return Vec::new(),
    };
    if let Some(ack_id) = &ack {
        let anchor = match conn
            .query_row(
                &format!(
                    "SELECT timestamp FROM feed WHERE id = ?1{}",
                    owner_clause(conn, "feed", boot_owner())
                ),
                params![ack_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
        {
            Ok(anchor) => anchor,
            Err(_) => return Vec::new(),
        };
        let Some(anchor_ts) = anchor else {
            return Vec::new();
        };
        // `timestamp` mixes RFC3339 (`T`) and SQLite `datetime('now')` (space).
        // Lexicographic `>` skips a later space-format row after an RFC3339 ack.
        let newest = cached_rows(
            conn,
            &format!(
                "SELECT agent, kind, summary FROM feed WHERE NOT {} AND (julianday(timestamp) > julianday(?3) OR (julianday(timestamp) = julianday(?3) AND rowid > (SELECT rowid FROM feed WHERE id = ?4))){} ORDER BY julianday(timestamp) DESC, rowid DESC LIMIT ?5",
                ident_match_sql("agent", 1),
                owner_clause(conn, "feed", boot_owner())
            ),
            params![ident, like, anchor_ts, ack_id, FEED_CAPSULE_LINES],
            feed_tuple,
        );
        return feed_json(agent, newest);
    }
    feed_json(
        agent,
        cached_rows(
            conn,
            &format!(
                "SELECT agent, kind, summary FROM feed WHERE NOT {}{} ORDER BY julianday(timestamp) DESC, rowid DESC LIMIT ?3",
                ident_match_sql("agent", 1),
                owner_clause(conn, "feed", boot_owner())
            ),
            params![ident, like, FEED_CAPSULE_LINES],
            feed_tuple,
        ),
    )
}

fn feed_tuple(row: &Row<'_>) -> rusqlite::Result<(String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn feed_json(agent: &str, mut newest: Vec<(String, String, String)>) -> Vec<Value> {
    newest.reverse();
    newest.into_iter().filter(|(entry_agent, ..)| !same_agent(entry_agent, agent)).map(|(entry_agent, kind, summary)| json!({"kind":kind,"agent":entry_agent,"summary":summary})).collect()
}
pub fn fetch_pending_tasks(conn: &Connection) -> Vec<Value> {
    cached_rows(
        conn,
        &format!(
            "SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending'{} ORDER BY julianday(created_at) ASC, task_id ASC LIMIT 5",
            owner_clause(conn, "tasks", boot_owner())
        ),
        [],
        |r| {
            let files_json: String = r.get(4)?;
            Ok(
                json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"priority":r.get::<_,String>(2)?,"project":r.get::<_,Option<String>>(3)?,"files":serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))}),
            )
        },
    )
}
fn feed_ack_last_seen(conn: &Connection, agent: &str) -> rusqlite::Result<Option<String>> {
    let Some((ident, like)) = agent_match_params(agent) else {
        return Ok(None);
    };
    conn.query_row(
        &format!(
            "SELECT last_seen_id FROM feed_acks WHERE {}{} ORDER BY rowid DESC LIMIT 1",
            ident_match_sql("agent", 1),
            owner_clause(conn, "feed_acks", boot_owner())
        ),
        params![ident, like],
        |row| row.get::<_, String>(0),
    )
    .optional()
}
pub fn fetch_claimed_tasks_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let Some((ident, like)) = agent_match_params(agent) else {
        return Vec::new();
    };
    cached_rows(
        conn,
        &format!(
            "SELECT task_id, title, priority, claimed_at FROM tasks WHERE status = 'claimed' AND {}{} ORDER BY julianday(claimed_at) ASC, task_id ASC",
            ident_match_sql("claimed_by", 1),
            owner_clause(conn, "tasks", boot_owner())
        ),
        params![ident, like],
        |r| {
            Ok(
                json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"priority":r.get::<_,String>(2)?,"claimedAt":r.get::<_,Option<String>>(3)?}),
            )
        },
    )
}
