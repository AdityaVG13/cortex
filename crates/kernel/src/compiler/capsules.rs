use crate::handlers::{estimate_tokens, estimate_tokens_from_chars};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::Path;
pub fn stored_max_timestamp(conn: &Connection) -> Option<String> {
    let mem_max: Option<String> = conn.query_row("SELECT MAX(updated_at) FROM memories WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))", [], |r| r.get(0)).ok().flatten();
    let dec_max: Option<String> = conn.query_row("SELECT MAX(updated_at) FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned'))", [], |r| r.get(0)).ok().flatten();
    match (mem_max, dec_max) {
        (Some(a), Some(b)) => Some(if a > b { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

pub fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
    if let Some(stored) = stored_max_timestamp(conn) {
        return Some(stored);
    }
    conn.query_row("SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ?1 ORDER BY created_at DESC, id DESC LIMIT 1", params![agent], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|data| serde_json::from_str::<Value>(&data).ok()?.get("timestamp")?.as_str().map(|s| s.to_string()))
}
/// `AND owner_id = N` when the table carries owner scoping and a caller is
/// known; empty otherwise. Team-mode boot must not leak another owner's
/// messages, tasks, locks, feed or decisions into a caller's capsule.
pub fn owner_clause(conn: &Connection, table: &str, owner: Option<i64>) -> String {
    // Solo mode (the common case) never touches PRAGMA table_info.
    match owner {
        Some(id) if crate::db::table_has_column(conn, table, "owner_id") => {
            format!(" AND owner_id = {id}")
        }
        _ => String::new(),
    }
}
thread_local! {
    static BOOT_OWNER: std::cell::Cell<Option<i64>> = const { std::cell::Cell::new(None) };
    static BOOT_PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}
pub fn set_boot_owner(owner: Option<i64>) {
    BOOT_OWNER.with(|cell| cell.set(owner));
}
pub fn boot_owner() -> Option<i64> {
    BOOT_OWNER.with(|cell| cell.get())
}
pub fn set_boot_paths(paths: &[String]) {
    let normalized = crate::handlers::recall::normalize_query_paths(paths);
    BOOT_PATHS.with(|cell| *cell.borrow_mut() = normalized);
}
pub fn with_boot_paths<R>(f: impl FnOnce(&[String]) -> R) -> R {
    BOOT_PATHS.with(|cell| f(&cell.borrow()))
}
/// `None` means the boot is unscoped and every id stays eligible.
pub(crate) fn boot_scope_allowlist(
    conn: &Connection,
    target_type: &str,
    ids: &[i64],
) -> Option<HashSet<i64>> {
    with_boot_paths(|paths| {
        if paths.is_empty() {
            return None;
        }
        let path_map = crate::handlers::recall::explicit_paths_by_target(conn, target_type, ids)
            .unwrap_or_default();
        Some(
            ids.iter()
                .copied()
                .filter(|id| {
                    crate::handlers::recall::read_path_sets(
                        paths,
                        path_map.get(id).map(Vec::as_slice).unwrap_or(&[]),
                    )
                })
                .collect(),
        )
    })
}
pub(crate) fn keep_boot_id(allow: &Option<HashSet<i64>>, id: i64) -> bool {
    allow.as_ref().map(|ids| ids.contains(&id)).unwrap_or(true)
}
pub fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    // The messages table has no runtime writer or pruner (auto_repair salvage
    // repopulates it whole), so an unbounded SELECT rendered one capsule line
    // per message ever salvaged on every boot. Bound the capsule to the newest
    // messages, rendered oldest-first as before.
    let scope = owner_clause(conn, "messages", boot_owner());
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT sender, message FROM ( \
             SELECT sender, message, timestamp, id FROM messages \
             WHERE recipient = ?1{scope} \
             ORDER BY timestamp DESC, id DESC LIMIT 10 \
         ) ORDER BY timestamp ASC, id ASC"
    )) {
        if let Ok(rows) = stmt.query_map(params![agent], |r| {
            Ok(json!({"from":r.get::<_,String>(0)?,"message":r.get::<_,String>(1)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn fetch_sessions(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT agent, project, description, files_json FROM sessions WHERE expires_at > ?1{} ORDER BY agent ASC, rowid ASC",
        owner_clause(conn, "sessions", boot_owner())
    )) {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            let files_json: String = r.get(3)?;
            Ok(json!({
"agent":r.get::<_,String>(0)?,"project":r.get::<_,Option<String>>(1)?,"description":r.get::<_,Option<String>>(2)?,"files":
serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn fetch_locks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT path, agent, expires_at FROM locks WHERE expires_at > ?1{} ORDER BY path ASC, rowid ASC",
        owner_clause(conn, "locks", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map(params![now], |r| {
            Ok(json!({"path":r.get::<_,String>(0)?,"agent":r.get::<_,String>(1)?,"expiresAt":r.get::<_,String
>(2)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn fetch_unread_feed(conn: &Connection, agent: &str) -> Vec<Value> {
    // The feed table has no retention pruner, so the previous whole-table scan
    // made every boot O(feed rows ever posted) in time and memory even though
    // the capsule renders at most the last 10 unread entries. Bound the scan
    // to the newest entries; (timestamp, id) tuple ordering matches the
    // previous positional scan, and an ack row that no longer exists still
    // yields no unread entries (it marked a position, not a filter).
    const FEED_CAPSULE_LINES: i64 = 10;
    let ack: Option<String> = conn
        .query_row(
            "SELECT last_seen_id FROM feed_acks WHERE agent = ?1",
            params![agent],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(ack_id) = &ack {
        let anchor: Option<String> = conn
            .query_row(
                "SELECT timestamp FROM feed WHERE id = ?1",
                params![ack_id],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let Some(anchor_ts) = anchor else {
            return Vec::new();
        };
        if let Ok(mut stmt) = conn.prepare_cached(&format!(
            "SELECT agent, kind, summary FROM feed \
             WHERE agent != ?1 AND (timestamp > ?2 OR (timestamp = ?2 AND rowid > (SELECT rowid FROM feed WHERE id = ?3))){} \
             ORDER BY timestamp DESC, rowid DESC LIMIT ?4",
            owner_clause(conn, "feed", boot_owner())
        )) {
            if let Ok(rows) = stmt.query_map(params![agent, anchor_ts, ack_id, FEED_CAPSULE_LINES], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            }) {
                let mut newest: Vec<(String, String, String)> = rows.flatten().collect();
                newest.reverse();
                return newest
                    .into_iter()
                    .map(|(entry_agent, kind, summary)| json!({"kind":kind,"agent":entry_agent,"summary":summary}))
                    .collect();
            }
        }
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT agent, kind, summary FROM feed WHERE agent != ?1{} ORDER BY timestamp DESC, rowid DESC LIMIT ?2",
        owner_clause(conn, "feed", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map(params![agent, FEED_CAPSULE_LINES], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))) {
            let mut newest: Vec<(String, String, String)> = rows.flatten().collect();
            newest.reverse();
            for (entry_agent, kind, summary) in newest {
                out.push(json!({"kind":kind,"agent":entry_agent,"summary":summary}));
            }
        }
    }
    out
}
pub fn fetch_pending_tasks(conn: &Connection) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending'{} ORDER BY created_at ASC, task_id ASC LIMIT 5",
        owner_clause(conn, "tasks", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map([], |r| {
            let files_json: String = r.get(4)?;
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,
"priority":r.get::<_,String>(2)?,"project":r.get::<_,Option<String>>(3)?,"files":serde_json::from_str::<Value>(&files_json).
unwrap_or(json!([]))}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn fetch_claimed_tasks_for_agent(conn: &Connection, agent: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare_cached(&format!(
        "SELECT task_id, title, priority, claimed_at FROM tasks WHERE status = 'claimed' AND claimed_by = ?1{} ORDER BY claimed_at ASC, task_id ASC",
        owner_clause(conn, "tasks", boot_owner())
    )) {
        if let Ok(rows) = stmt.query_map(params![agent], |r| {
            Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"priority":r.get
::<_,String>(2)?,"claimedAt":r.get::<_,Option<String>>(3)?}))
        }) {
            for row in rows.flatten() {
                out.push(row);
            }
        }
    }
    out
}
pub fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
    let last_boot = get_last_boot_time(conn, agent);
    let mut parts: Vec<String> = Vec::new();
    let messages = fetch_messages_for_agent(conn, agent);
    if !messages.is_empty() {
        let lines: Vec<String> = messages
            .iter()
            .map(|m| {
                let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                let msg = m.get("message").and_then(|v| v.as_str()).unwrap_or("");
                let truncated: String = msg.chars().take(200).collect();
                format!("- From {from}: \"{truncated}\"")
            })
            .collect();
        parts.push(format!("## Pending Messages\n{}", lines.join("\n")));
    }
    let sessions = fetch_sessions(conn);
    let other_sessions: Vec<&Value> = sessions
        .iter()
        .filter(|s| s.get("agent").and_then(|v| v.as_str()) != Some(agent))
        .collect();
    if !other_sessions.is_empty() {
        let lines: Vec<String> = other_sessions
            .iter()
            .map(|s| {
                let ag = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                let proj = s
                    .get("project")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let desc = s
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no description");
                format!("- {ag} working on {proj}: \"{desc}\"")
            })
            .collect();
        parts.push(format!("## Active Agents\n{}", lines.join("\n")));
    }
    let locks = fetch_locks(conn);
    if !locks.is_empty() {
        let lines: Vec<String> = locks
            .iter()
            .map(|l| {
                let path = l.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                let ag = l.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- {path} locked by {ag}")
            })
            .collect();
        parts.push(format!("## Active Locks\n{}", lines.join("\n")));
    }
    let mut feed = fetch_unread_feed(conn, agent);
    if feed.len() > 10 {
        feed = feed.split_off(feed.len() - 10);
    }
    if !feed.is_empty() {
        let lines: Vec<String> = feed
            .iter()
            .map(|e| {
                let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let ag = e.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
                let summary = e.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                format!("- [{kind}] {ag}: {summary}")
            })
            .collect();
        parts.push(format!("## Feed\n{}", lines.join("\n")));
    }
    let pending_tasks = fetch_pending_tasks(conn);
    if !pending_tasks.is_empty() {
        let lines: Vec<String> = pending_tasks
            .iter()
            .map(|t| {
                let pri = t.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- [{pri}] {title}")
            })
            .collect();
        parts.push(format!("## Pending Tasks\n{}", lines.join("\n")));
    }
    let my_tasks = fetch_claimed_tasks_for_agent(conn, agent);
    if !my_tasks.is_empty() {
        let lines: Vec<String> = my_tasks
            .iter()
            .map(|t| {
                let pri = t.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                format!("- [{pri}] {title}")
            })
            .collect();
        parts.push(format!("## Your Active Tasks\n{}", lines.join("\n")));
    }
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT d.id, d.decision, d.disputes_id, d.confirmed_by,
                COALESCE(d.valid_from, d.observed_at, d.created_at), d.valid_until
         FROM decisions d
         WHERE d.status = 'disputed'
           AND (d.valid_from IS NULL OR julianday(d.valid_from) <= julianday('now'))
           AND (d.valid_until IS NULL OR julianday(d.valid_until) > julianday('now'))
           AND EXISTS (SELECT 1 FROM decision_conflicts c WHERE c.source_decision_id = d.id AND c.status = 'open')
         ORDER BY d.created_at DESC, d.id DESC LIMIT 6",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        }) {
            let mut seen = HashSet::new();
            let mut lines: Vec<String> = Vec::new();
            for (id, decision, disputes_id, confirmed_by, valid_from, valid_until) in rows.flatten() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(disputed_id) = disputes_id {
                    seen.insert(disputed_id);
                }
                let mut line = crate::compiler::format_fact_line(
                    "decision",
                    id,
                    &decision,
                    "disputed",
                    confirmed_by.as_deref(),
                    valid_from.as_deref(),
                    valid_until.as_deref(),
                );
                if let Some(disputed_id) = disputes_id {
                    if let Ok((partner_decision, partner_status, partner_confirmed_by, partner_valid_from, partner_valid_until)) = conn.query_row(
                        "SELECT decision, status, confirmed_by, COALESCE(valid_from, observed_at, created_at), valid_until FROM decisions WHERE id = ?1",
                        params![disputed_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, Option<String>>(4)?,
                            ))
                        },
                    ) {
                        line.push_str(" vs ");
                        line.push_str(&crate::compiler::format_fact_line(
                            "decision",
                            disputed_id,
                            &partner_decision,
                            &partner_status,
                            partner_confirmed_by.as_deref(),
                            partner_valid_from.as_deref(),
                            partner_valid_until.as_deref(),
                        ));
                    }
                }
                lines.push(line);
            }
            if !lines.is_empty() {
                parts.push(format!("CONFLICTS:\n{}", lines.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n")));
            }
        }
    }
    if let Some(focus) = crate::focus::focus_current(conn, agent) {
        let label = focus.get("label").and_then(|v| v.as_str()).unwrap_or("?");
        let entries = focus.get("entries").and_then(|v| v.as_u64()).unwrap_or(0);
        parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));
    }
    if let Some(ref lb) = last_boot {
        if let Ok(mut stmt) =
            conn.prepare_cached(&format!("SELECT id, decision, context, source_agent FROM decisions WHERE status = 'active'{} AND created_at >= ?1 AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY created_at DESC, rowid DESC LIMIT 20", owner_clause(conn, "decisions", boot_owner())))
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?))) {
                let collected: Vec<(i64, String, Option<String>, String)> = rows.flatten().collect();
                let ids: Vec<i64> = collected.iter().map(|row| row.0).collect();
                let allow = boot_scope_allowlist(conn, "decision", &ids);
                let lines: Vec<String> = collected
                    .into_iter()
                    .filter(|(id, ..)| keep_boot_id(&allow, *id))
                    .take(5)
                    .map(|(_, dec, ctx, ag)| {
                        let c = ctx.map(|c| format!(" ({c})")).unwrap_or_default();
                        format!("- [{ag}] {dec}{c}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New decisions:\n{}", lines.join("\n")));
                }
            }
        }
        if let Ok(mut stmt) =
            conn.prepare_cached(&format!("SELECT text, type FROM memories WHERE status = 'active'{} AND updated_at >= ?1 AND type != 'state' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY updated_at DESC, id DESC LIMIT 3", owner_clause(conn, "memories", boot_owner())))
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                let lines: Vec<String> = rows
                    .flatten()
                    .map(|(text, mtype)| {
                        let truncated: String = text.chars().take(100).collect();
                        format!("- [{mtype}] {truncated}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("New knowledge:\n{}", lines.join("\n")));
                }
            }
        }
        if let Ok(mut stmt) = conn
            .prepare("SELECT type, COUNT(*) as cnt FROM events WHERE created_at > ?1 AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type ORDER BY type ASC")
        {
            if let Ok(rows) = stmt.query_map(params![lb], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                let entries: Vec<String> = rows.flatten().map(|(etype, cnt)| format!("{cnt} {}", etype.replace('_', " "))).collect();
                if !entries.is_empty() {
                    parts.push(format!("Activity since last boot: {}", entries.join(", ")));
                }
            }
        }
    }
    let has_new_section = parts.iter().any(|p| {
        p.starts_with("New decisions:")
            || p.starts_with("New knowledge:")
            || p.starts_with("Activity since last boot:")
    });
    if !has_new_section {
        let already_has_recent = parts.iter().any(|p| p.starts_with("Recent decisions:"));
        if !already_has_recent {
            if let Ok(mut stmt) = conn.prepare_cached("SELECT id, decision, context FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now')) AND (version_id IS NULL OR version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) ORDER BY created_at DESC, id DESC LIMIT 20") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))) {
                let collected: Vec<(i64, String, Option<String>)> = rows.flatten().collect();
                let ids: Vec<i64> = collected.iter().map(|row| row.0).collect();
                let allow = boot_scope_allowlist(conn, "decision", &ids);
                let lines: Vec<String> = collected
                    .into_iter()
                    .filter(|(id, ..)| keep_boot_id(&allow, *id))
                    .take(5)
                    .map(|(_, dec, ctx)| {
                        let c = ctx.map(|c| format!(" — {c}")).unwrap_or_default();
                        format!("- {dec}{c}")
                    })
                    .collect();
                if !lines.is_empty() {
                    parts.push(format!("Recent decisions:\n{}", lines.join("\n")));
                }
            }
        }
        }
    }
    let text = parts.join("\n\n");
    let tokens = estimate_tokens(&text);
    let freshness = last_boot
        .as_ref()
        .map(|lb| {
            let prefix: String = lb.chars().take(16).collect();
            format!("since {prefix}")
        })
        .unwrap_or_else(|| "first boot".to_string());
    (text, tokens, freshness)
}
pub fn estimate_raw_baseline(conn: &Connection, _home: &Path) -> usize {
    let mut total_chars: usize = 0;
    let mem_chars: i64 = conn
        .query_row("SELECT COALESCE(SUM(LENGTH(text)), 0) FROM memories WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))", [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += mem_chars as usize;
    let dec_chars: i64 = conn
        .query_row("SELECT COALESCE(SUM(LENGTH(decision)), 0) FROM decisions WHERE status = 'active' AND (expires_at IS NULL OR julianday(expires_at) > julianday('now')) AND (valid_from IS NULL OR julianday(valid_from) <= julianday('now')) AND (valid_until IS NULL OR julianday(valid_until) > julianday('now'))", [], |r| r.get(0))
        .unwrap_or(0);
    total_chars += dec_chars as usize;
    estimate_tokens_from_chars(total_chars)
}
pub fn record_boot(conn: &Connection, agent: &str) {
    let now = stored_max_timestamp(conn)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let _ = conn.execute(
        "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
        params![
            "agent_boot",
            serde_json::to_string(&json!({"timestamp"
:&now,"agent":agent}))
            .unwrap_or_default(),
            agent
        ],
    );
}
